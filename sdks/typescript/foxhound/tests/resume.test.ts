// A crawl's Run ID names it for good across stops, crashes, ended Runs and
// refusals (spec #611's #1493; ADRs 0093, 0102, 0103 and 0142).
import { copyFile, mkdtemp, readdir, rm, unlink } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import {
  Crawl,
  DatasetKeyInvalidError,
  DatasetUnknownError,
  Foxhound,
  LabelUnknownError,
  LimitReachedError,
  RunConflictError,
  RunInUseError,
  RunStateUnavailableError,
  SettingNotApplicableError,
  seed,
  type CrawlSettings,
} from "@goodfetch/foxhound";
import { failure, startRecordingLogger, taskStartUrl, test } from "@goodfetch/test-support";
import { describe, expect } from "vitest";
import { z } from "zod";

const itemSchema = z.object({
  id: z.string(),
  title: z.string(),
});
type Item = z.infer<typeof itemSchema>;

const extendedItemSchema = z.object({
  id: z.string(),
  title: z.string(),
  note: z.string().optional(),
});

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

function crawlWithRetry(max: number): NonNullable<CrawlSettings["crawl"]> {
  const retry = { max };
  return { handler: { concurrency: 1 }, retry };
}

describe("Crawl Run ID resume, ended, crash recovery and conflicts (Attached engine)", () => {
  test.override("product", "foxhound");
  test.override("provingGroundSeed", 1493);
  test.override("withBrowser", false);

  test(
    "resumes cleanly after stop(), logs info/warn on resume/ended, and counts handler_lost after a crash",
    {
      tags: [
        "attached",
        "story:11-127",
        "story:3-176",
        "story:3-177",
        "operation:crawl.start",
        "operation:run.started_as",
        "operation:run.stop",
        "error:handler_lost",
      ],
    },
    async ({ env, provingGround }) => {
      const firstUrl = await taskStartUrl(provingGround, "catalog/1");
      const secondUrl = `${firstUrl}?page=2`;
      const home = env.GOODFETCH_HOME;
      if (home === undefined) throw new Error("GOODFETCH_HOME must be set");

      const tempDir = await mkdtemp(join(tmpdir(), "gf-resume-ts-"));
      const { logger, logged } = startRecordingLogger();
      const baseSettings: CrawlSettings = {
        ...siteSettings(provingGround),
        runState: { durability: "full" },
        crawl: crawlWithRetry(1),
      };

      try {
        // 1. Clean stop() right after page 1 pushes and enqueues page 2 (and before
        // page 1 returns so stop() reaches the safe point before page 2 starts).
        const cleanId = "ts-resume-clean";
        const { promise: firstReady, resolve: markFirstReady } = Promise.withResolvers<void>();
        const { promise: releaseFirst, resolve: letFirstFinish } = Promise.withResolvers<void>();

        const crawl1 = new Crawl();
        const items1 = crawl1.dataset("items", { schema: itemSchema, key: ["id"] });
        const first1 = crawl1.label("first");
        const second1 = crawl1.label("second");
        crawl1.handle(first1, async (ctx) => {
          await ctx.push(items1, { id: "one", title: "First" });
          await ctx.enqueue(secondUrl, second1);
          markFirstReady();
          await releaseFirst;
        });
        crawl1.handle(second1, () => undefined);

        // oxlint-disable-next-line no-restricted-properties -- Foxhound.connect is the SDK entry point.
        const fh1 = await Foxhound.connect({ env, logger, settings: baseSettings });
        try {
          const run1 = await fh1.startCrawl(crawl1, {
            seeds: [seed(firstUrl, first1)],
            runId: cleanId,
          });
          expect(run1.startedAs()).toBe("new");
          await firstReady;
          const stopping = run1.stop();
          await run1.keyValueStore.get("__stop_barrier");
          letFirstFinish();
          const stopped = await stopping;
          expect(stopped.lifecycle).toBe("stopped");
        } finally {
          await fh1.close();
        }

        // Resume clean stop with retry.max = 0 -> info logged, still succeeds (no handler_lost counted!), no duplicate records.
        let firstCalls = 0;
        let secondCalls = 0;
        const crawl2 = new Crawl();
        const items2 = crawl2.dataset("items", { schema: itemSchema, key: ["id"] });
        const first2 = crawl2.label("first");
        const second2 = crawl2.label("second");
        crawl2.handle(first2, () => {
          firstCalls += 1;
        });
        crawl2.handle(second2, async (ctx) => {
          secondCalls += 1;
          await ctx.push(items2, { id: "two", title: "Second" });
        });

        const retry = { max: 0 };
        // oxlint-disable-next-line no-restricted-properties -- Foxhound.connect is the SDK entry point.
        const fh2 = await Foxhound.connect({
          env,
          logger,
          settings: {
            ...siteSettings(provingGround),
            runState: { durability: "full" },
            crawl: { handler: { concurrency: 1 }, retry },
          },
        });
        try {
          const resumed = await fh2.startCrawl(crawl2, {
            seeds: [seed(firstUrl, first2)],
            runId: cleanId,
          });
          expect(resumed.startedAs()).toBe("resumed");
          const finished = await resumed.finish();
          expect(finished.lifecycle).toBe("finished");
          expect(firstCalls).toBe(0);
          expect(secondCalls).toBe(1);
          expect(
            finished.requests?.byErrorCode?.some((e) => e.name === "handler_lost") ?? false,
          ).toBe(false);

          const collected: Item[] = [];
          for await (const rec of resumed.records(items2)) {
            collected.push(rec.value);
          }
          expect(collected).toEqual([
            { id: "one", title: "First" },
            { id: "two", title: "Second" },
          ]);

          // Start under ended Run ID -> startedAs === "ended", warn logged, no handler runs.
          let endedCalls = 0;
          const crawl3 = new Crawl();
          crawl3.dataset("items", { schema: itemSchema, key: ["id"] });
          const first3 = crawl3.label("first");
          const second3 = crawl3.label("second");
          crawl3.handle(first3, () => {
            endedCalls += 1;
          });
          crawl3.handle(second3, () => {
            endedCalls += 1;
          });
          const ended = await fh2.startCrawl(crawl3, {
            seeds: [seed(firstUrl, first3)],
            runId: cleanId,
          });
          expect(ended.startedAs()).toBe("ended");
          const endedReport = await ended.report();
          expect(endedReport.lifecycle).toBe("finished");
          expect(endedCalls).toBe(0);
        } finally {
          await fh2.close();
        }

        expect(logged).toEqual([
          [
            "info",
            `The start resumed the Run \`${cleanId}\` from its committed state.`,
            { runId: cleanId, startedAs: "resumed" },
          ],
          [
            "warn",
            `The start found the Run \`${cleanId}\` ended: it is returned as it ended and takes no new work.`,
            { runId: cleanId, startedAs: "ended" },
          ],
        ]);

        // 2. Crash recovery at full durability: checkpoint in-flight state, restore
        // run.sqlite, and verify attempt 1 counts as handler_lost against crawl.retry.max.
        const crashId = "ts-resume-crash";
        const checkpointPath = join(tempDir, "in-flight.sqlite");
        const { promise: crashInFlight, resolve: markCrashInFlight } =
          Promise.withResolvers<void>();
        const { promise: releaseCrashSecond, resolve: letCrashSecondExit } =
          Promise.withResolvers<void>();

        const crashCrawl1 = new Crawl();
        const crashItems1 = crashCrawl1.dataset("items", { schema: itemSchema, key: ["id"] });
        const cf1 = crashCrawl1.label("first");
        const cs1 = crashCrawl1.label("second");
        crashCrawl1.handle(cf1, async (ctx) => {
          await ctx.push(crashItems1, { id: "one", title: "First" });
          await ctx.enqueue(secondUrl, cs1);
        });
        crashCrawl1.handle(cs1, async () => {
          markCrashInFlight();
          await releaseCrashSecond;
        });

        // oxlint-disable-next-line no-restricted-properties -- Foxhound.connect is the SDK entry point.
        const fhCrash1 = await Foxhound.connect({ env, settings: baseSettings });
        try {
          const run = await fhCrash1.startCrawl(crashCrawl1, {
            seeds: [seed(firstUrl, cf1)],
            runId: crashId,
          });
          await crashInFlight;
          await fhCrash1.call("run.checkpoint", { run_id: crashId, location: checkpointPath });
          const closing = run.close();
          letCrashSecondExit();
          await closing;
        } finally {
          letCrashSecondExit();
          await fhCrash1.close();
        }

        const runSqlite = await findRunSqlite(home, crashId);
        await restoreRunFile(checkpointPath, runSqlite);

        let crashFirstCalls = 0;
        let crashSecondCalls = 0;
        const crashCrawl2 = new Crawl();
        const crashItems2 = crashCrawl2.dataset("items", { schema: itemSchema, key: ["id"] });
        const cf2 = crashCrawl2.label("first");
        const cs2 = crashCrawl2.label("second");
        crashCrawl2.handle(cf2, () => {
          crashFirstCalls += 1;
        });
        crashCrawl2.handle(cs2, async (ctx) => {
          crashSecondCalls += 1;
          await ctx.push(crashItems2, { id: "two", title: "Second" });
        });

        // oxlint-disable-next-line no-restricted-properties -- Foxhound.connect is the SDK entry point.
        const fhCrash2 = await Foxhound.connect({ env, settings: baseSettings });
        try {
          const resumed = await fhCrash2.startCrawl(crashCrawl2, {
            seeds: [seed(firstUrl, cf2)],
            runId: crashId,
          });
          expect(resumed.startedAs()).toBe("resumed");
          const finished = await resumed.finish();
          expect(finished.lifecycle).toBe("finished");
          expect(crashFirstCalls).toBe(0);
          expect(crashSecondCalls).toBe(1);
          const recs: Item[] = [];
          for await (const rec of resumed.records(crashItems2)) {
            recs.push(rec.value);
          }
          expect(recs).toHaveLength(2);
        } finally {
          await fhCrash2.close();
        }

        // With retry.max = 0, restoring the same crashed state fails immediately with handler_lost.
        await restoreRunFile(checkpointPath, runSqlite);
        // oxlint-disable-next-line no-restricted-properties -- Foxhound.connect is the SDK entry point.
        const fhCrashZero = await Foxhound.connect({
          env,
          settings: {
            ...siteSettings(provingGround),
            runState: { durability: "full" },
            crawl: { handler: { concurrency: 1 }, retry },
          },
        });
        try {
          const resumedZero = await fhCrashZero.startCrawl(crashCrawl2, {
            seeds: [seed(firstUrl, cf2)],
            runId: crashId,
          });
          expect(resumedZero.startedAs()).toBe("resumed");
          const reportZero = await resumedZero.finish();
          expect(
            reportZero.requests?.byErrorCode?.some(
              (e) => e.name === "handler_lost" && e.count === 1,
            ) ?? false,
          ).toBe(true);
        } finally {
          await fhCrashZero.close();
        }
      } finally {
        await rm(tempDir, { recursive: true, force: true });
      }
    },
  );

  test(
    "raises typed errors for run_in_use, run_conflict, run_state_unavailable, limit_reached and resume validation",
    {
      tags: [
        "attached",
        "story:3-167",
        "story:3-172",
        "story:6-272",
        "operation:crawl.start",
        "operation:run.start",
        "error:run_in_use",
        "error:run_conflict",
        "error:run_state_unavailable",
        "error:limit_reached",
        "error:label_unknown",
        "error:dataset_unknown",
        "error:dataset_key_invalid",
        "error:setting_not_applicable",
      ],
    },
    async ({ env, provingGround }) => {
      const firstUrl = await taskStartUrl(provingGround, "catalog/1");
      const secondUrl = `${firstUrl}?page=2`;
      const home = env.GOODFETCH_HOME;
      if (home === undefined) throw new Error("GOODFETCH_HOME must be set");

      const { promise: firstReady, resolve: markFirstReady } = Promise.withResolvers<void>();
      const { promise: releaseFirst, resolve: letFirstFinish } = Promise.withResolvers<void>();

      const crawl = new Crawl();
      const items = crawl.dataset("items", { schema: itemSchema, key: ["id"] });
      const first = crawl.label("first");
      const second = crawl.label("second");
      crawl.handle(first, async (ctx) => {
        await ctx.push(items, { id: "one", title: "First" });
        await ctx.enqueue(secondUrl, second);
        markFirstReady();
        await releaseFirst;
      });
      crawl.handle(second, () => undefined);

      const firstOrigin = new URL(firstUrl).origin;
      const scopedSettings: CrawlSettings = {
        ...siteSettings(provingGround),
        runState: { durability: "full" },
        crawl: {
          handler: { concurrency: 1 },
          scope: { include: [`${firstOrigin}/**`] },
        },
      };

      const runId = "ts-validation-run";
      // oxlint-disable-next-line no-restricted-properties -- Foxhound.connect is the SDK entry point.
      const fh = await Foxhound.connect({ env });
      try {
        const live = await fh.startCrawl(crawl, {
          seeds: [seed(firstUrl, first)],
          runId,
          settings: scopedSettings,
        });
        await firstReady;

        // 1. run_in_use while live.
        const inUse = await failure(
          fh.startCrawl(crawl, {
            seeds: [seed(firstUrl, first)],
            runId,
            settings: scopedSettings,
          }),
        );
        expect(inUse).toBeInstanceOf(RunInUseError);
        expect(inUse).toMatchObject({ code: "run_in_use", retryable: true });

        const stoppingLive = live.stop();
        await live.keyValueStore.get("__stop_barrier");
        letFirstFinish();
        await stoppingLive;

        // 2. run_conflict on changed seeds.
        const seedConflict = await failure(
          fh.startCrawl(crawl, {
            seeds: [seed(secondUrl, first)],
            runId,
            settings: scopedSettings,
          }),
        );
        expect(seedConflict).toBeInstanceOf(RunConflictError);
        if (!(seedConflict instanceof RunConflictError))
          throw new Error("expected RunConflictError");
        expect(seedConflict.typedDetails?.differences).toContain("seeds");

        // 3. run_conflict on changed Plan file.
        const planId = "ts-plan-conflict";
        const planRun = await fh.start({ runId: planId, plan: "a.plan.json" });
        await planRun.stop();
        const planConflict = await failure(fh.start({ runId: planId, plan: "b.plan.json" }));
        expect(planConflict).toBeInstanceOf(RunConflictError);
        if (!(planConflict instanceof RunConflictError))
          throw new Error("expected RunConflictError");
        expect(planConflict.typedDetails?.differences).toContain("plan");

        // 4. Dropped label with pending request -> label_unknown.
        const missingLabel = new Crawl();
        missingLabel.dataset("items", { schema: itemSchema, key: ["id"] });
        const mlFirst = missingLabel.label("first");
        missingLabel.handle(mlFirst, () => undefined);
        const labelErr = await failure(
          fh.startCrawl(missingLabel, {
            seeds: [seed(firstUrl, mlFirst)],
            runId,
            settings: scopedSettings,
          }),
        );
        expect(labelErr).toBeInstanceOf(LabelUnknownError);

        // 5. Dropped dataset with committed records -> dataset_unknown.
        const missingDs = new Crawl();
        const mdFirst = missingDs.label("first");
        const mdSecond = missingDs.label("second");
        missingDs.handle(mdFirst, () => undefined);
        missingDs.handle(mdSecond, () => undefined);
        const dsErr = await failure(
          fh.startCrawl(missingDs, {
            seeds: [seed(firstUrl, mdFirst)],
            runId,
            settings: scopedSettings,
          }),
        );
        expect(dsErr).toBeInstanceOf(DatasetUnknownError);

        // 6. Changed dataset key -> dataset_key_invalid.
        const wrongKey = new Crawl();
        wrongKey.dataset("items", { schema: itemSchema, key: ["title"] });
        const wkFirst = wrongKey.label("first");
        const wkSecond = wrongKey.label("second");
        wrongKey.handle(wkFirst, () => undefined);
        wrongKey.handle(wkSecond, () => undefined);
        const keyErr = await failure(
          fh.startCrawl(wrongKey, {
            seeds: [seed(firstUrl, wkFirst)],
            runId,
            settings: scopedSettings,
          }),
        );
        expect(keyErr).toBeInstanceOf(DatasetKeyInvalidError);

        // 7. Changed fixed crawl.scope.include -> setting_not_applicable.
        const scopeErr = await failure(
          fh.startCrawl(crawl, {
            seeds: [seed(firstUrl, first)],
            runId,
            settings: {
              ...scopedSettings,
              crawl: {
                handler: { concurrency: 1 },
                scope: { include: ["https://other.test/**"] },
              },
            },
          }),
        );
        expect(scopeErr).toBeInstanceOf(SettingNotApplicableError);

        // 8. Added new Label + widened schema resumes and completes!
        const widened = new Crawl();
        const wideItems = widened.dataset("items", {
          schema: extendedItemSchema,
          key: ["id"],
        });
        const wFirst = widened.label("first");
        const wSecond = widened.label("second");
        const wThird = widened.label("third");
        widened.handle(wFirst, () => undefined);
        widened.handle(wSecond, async (ctx) => {
          await ctx.push(wideItems, { id: "two", title: "Second", note: "widened" });
        });
        widened.handle(wThird, () => undefined);
        const resumed = await fh.startCrawl(widened, {
          seeds: [seed(firstUrl, wFirst)],
          runId,
          settings: scopedSettings,
        });
        expect(resumed.startedAs()).toBe("resumed");
        const report = await resumed.finish();
        expect(report.lifecycle).toBe("finished");
      } finally {
        await fh.close();
      }

      // 9. Missing run state while Run index lists it -> run_state_unavailable (never starts afresh).
      const planRunSqlite = await findRunSqlite(home, "ts-plan-conflict");
      await rm(dirname(planRunSqlite), { recursive: true, force: true });
      // oxlint-disable-next-line no-restricted-properties -- Foxhound.connect is the SDK entry point.
      const fhCorrupt = await Foxhound.connect({ env });
      try {
        const unavailable = await failure(
          fhCorrupt.start({ runId: "ts-plan-conflict", plan: "a.plan.json" }),
        );
        expect(unavailable).toBeInstanceOf(RunStateUnavailableError);
        expect(unavailable).toMatchObject({ code: "run_state_unavailable", retryable: false });
      } finally {
        await fhCorrupt.close();
      }

      // 10. Used-up Limit -> limit_reached with typed details.
      // oxlint-disable-next-line no-restricted-properties -- Foxhound.connect is the SDK entry point.
      const fhCapped = await Foxhound.connect({
        env,
        limits: { requests: 0 },
        settings: siteSettings(provingGround),
      });
      try {
        const limited = await failure(
          fhCapped.startCrawl(crawl, { seeds: [seed(firstUrl, first)] }),
        );
        expect(limited).toBeInstanceOf(LimitReachedError);
        if (!(limited instanceof LimitReachedError)) throw new Error("expected LimitReachedError");
        expect(limited.typedDetails?.usageKind).toBe("requests");
      } finally {
        await fhCapped.close();
      }
    },
  );
});
