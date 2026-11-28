// `run.progress()` says why a slow crawl is slow (spec #611's #1496; ADRs 0040,
// 0086, 0102 and 0118).
import {
  Crawl,
  Foxhound,
  seed,
  sitemap,
  type CrawlRunProgress,
  type CrawlSettings,
} from "@goodfetch/foxhound";
import { provingGroundUrl, test } from "@goodfetch/test-support";
import { describe, expect } from "vitest";

function resolveSettings(provingGround: {
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

describe("progress() says why a slow crawl is slow (Attached engine)", () => {
  test.override("product", "foxhound");
  test.override("provingGroundSeed", 1496);
  test.override("withBrowser", false);

  test(
    "reports default politeness waits, 503 Retry-After backoff, disallowing robots.txt, and handler/governor concurrency",
    {
      tags: ["attached", "story:3-162", "story:11-129", "story:11-1080", "operation:run.progress"],
    },
    async ({ env, provingGround }) => {
      const accessHost = "access.test";
      const edgeHost = "fetch-edge.test";

      // 1. Default one-request-per-domain politeness + concurrency details.
      // oxlint-disable-next-line no-restricted-properties -- `Foxhound.connect` is the SDK's own factory (ADR 0078).
      const foxhound = await Foxhound.connect({
        env,
        settings: resolveSettings(provingGround),
      });
      try {
        const crawl = new Crawl();
        const page = crawl.label("page");
        crawl.handle(page, async () => {
          await new Promise<void>((resolve) => {
            setTimeout(resolve, 120);
          });
        });

        const run = await foxhound.startCrawl(crawl, {
          seeds: [
            seed(provingGroundUrl(provingGround, accessHost, "/corpus/article/1"), page),
            seed(provingGroundUrl(provingGround, accessHost, "/corpus/article/2"), page),
            seed(provingGroundUrl(provingGround, accessHost, "/corpus/article/3"), page),
          ],
        });

        let sawPolitenessWait = false;
        let sawNextAdmittedAt = false;
        let sawSettingKey: string | undefined;
        let sawSettingLayer: string | undefined;
        let sawConcurrency = false;

        for await (const step of run.progress()) {
          for (const group of step.waiting ?? []) {
            if (group.reason === "domain_concurrency" || group.reason === "domain_delay") {
              if (group.domains.some((d) => d.domain === accessHost)) {
                sawPolitenessWait = true;
              }
              if (group.domains.some((d) => d.nextAdmittedAt instanceof Date)) {
                sawNextAdmittedAt = true;
              }
              if (group.setting !== undefined && group.setting !== null) {
                sawSettingKey = group.setting.key;
                sawSettingLayer = group.setting.layer;
              }
            }
          }
          if (
            step.concurrency?.handlerConcurrency === 16 &&
            step.concurrency.handlerConcurrencyLayer === "default" &&
            step.concurrency.governor !== undefined &&
            step.concurrency.governor !== null &&
            step.concurrency.allowanceSetBy !== undefined &&
            step.concurrency.allowanceSetBy !== null
          ) {
            sawConcurrency = true;
          }
        }
        await run.close();
        expect(sawPolitenessWait).toBe(true);
        expect(sawNextAdmittedAt).toBe(true);
        expect(["politeness.domain.concurrency", "politeness.domain.delay"]).toContain(
          sawSettingKey,
        );
        expect(sawSettingLayer).toBe("default");
        expect(sawConcurrency).toBe(true);

        // 2. 503 with Retry-After -> `backoff` wait group.
        const backoffCrawl = new Crawl();
        const backoffPage = backoffCrawl.label("page");
        backoffCrawl.handle(backoffPage, async () => undefined);
        const backoffRun = await foxhound.startCrawl(backoffCrawl, {
          seeds: [
            seed(
              provingGroundUrl(provingGround, edgeHost, "/retry-after/503/seconds"),
              backoffPage,
            ),
            seed(provingGroundUrl(provingGround, edgeHost, "/after-503"), backoffPage),
          ],
          settings: { politeness: { domain: { concurrency: 1, delay: 0 } } },
        });
        let sawBackoff = false;
        let sawBackoffNextAdmitted = false;
        for await (const step of backoffRun.progress()) {
          const group = step.waiting?.find((g) => g.reason === "backoff");
          if (group !== undefined && group.domains.some((d) => d.domain === edgeHost)) {
            sawBackoff = true;
            if (group.domains.some((d) => d.nextAdmittedAt instanceof Date)) {
              sawBackoffNextAdmitted = true;
            }
          }
        }
        await backoffRun.close();
        expect(sawBackoff).toBe(true);
        expect(sawBackoffNextAdmitted).toBe(true);

        // 3. Disallowing robots.txt -> `robots_txt` in `requests.bySkipReason`.
        const robotsCrawl = new Crawl();
        const robotsPage = robotsCrawl.label("page");
        robotsCrawl.handle(robotsPage, async () => undefined);
        const robotsRun = await foxhound.startCrawl(robotsCrawl, {
          seeds: [seed(provingGroundUrl(provingGround, edgeHost, "/private/secret"), robotsPage)],
        });
        let lastRobots: CrawlRunProgress | undefined;
        for await (const step of robotsRun.progress()) {
          lastRobots = step;
        }
        await robotsRun.close();
        expect(lastRobots?.requests?.skipped).toBe(1);
        expect(lastRobots?.requests?.bySkipReason).toEqual(
          expect.arrayContaining([expect.objectContaining({ name: "robots_txt", count: 1 })]),
        );
      } finally {
        await foxhound.close();
      }
    },
  );

  test(
    "reports sitemaps added/not-added by reason, keep-alive idle, immediate first value, and coalescing under load",
    {
      tags: [
        "attached",
        "story:3-162",
        "story:6-207",
        "story:11-129",
        "story:11-1080",
        "operation:run.progress",
      ],
    },
    async ({ env, provingGround }) => {
      const sitemapHost = "sitemap-seed.test";
      const accessHost = "access.test";

      // oxlint-disable-next-line no-restricted-properties -- `Foxhound.connect` is the SDK's own factory (ADR 0078).
      const foxhound = await Foxhound.connect({
        env,
        settings: {
          ...resolveSettings(provingGround),
          politeness: { domain: { concurrency: 16, delay: 0 } },
        },
      });
      try {
        // 1. Sitemaps added and not-added by reason.
        const smCrawl = new Crawl();
        const book = smCrawl.label("book");
        const shelf = smCrawl.label("shelf");
        const author = smCrawl.label("author");
        for (const label of [book, shelf, author]) {
          smCrawl.handle(label, async () => undefined);
        }
        const smRun = await foxhound.startCrawl(smCrawl, {
          seeds: [
            seed(provingGroundUrl(provingGround, sitemapHost, "/books/1"), book),
            sitemap(provingGroundUrl(provingGround, sitemapHost, "/sitemap-index.xml"), [
              ["**/books/*", book],
              ["**/books/**", shelf],
              ["**/authors/*", author],
            ]),
          ],
        });
        let lastSitemap: CrawlRunProgress | undefined;
        for await (const step of smRun.progress()) {
          lastSitemap = step;
        }
        await smRun.close();
        const smReport = lastSitemap?.sitemaps?.[0];
        expect(smReport?.added).toBeGreaterThan(0);
        expect(smReport?.notAdded).toBeGreaterThan(0);
        const reasons = (smReport?.notAddedByReason ?? []).map((entry) => entry.name);
        expect(reasons).toEqual(expect.arrayContaining(["duplicate", "no_rule", "out_of_scope"]));

        // 2. Keep-alive with no work: first value arrives at once with `idle: true`.
        const keepCrawl = new Crawl();
        const keepPage = keepCrawl.label("page");
        keepCrawl.handle(keepPage, async () => undefined);
        const keepRun = await foxhound.startCrawl(keepCrawl, {
          seeds: [seed("/relative-only", keepPage)],
          settings: { crawl: { keepAlive: true } },
        });
        const keepProgress = keepRun.progress();
        const firstKeep = await keepProgress[Symbol.asyncIterator]().next();
        expect(firstKeep.done).toBe(false);
        expect(firstKeep.value).toMatchObject({ lifecycle: "running", idle: true });
        keepProgress.close();
        await keepRun.finish();
        await keepRun.close();

        // 3. Coalescing under load: 30 fast requests coalesce into fewer delivered values.
        const burstCrawl = new Crawl();
        const burstPage = burstCrawl.label("page");
        burstCrawl.handle(burstPage, async () => undefined);
        const burstSeeds = Array.from({ length: 30 }, (_unused, idx) =>
          seed(
            provingGroundUrl(
              provingGround,
              accessHost,
              `/corpus/article/1?burst=${String(idx + 1)}`,
            ),
            burstPage,
          ),
        );
        const [firstSeed, ...restSeeds] = burstSeeds;
        if (firstSeed === undefined) throw new Error("expected burst seeds");
        const burstRun = await foxhound.startCrawl(burstCrawl, {
          seeds: [firstSeed, ...restSeeds],
        });
        const burstProgress = burstRun.progress();
        const burstIter = burstProgress[Symbol.asyncIterator]();
        const firstBurst = await burstIter.next();
        expect(firstBurst.done).toBe(false);
        const burstReport = await burstRun.finish();
        expect(burstReport.lifecycle).toBe("finished");

        let delivered = 1;
        let finalSucceeded =
          firstBurst.done === false ? (firstBurst.value.requests?.succeeded ?? 0) : 0;
        for await (const step of burstProgress) {
          delivered += 1;
          finalSucceeded = step.requests?.succeeded ?? 0;
        }
        await burstRun.close();
        expect(finalSucceeded).toBe(30);
        expect(delivered).toBeLessThan(30);
      } finally {
        await foxhound.close();
      }
    },
  );
});
