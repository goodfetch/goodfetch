// Seeds of a running crawl (`seeds()`, `addSeed`, `addSeeds`, and `crawl.keep_alive`,
// spec #611's #1494; ADRs 0024, 0082, 0093 and 0102).
import {
  Crawl,
  Foxhound,
  PayloadInvalidError,
  seed,
  type CrawlSettings,
  type Enqueued,
} from "@goodfetch/foxhound";
import { provingGroundUrl, test } from "@goodfetch/test-support";
import { describe, expect } from "vitest";
import { z } from "zod";

function fastSiteSettings(provingGround: {
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
    politeness: { domain: { concurrency: 16, delay: 0 } },
  };
}

describe("Seeds of a running crawl (Attached engine)", () => {
  test.override("product", "foxhound");
  test.override("provingGroundSeed", 1494);
  test.override("withBrowser", false);

  test(
    "seeds() answers without a round trip including resume, all-refused ends unless keep_alive, keep_alive takes concurrent addSeed and idle counts toward timeout",
    {
      tags: [
        "attached",
        "story:6-208",
        "story:6-209",
        "story:6-210",
        "story:11-1017",
        "story:11-1018",
        "operation:run.seeds",
        "operation:run.add_seed",
        "operation:run.add_seeds",
        "setting:crawl.keep_alive",
      ],
    },
    async ({ env, provingGround }) => {
      const host = "access.test";
      const rootUrl = provingGroundUrl(provingGround, host, "/corpus/article/1");
      const secondUrl = provingGroundUrl(provingGround, host, "/corpus/article/2");

      // oxlint-disable-next-line no-restricted-properties -- `Foxhound.connect` is the SDK's own factory (ADR 0078).
      const foxhound = await Foxhound.connect({
        env,
        settings: fastSiteSettings(provingGround),
      });

      try {
        // 1. Mixed valid, duplicate, and invalid seeds + resume preserves `seeds()` with no round trip.
        const runId = "ts-live-seeds-resume-1494";
        const { promise: entered, resolve: markEntered } = Promise.withResolvers<void>();
        const { promise: releaseHandler, resolve: letHandlerFinish } =
          Promise.withResolvers<void>();
        const crawl = new Crawl();
        const page = crawl.label("page");
        crawl.handle(page, async () => {
          markEntered();
          await releaseHandler;
        });

        const run = await foxhound.startCrawl(crawl, {
          runId,
          seeds: [seed(rootUrl, page), seed(rootUrl, page), seed("/relative-not-a-url", page)],
        });
        expect(run.startedAs()).toBe("new");
        const initialSeeds = run.seeds();
        expect(initialSeeds).toHaveLength(3);
        expect(initialSeeds[0]?.kind).toBe("added");
        expect(initialSeeds[1]).toMatchObject({ kind: "not_added", reason: "duplicate" });
        expect(initialSeeds[2]).toMatchObject({ kind: "not_added", reason: "invalid_url" });

        await entered;
        const stopping = run.stop();
        await run.keyValueStore.get("__stop_barrier");
        letHandlerFinish();
        const stopped = await stopping;
        expect(stopped.lifecycle).toBe("stopped");
        await run.close();

        const resumedCrawl = new Crawl();
        const resumedPage = resumedCrawl.label("page");
        resumedCrawl.handle(resumedPage, async () => undefined);
        const resumed = await foxhound.startCrawl(resumedCrawl, {
          runId,
          seeds: [
            seed(rootUrl, resumedPage),
            seed(rootUrl, resumedPage),
            seed("/relative-not-a-url", resumedPage),
          ],
        });
        expect(resumed.startedAs()).toBe("resumed");
        expect(resumed.seeds()).toEqual(initialSeeds);
        const resumedReport = await resumed.finish();
        expect(resumedReport.lifecycle).toBe("finished");
        await resumed.close();

        // 2. All seeds refused -> ends immediately without `crawl.keep_alive`.
        const emptyCrawl = new Crawl();
        const emptyPage = emptyCrawl.label("page");
        emptyCrawl.handle(emptyPage, async () => undefined);
        const emptyRun = await foxhound.startCrawl(emptyCrawl, {
          seeds: [seed("/relative-1", emptyPage), seed("/relative-2", emptyPage)],
        });
        expect(emptyRun.seeds()).toEqual([
          expect.objectContaining({ kind: "not_added", reason: "invalid_url" }),
          expect.objectContaining({ kind: "not_added", reason: "invalid_url" }),
        ]);
        const emptyReport = await emptyRun.finish();
        expect(emptyReport.lifecycle).toBe("finished");
        await emptyRun.close();

        // 3. All seeds refused with `crawl.keep_alive`: stays running and `idle: true`,
        // accepts concurrent `addSeed`, and finishes only after `finish()`.
        const seen: string[] = [];
        const keepCrawl = new Crawl();
        const keepPage = keepCrawl.label("page");
        keepCrawl.handle(keepPage, async (ctx) => {
          seen.push(ctx.request.url);
        });
        const keepRun = await foxhound.startCrawl(keepCrawl, {
          seeds: [seed("/relative-only", keepPage)],
          settings: { crawl: { keepAlive: true } },
        });
        const progress = keepRun.progress();
        const firstProgress = await progress[Symbol.asyncIterator]().next();
        expect(firstProgress.done).toBe(false);
        expect(firstProgress.value).toMatchObject({ lifecycle: "running", idle: true });
        progress.close();

        const added = await keepRun.addSeed(secondUrl, keepPage);
        expect(added.kind).toBe("added");
        const keepReport = await keepRun.finish();
        expect(keepReport.lifecycle).toBe("finished");
        expect(seen).toEqual([secondUrl]);
        await keepRun.close();

        // 4. Idle time under `crawl.keep_alive` counts toward `timeout.run`.
        // oxlint-disable-next-line no-restricted-properties -- `Foxhound.connect` is the SDK's own factory (ADR 0078).
        const timeoutFoxhound = await Foxhound.connect({
          env,
          settings: {
            ...fastSiteSettings(provingGround),
            timeout: { run: 600 },
          },
        });
        try {
          const timeoutCrawl = new Crawl();
          const timeoutPage = timeoutCrawl.label("page");
          timeoutCrawl.handle(timeoutPage, async () => undefined);
          const timeoutRun = await timeoutFoxhound.startCrawl(timeoutCrawl, {
            seeds: [seed(rootUrl, timeoutPage)],
            settings: { crawl: { keepAlive: true } },
          });
          let sawIdle = false;
          let finalStep: { readonly lifecycle: string; readonly stopReason?: unknown } | undefined;
          for await (const step of timeoutRun.progress()) {
            if (step.idle) sawIdle = true;
            finalStep = step;
          }
          expect(sawIdle).toBe(true);
          expect(finalStep).toMatchObject({
            lifecycle: "stopped",
            stopReason: { kind: "timeout" },
          });
          await timeoutRun.close();
        } finally {
          await timeoutFoxhound.close();
        }
      } finally {
        await foxhound.close();
      }
    },
  );

  test(
    "ctx.run.addSeed/addSeeds stage with the attempt, addSeeds streams 100 seeds in order, and invalid Payloads fail with payload_invalid",
    {
      tags: [
        "attached",
        "story:6-208",
        "story:6-209",
        "story:11-1017",
        "story:11-1018",
        "operation:run.add_seed",
        "operation:run.add_seeds",
        "error:payload_invalid",
      ],
    },
    async ({ env, provingGround }) => {
      const host = "access.test";
      const rootUrl = provingGroundUrl(provingGround, host, "/corpus/article/1");
      const discardedSingle = provingGroundUrl(provingGround, host, "/corpus/article/10");
      const discardedStream = provingGroundUrl(provingGround, host, "/corpus/article/11");
      const committedSingle = provingGroundUrl(provingGround, host, "/corpus/article/12");
      const committedStream = provingGroundUrl(provingGround, host, "/corpus/article/13");

      // oxlint-disable-next-line no-restricted-properties -- `Foxhound.connect` is the SDK's own factory (ADR 0078).
      const foxhound = await Foxhound.connect({
        env,
        settings: fastSiteSettings(provingGround),
      });

      try {
        // 1. Staging through `ctx.run.addSeed` and `ctx.run.addSeeds`.
        let attemptCount = 0;
        const handledChildren: string[] = [];
        const attempt1Kinds: string[] = [];
        const attempt2Kinds: string[] = [];
        const stageCrawl = new Crawl();
        const root = stageCrawl.label("root");
        const child = stageCrawl.label("child");

        stageCrawl.handle(root, async (ctx) => {
          attemptCount += 1;
          if (attemptCount === 1) {
            const one = await ctx.run.addSeed(discardedSingle, child);
            attempt1Kinds.push(one.kind);
            for await (const ans of ctx.run.addSeeds([seed(discardedStream, child)])) {
              attempt1Kinds.push(ans.kind);
            }
            throw new Error("intentional attempt 1 failure");
          }
          const one = await ctx.run.addSeed(committedSingle, child);
          attempt2Kinds.push(one.kind);
          for await (const ans of ctx.run.addSeeds([seed(committedStream, child)])) {
            attempt2Kinds.push(ans.kind);
          }
        });
        stageCrawl.handle(child, async (ctx) => {
          handledChildren.push(ctx.request.url);
        });

        const retry = { max: 2, handlerFailed: true };
        const stageRun = await foxhound.startCrawl(stageCrawl, {
          seeds: [seed(rootUrl, root)],
          settings: { crawl: { retry } },
        });
        const stageReport = await stageRun.finish();
        expect(stageReport.lifecycle).toBe("finished");
        expect(attempt1Kinds).toEqual(["added", "added"]);
        expect(attempt2Kinds).toEqual(["added", "added"]);
        expect(handledChildren.toSorted()).toEqual([committedSingle, committedStream].toSorted());
        await stageRun.close();

        // 2. 100 seeds from an AsyncIterable in order + `payload_invalid` checks.
        const handledNumbers: number[] = [];
        const batchCrawl = new Crawl();
        const shelf = batchCrawl.label("shelf", {
          payload: z.object({ number: z.number().int().min(1) }),
        });
        const raw = batchCrawl.label("raw", {
          payload: {
            type: "object",
            properties: { code: { type: "string" } },
            required: ["code"],
            additionalProperties: false,
          },
        });
        batchCrawl.handle(shelf, async (ctx) => {
          const num = ctx.request.payload?.["number"];
          if (typeof num === "number") handledNumbers.push(num);
        });
        batchCrawl.handle(raw, async () => undefined);

        const batchRun = await foxhound.startCrawl(batchCrawl, {
          seeds: [seed(rootUrl, shelf, { number: 1 })],
          settings: { crawl: { keepAlive: true, payload: { maxSize: 64 } } },
        });

        // Schema violation (client-side Zod check).
        const zodErr = await batchRun
          .addSeed(provingGroundUrl(provingGround, host, "/corpus/article/bad-zod"), shelf, {
            number: 0,
          })
          .catch((error: unknown) => error);
        expect(zodErr).toBeInstanceOf(PayloadInvalidError);
        expect(
          zodErr instanceof PayloadInvalidError ? (zodErr.typedDetails?.issues?.length ?? 0) : 0,
        ).toBeGreaterThan(0);

        // Non-JSON payload (`NaN`).
        const nanErr = await batchRun
          .addSeed(provingGroundUrl(provingGround, host, "/corpus/article/nan"), raw, {
            code: Number.NaN,
          })
          .catch((error: unknown) => error);
        expect(nanErr).toBeInstanceOf(PayloadInvalidError);

        // Engine-side JSON Schema violation.
        const schemaErr = await batchRun
          .addSeed(provingGroundUrl(provingGround, host, "/corpus/article/bad-schema"), raw, {
            code: 42,
          })
          .catch((error: unknown) => error);
        expect(schemaErr).toBeInstanceOf(PayloadInvalidError);
        expect(
          schemaErr instanceof PayloadInvalidError
            ? (schemaErr.typedDetails?.issues?.length ?? 0)
            : 0,
        ).toBeGreaterThan(0);

        // Exceeding `crawl.payload.max_size` (64 bytes).
        const sizeErr = await batchRun
          .addSeed(provingGroundUrl(provingGround, host, "/corpus/article/too-big"), raw, {
            code: "x".repeat(120),
          })
          .catch((error: unknown) => error);
        expect(sizeErr).toBeInstanceOf(PayloadInvalidError);

        // 100 seeds from an AsyncIterable yielded in exact order.
        async function* generate100() {
          for (let i = 2; i <= 101; i += 1) {
            yield seed(
              provingGroundUrl(provingGround, host, `/corpus/article/1?seed=${String(i)}`),
              shelf,
              { number: i },
            );
          }
        }
        const answers: Enqueued[] = [];
        for await (const answer of batchRun.addSeeds(generate100())) {
          answers.push(answer);
        }
        expect(answers).toHaveLength(100);
        expect(answers.map((answer) => (answer.kind === "added" ? answer.url : ""))).toEqual(
          Array.from({ length: 100 }, (_unused, idx) =>
            provingGroundUrl(provingGround, host, `/corpus/article/1?seed=${String(idx + 2)}`),
          ),
        );

        const batchReport = await batchRun.finish();
        expect(batchReport.lifecycle).toBe("finished");
        expect(handledNumbers).toHaveLength(101);
        await batchRun.close();
      } finally {
        await foxhound.close();
      }
    },
  );
});
