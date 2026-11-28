// Reading a crawl Run another program started: `openCrawlRun` and
// `CrawlRunView` (ADRs 0102 and 0103; issue 1489).
import { copyFile, mkdtemp, readdir, rm, unlink } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import {
  ClosedError,
  Crawl,
  Foxhound,
  RunInterruptedError,
  seed,
  type CrawlSettings,
} from "@goodfetch/foxhound";
import { failure, taskStartUrl, test } from "@goodfetch/test-support";
import { describe, expect } from "vitest";
import { z } from "zod";

const itemSchema = z.object({
  id: z.string(),
  title: z.string(),
});

async function findRunSqlite(home: string, runId: string): Promise<string> {
  const entries = await readdir(home);
  for (const entry of entries) {
    const candidate = join(home, entry, "runs", runId, "run.sqlite");
    const runs = await readdir(join(home, entry, "runs")).catch(() => [] as string[]);
    if (runs.includes(runId)) return candidate;
  }
  const direct = join(home, "runs", runId, "run.sqlite");
  const directRuns = await readdir(join(home, "runs")).catch(() => [] as string[]);
  if (directRuns.includes(runId)) return direct;
  throw new Error(`run.sqlite for ${runId} not found under ${home}`);
}

async function restoreRunFile(src: string, dst: string): Promise<void> {
  await unlink(`${dst}-wal`).catch(() => undefined);
  await unlink(`${dst}-shm`).catch(() => undefined);
  await copyFile(src, dst);
}

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

describe("Foxhound.openCrawlRun and CrawlRunView", () => {
  test.override("product", "foxhound");
  test.override("provingGroundSeed", 1489);
  test.override("withBrowser", false);

  test(
    "opens a running crawl by ID, reads Records live, ends on interrupted Run with run_interrupted on report(), and never blocks resume",
    {
      tags: ["attached", "story:11-1068"],
    },
    async ({ env, provingGround }) => {
      const firstUrl = await taskStartUrl(provingGround, "catalog/1");
      const runId = "ts-open-crawl-run-live";

      let resolveFirstCommitted!: () => void;
      const firstCommitted = new Promise<void>((resolve) => {
        resolveFirstCommitted = resolve;
      });

      const writerCrawl = new Crawl();
      const page = writerCrawl.label("page");
      const items = writerCrawl.dataset("items", { schema: itemSchema, key: ["id"] });
      writerCrawl.handle(page, async (ctx) => {
        await ctx.push(items, [
          { id: "item-1", title: "First" },
          { id: "item-2", title: "Second" },
        ]);
        await ctx.run.keyValueStore.set("stage", "one");
        resolveFirstCommitted();
      });

      // oxlint-disable-next-line no-restricted-properties -- `Foxhound.connect` is the SDK's own factory (ADR 0078).
      const writer = await Foxhound.connect({ env });
      // oxlint-disable-next-line no-restricted-properties -- second program opening a view on the same home.
      const reader = await Foxhound.connect({ env });

      const run = await writer.startCrawl(writerCrawl, {
        runId,
        seeds: [seed(firstUrl, page)],
        settings: {
          ...siteSettings(provingGround),
          crawl: { handler: { concurrency: 1 }, keepAlive: true },
        },
      });

      await firstCommitted;

      // Second program declares its own Dataset handle of the same name and opens a view.
      const readerCrawl = new Crawl();
      const readerItems = readerCrawl.dataset("items", { schema: itemSchema, key: ["id"] });
      const view = await reader.openCrawlRun(runId);
      expect(view.id()).toBe(runId);

      // Read the first committed Record live.
      const liveRecords = view.records(readerItems);
      const iterator = liveRecords[Symbol.asyncIterator]();
      const firstResult = await iterator.next();
      expect(firstResult.done).toBe(false);
      expect(firstResult.value?.number).toBe(1);
      expect(firstResult.value?.value).toEqual({ id: "item-1", title: "First" });

      // Snapshot the running Run before stopping the writer, then restore it
      // so the Run is interrupted (no live writer, not ended).
      const home = env.GOODFETCH_HOME;
      if (home === undefined) throw new Error("GOODFETCH_HOME must be set");
      const tempDir = await mkdtemp(join(tmpdir(), "gf-open-crawl-run-ts-"));
      try {
        const checkpointPath = join(tempDir, "interrupted.sqlite");
        const checkpoint = await run.checkpoint();
        await checkpoint.saveAs(checkpointPath);

        await run.stop();
        await writer.close();

        // The live sequence yields item-2 then ends cleanly after the last committed Record.
        const secondResult = await iterator.next();
        expect(secondResult.done).toBe(false);
        expect(secondResult.value?.number).toBe(2);
        const afterStop = await iterator.next();
        expect(afterStop.done).toBe(true);

        const runSqlite = await findRunSqlite(home, runId);
        await restoreRunFile(checkpointPath, runSqlite);

        // Reading Records on the interrupted Run ends cleanly after the committed Records.
        const interruptedRecords = [];
        for await (const rec of view.records(readerItems)) interruptedRecords.push(rec);
        expect(interruptedRecords).toHaveLength(2);
        expect(interruptedRecords[0]?.value).toEqual({ id: "item-1", title: "First" });

        // `report()` on an interrupted Run raises `run_interrupted` with a fix naming the Run ID.
        const interrupted = await failure(view.report());
        expect(interrupted).toBeInstanceOf(RunInterruptedError);
        expect(interrupted instanceof RunInterruptedError ? interrupted.fix : undefined).toContain(
          runId,
        );

        // View reads Key-value store and progress without taking a writer lock.
        expect(await view.keyValueStore.get("stage")).toBe("one");
        const progressItems = [];
        for await (const p of view.progress()) progressItems.push(p);
        expect(progressItems.length).toBeGreaterThan(0);

        // Holding the view takes no writer lock: a third connection resumes the Run.
        const resumeCrawl = new Crawl();
        const resumePage = resumeCrawl.label("page");
        resumeCrawl.dataset("items", { schema: itemSchema, key: ["id"] });
        resumeCrawl.handle(resumePage, async () => undefined);

        // oxlint-disable-next-line no-restricted-properties -- resuming program.
        const resumer = await Foxhound.connect({ env });
        const resumed = await resumer.startCrawl(resumeCrawl, {
          runId,
          seeds: [seed(firstUrl, resumePage)],
          settings: {
            ...siteSettings(provingGround),
            crawl: { handler: { concurrency: 1 }, keepAlive: true },
          },
        });
        expect(resumed.startedAs()).toBe("resumed");
        await resumed.finish();

        // Now that the Run ended, the view reads report, records(after), export, outcomes, failedRequests.
        const report = await view.report();
        expect(report.runId).toBe(runId);

        const afterOne = [];
        for await (const rec of view.records(readerItems, { after: 1 })) afterOne.push(rec);
        expect(afterOne).toHaveLength(1);
        expect(afterOne[0]?.number).toBe(2);
        expect(afterOne[0]?.value).toEqual({ id: "item-2", title: "Second" });

        const exported = await view.export(readerItems, { format: "json" });
        expect(exported.records).toBe(2);

        const outcomes = [];
        for await (const outcome of view.outcomes()) outcomes.push(outcome);
        expect(outcomes).toHaveLength(1);

        const failed = await view.failedRequests();
        expect(failed).toEqual([]);

        await view.close();
        await expect(view.report()).rejects.toBeInstanceOf(ClosedError);

        await resumer.close();
        await reader.close();
      } finally {
        await rm(tempDir, { recursive: true, force: true });
      }
    },
  );
});
