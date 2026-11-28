// A crawl's outcome log, current failed requests and `retryFailed` on an
// Attached engine (spec #611's #1495; ADRs 0102 and 0117).
import {
  BlockedError,
  Crawl,
  Foxhound,
  seed,
  type CrawlRequestOutcome,
  type CrawlSettings,
} from "@goodfetch/foxhound";
import { provingGroundUrl, test } from "@goodfetch/test-support";
import { describe, expect } from "vitest";

function siteSettings(provingGround: {
  readonly resolve: Readonly<Record<string, string>>;
}): CrawlSettings {
  const resolve: Record<string, readonly string[]> = {};
  for (const [host, address] of Object.entries(provingGround.resolve)) {
    resolve[host] = [address];
  }
  return {
    fetch: { tier: "http" },
    network: { resolve },
    egress: { privateRanges: Object.keys(resolve) },
  };
}

describe("Crawl outcomes, failedRequests and retryFailed (Attached engine)", () => {
  test.override("product", "foxhound");
  test.override("provingGroundSeed", 1495);
  test.override("withBrowser", false);

  test(
    "streams filtered outcomes in commit order, never stalls on an unread stream, and retries failed/blocked requests superseding by request.id",
    {
      tags: [
        "attached",
        "story:6-238",
        "story:6-306",
        "story:12-219",
        "operation:run.outcomes",
        "operation:run.failed_requests",
        "operation:run.retry_failed",
      ],
    },
    async ({ env, provingGround }) => {
      const okUrl = provingGroundUrl(provingGround, "sitemap-seed.test", "/books/1");
      const skipUrl = provingGroundUrl(provingGround, "sitemap-seed.test", "/private/books/1");
      const failUrl = provingGroundUrl(provingGround, "sitemap-seed.test", "/books/2");
      const blockUrl = provingGroundUrl(provingGround, "sitemap-seed.test", "/books/3");

      let failHits = 0;
      let blockHits = 0;
      let retryEntered!: () => void;
      const retryAttemptEntered = new Promise<void>((resolve) => {
        retryEntered = resolve;
      });
      let releaseRetry!: () => void;
      const allowRetryCommit = new Promise<void>((resolve) => {
        releaseRetry = resolve;
      });

      const crawl = new Crawl();
      const notes = crawl.dataset("notes");
      const okLabel = crawl.label("ok");
      const failLabel = crawl.label("fail");
      const extraLabel = crawl.label("extra");

      crawl.handle(okLabel, async (ctx) => {
        await ctx.push(notes, { url: okUrl });
      });

      crawl.handle(failLabel, async (ctx) => {
        if (failHits++ === 0) {
          throw new Error("transient boom");
        }
        retryEntered();
        await allowRetryCommit;
        await ctx.push(notes, { url: failUrl });
      });

      crawl.handle(extraLabel, async (ctx) => {
        if (blockHits++ === 0) {
          throw new BlockedError("blocked by captcha", { details: { cause: "captcha" } });
        }
        await ctx.push(notes, { url: blockUrl });
      });

      const retry = { max: 0 };
      const crawlSettings: CrawlSettings = {
        ...siteSettings(provingGround),
        crawl: { handler: { concurrency: 1 }, retry },
      };

      // oxlint-disable-next-line no-restricted-properties -- Foxhound.connect is the SDK entry point.
      const foxhound = await Foxhound.connect({ env });
      try {
        const run = await foxhound.startCrawl(crawl, {
          runId: "ts-outcomes-retry",
          seeds: [
            seed(okUrl, okLabel),
            seed(skipUrl, extraLabel),
            seed(failUrl, failLabel),
            seed(blockUrl, extraLabel),
          ],
          settings: crawlSettings,
        });

        // A stalled outcomes reader left unread never slows or blocks the crawl.
        const stalled = run.outcomes();
        const report = await run.finish();
        expect(report.lifecycle).toBe("finished");
        stalled.close();

        const all: CrawlRequestOutcome[] = [];
        for await (const outcome of run.outcomes()) {
          all.push(outcome);
        }
        expect(all).toHaveLength(4);
        for (let i = 1; i < all.length; i += 1) {
          const prev = all[i - 1];
          const curr = all[i];
          if (prev === undefined || curr === undefined) throw new Error("expected 4 outcomes");
          expect(prev.position).toBeLessThan(curr.position);
        }

        const kindByUrl = new Map(all.map((o) => [o.request.url, o.kind]));
        expect(kindByUrl.get(okUrl)).toBe("succeeded");
        expect(kindByUrl.get(skipUrl)).toBe("skipped");
        expect(kindByUrl.get(failUrl)).toBe("failed");
        expect(kindByUrl.get(blockUrl)).toBe("blocked");

        // Engine-side filters: kinds, labels, codes.
        const badKinds: CrawlRequestOutcome[] = [];
        for await (const outcome of run.outcomes({ kinds: ["failed", "blocked"] })) {
          badKinds.push(outcome);
        }
        expect(badKinds).toHaveLength(2);

        const extraOnly: CrawlRequestOutcome[] = [];
        for await (const outcome of run.outcomes({ labels: [extraLabel] })) {
          extraOnly.push(outcome);
        }
        expect(extraOnly).toHaveLength(2);
        expect(extraOnly.every((o) => o.request.label === "extra")).toBe(true);

        const handlerFailedOnly: CrawlRequestOutcome[] = [];
        for await (const outcome of run.outcomes({ codes: ["handler_failed"] })) {
          handlerFailedOnly.push(outcome);
        }
        expect(handlerFailedOnly).toHaveLength(1);
        expect(handlerFailedOnly[0]?.request.url).toBe(failUrl);

        // failedRequests() matches retryFailed() without and with blocked.
        const failedOnly = await run.failedRequests();
        expect(failedOnly).toHaveLength(1);
        const firstFailed = failedOnly[0];
        if (firstFailed === undefined) throw new Error("expected one failed request");
        expect(firstFailed.request.url).toBe(failUrl);
        const failRequestId = firstFailed.request.id;

        const failedAndBlocked = await run.failedRequests({ blocked: true });
        expect(failedAndBlocked).toHaveLength(2);

        // Retry failed-only: returns ended Run to running; after: "now" sees only later endings.
        const lastOutcome = all.at(-1);
        if (lastOutcome === undefined) throw new Error("expected outcomes");
        const lastBeforeRetry = lastOutcome.position;
        const readmitted = await run.retryFailed({ settings: crawlSettings });
        expect(readmitted).toBe(1);
        await retryAttemptEntered;
        const later: CrawlRequestOutcome[] = [];
        const laterTask = (async () => {
          for await (const outcome of run.outcomes({ after: "now" })) {
            later.push(outcome);
          }
        })();
        // Flush `run.outcomes({ after: "now" })` onto the wire before releasing the retry commit.
        await run.failedRequests();
        releaseRetry();
        const afterRetryReport = await run.finish();
        expect(afterRetryReport.lifecycle).toBe("finished");
        await laterTask;
        expect(later).toHaveLength(1);
        expect(later[0]?.position).toBeGreaterThan(lastBeforeRetry);
        expect(later[0]?.request.id).toBe(failRequestId);
        expect(later[0]?.kind).toBe("succeeded");

        expect(await run.failedRequests()).toEqual([]);
        expect(await run.retryFailed({ settings: crawlSettings })).toBe(0);

        // Retry blocked with blocked: true.
        const blockedLeft = await run.failedRequests({ blocked: true });
        expect(blockedLeft).toHaveLength(1);
        expect(blockedLeft[0]?.request.url).toBe(blockUrl);
        expect(await run.retryFailed({ blocked: true, settings: crawlSettings })).toBe(1);
        const finalReport = await run.finish();
        expect(finalReport.lifecycle).toBe("finished");
        expect(await run.failedRequests({ blocked: true })).toEqual([]);
        expect(await run.retryFailed({ blocked: true, settings: crawlSettings })).toBe(0);

        // Re-admitted request's later outcome supersedes the first by request.id.
        const fullLog: CrawlRequestOutcome[] = [];
        for await (const outcome of run.outcomes()) {
          fullLog.push(outcome);
        }
        const failHistory = fullLog
          .filter((o) => o.request.id === failRequestId)
          .map((o) => o.kind);
        expect(failHistory).toEqual(["failed", "succeeded"]);
      } finally {
        await foxhound.close();
      }
    },
  );
});
