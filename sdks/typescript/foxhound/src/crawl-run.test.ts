// A crawl's own report and progress (ADR 0120) on the Scripted engine (ADR
// 0069): `startCrawl` sends the crawl's Run ID, tags and Limits, and its
// Run's `progress()`, `stop()`, `report()` and `finish()` answer Foxhound's
// `CrawlRunProgress` and `CrawlRunReport`, the crawl's own parts in
// camelCase and its instants as dates, beside every Run's parts.
import { playScript, test } from "@goodfetch/test-support";
import { describe, expect, expectTypeOf } from "vitest";
import { crawl, listing } from "../tests/programs/two-page-types.ts";
import { seed, type CrawlRun, type CrawlRunProgress, type CrawlRunReport } from "./index.ts";
import { connectOver } from "./testing.ts";

describe("a crawl's Run", () => {
  test(
    "reads its progress until idle, then its stop and its report answer the crawl's report",
    { tags: ["story:11-1080", "story:11-1081", "story:11-1082"] },
    async () => {
      const played = playScript("run/foxhound-crawl-report-and-progress.json");
      const foxhound = await connectOver(played.transport);
      const run = await foxhound.startCrawl(crawl, {
        seeds: [seed("https://shop.test/catalog/1", listing)],
        runId: "kept",
        tags: ["crawls"],
        limits: { requests: 10 },
      });

      let idle: CrawlRunProgress | undefined;
      for await (const value of run.progress()) {
        if (value.idle === true) {
          idle = value;
          break;
        }
      }
      const stopped = await run.stop();
      const report = await run.report();
      const finished = await run.finish();

      expect(idle).toMatchObject({
        idle: true,
        requests: {
          inHumanPause: 0,
          succeeded: 1,
          bySkipReason: [{ name: "max_requests", count: 1 }],
        },
        inFlight: [
          { requestId: "1", phase: "handling", startedAt: new Date("2026-01-01T00:00:01Z") },
        ],
        concurrency: { handlers: 1, handlerConcurrency: 8 },
      });
      expect(stopped).toMatchObject({
        lifecycle: "stopped",
        stopReason: { kind: "abort", by: "program" },
        completeness: "incomplete",
        completenessReasons: [{ code: "crawl.max_requests", count: 1 }],
        endedAt: new Date("2026-01-01T00:00:05Z"),
        checkpoint: { runId: "kept", journalPosition: 7, time: new Date("2026-01-01T00:00:05Z") },
        requests: { succeeded: 1, skipped: 1, bySkipReason: [{ name: "max_requests", count: 1 }] },
        records: [{ dataset: "products", committed: 1, dropped: 0, rejected: 0 }],
        learnedRules: [],
      });
      expect(report).toEqual(stopped);
      expect(finished).toEqual(stopped);
      await foxhound.close();
      expect(played.done()).toEqual({ kind: "finished" });
    },
  );

  test("types its report and progress as the crawl's", () => {
    expectTypeOf<CrawlRun["finish"]>().returns.resolves.toEqualTypeOf<CrawlRunReport>();
    expectTypeOf<CrawlRun["stop"]>().returns.resolves.toEqualTypeOf<CrawlRunReport>();
    expectTypeOf<CrawlRun["report"]>().returns.resolves.toEqualTypeOf<CrawlRunReport>();
    expectTypeOf<CrawlRun["progress"]>().returns.toExtend<AsyncIterable<CrawlRunProgress>>();
  });
});
