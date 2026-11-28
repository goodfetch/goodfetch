// Multi-process Crawl workers in TypeScript (spec #611's #1497; ADR 0024):
// `workers: { count, module, export? }` forks `count - 1` Node processes that
// import `module`, connect to the Attached engine's private endpoint, join
// via `crawl.serve`, commit no Record twice, recover when one worker dies,
// and exit cleanly when the starter closes the Run.
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import {
  Crawl,
  CrawlNotImportableError,
  Foxhound,
  seed,
  SettingInvalidError,
  type CrawlRunReport,
  type FoxhoundSettings,
} from "@goodfetch/foxhound";
import { taskStartUrl, test } from "@goodfetch/test-support";
import { describe, expect } from "vitest";
import catalogCrawl, {
  listingLabel,
  makeLocalCrawl,
  namedCatalogCrawl,
  pagesDataset,
} from "./fixtures/workers-crawl.ts";

const FIXTURE_MODULE = fileURLToPath(new URL("./fixtures/workers-crawl.ts", import.meta.url));

function isAlive(pid: number): boolean {
  try {
    process.kill(pid, 0);
    return true;
  } catch {
    return false;
  }
}

function fastSettings(provingGround: {
  readonly resolve: Readonly<Record<string, string>>;
}): FoxhoundSettings {
  const resolve: Record<string, readonly string[]> = {};
  for (const [host, address] of Object.entries(provingGround.resolve)) {
    resolve[host] = [address];
  }
  return {
    fetch: { tier: "http" },
    politeness: { domain: { concurrency: 16, delay: 0 } },
    network: { resolve },
    egress: { privateRanges: Object.keys(resolve) },
  };
}

describe("workers: one crawl across Node processes", () => {
  test.override("product", "foxhound");
  test.override("provingGroundSeed", 1467);
  test.override("withBrowser", false);

  test(
    "runs three processes on the catalog family without duplicate Records, refuses unimportable Crawls, recovers a killed worker, and exits workers on close",
    {
      tags: ["attached", "story:3-17", "operation:crawl.serve", "error:crawl_not_importable"],
      timeout: 90_000,
    },
    async ({ env, provingGround }) => {
      const catalogStart = await taskStartUrl(provingGround, "catalog/1");
      const markerDir = mkdtempSync(join(tmpdir(), "gf-ts-workers-"));

      // oxlint-disable-next-line no-restricted-properties -- `Foxhound.connect` is the SDK's own factory (ADR 0078).
      const foxhound = await Foxhound.connect({
        env,
        settings: fastSettings(provingGround),
      });
      try {
        // 1. Function-local Crawl / missing module / non-Crawl export refused with `crawl_not_importable` + `fix`.
        const localCrawl = makeLocalCrawl();
        const localLabel = new Crawl().label("page");
        await expect(
          foxhound.startCrawl(localCrawl, {
            seeds: [seed(catalogStart, localLabel)],
            workers: 3,
          }),
        ).rejects.toSatisfy((error: unknown) => {
          expect(error).toBeInstanceOf(CrawlNotImportableError);
          if (!(error instanceof CrawlNotImportableError)) return false;
          expect(error.retryable).toBe(false);
          expect(error.fix).toBeTruthy();
          return true;
        });

        await expect(
          foxhound.startCrawl(localCrawl, {
            seeds: [seed(catalogStart, localLabel)],
            workers: { count: 3, module: FIXTURE_MODULE, export: "makeLocalCrawl" },
          }),
        ).rejects.toSatisfy((error: unknown) => {
          expect(error).toBeInstanceOf(CrawlNotImportableError);
          if (!(error instanceof CrawlNotImportableError)) return false;
          expect(error.fix).toBeTruthy();
          return true;
        });

        await expect(
          foxhound.startCrawl(catalogCrawl, {
            seeds: [seed(catalogStart, listingLabel, {})],
            workers: 0,
          }),
        ).rejects.toBeInstanceOf(SettingInvalidError);

        // 2. Three processes on the catalog family: 3 distinct PIDs handle requests, no Record committed twice.
        const run = await foxhound.startCrawl(catalogCrawl, {
          seeds: [seed(catalogStart, listingLabel, { delayMs: 80 })],
          settings: { crawl: { handler: { concurrency: 1 } } },
          workers: { count: 3, module: FIXTURE_MODULE },
        });
        try {
          const report: CrawlRunReport = await run.finish();
          expect(report.lifecycle).toBe("finished");
          expect(report.completeness).toBe("complete");

          const urls: string[] = [];
          const pids = new Set<number>();
          for await (const record of run.records(pagesDataset)) {
            urls.push(record.value.url);
            pids.add(record.value.pid);
          }
          expect(urls.length).toBeGreaterThanOrEqual(6);
          expect(new Set(urls).size).toBe(urls.length);
          expect(pids.size).toBe(3);
        } finally {
          await run.close();
        }

        // 3. Killing one worker mid-attempt leaves its leased attempt `client_gone`,
        //    retried on a surviving process, and the crawl completes with no duplicate Record.
        const killMarker = join(markerDir, "killed-once");
        const retry = { max: 2 };
        const recoveryRun = await foxhound.startCrawl(namedCatalogCrawl, {
          seeds: [
            seed(catalogStart, listingLabel, {
              starterPid: process.pid,
              killMarker,
              delayMs: 80,
            }),
          ],
          settings: { crawl: { handler: { concurrency: 1 }, retry } },
          workers: { count: 3, module: FIXTURE_MODULE, export: "namedCatalogCrawl" },
        });
        try {
          const report = await recoveryRun.finish();
          expect(report.lifecycle).toBe("finished");
          expect(report.completeness).toBe("complete");

          const committedUrls: string[] = [];
          for await (const record of recoveryRun.records(pagesDataset)) {
            committedUrls.push(record.value.url);
          }
          expect(committedUrls.length).toBeGreaterThanOrEqual(6);
          expect(new Set(committedUrls).size).toBe(committedUrls.length);

          let sawClientGoneRetry = false;
          for await (const outcome of recoveryRun.outcomes()) {
            if (
              outcome.attempts > 1 &&
              outcome.errors.some((error) => error.code === "client_gone")
            ) {
              sawClientGoneRetry = true;
            }
          }
          expect(sawClientGoneRetry).toBe(true);
        } finally {
          await recoveryRun.close();
        }

        // 4. Workers exit cleanly when the starter closes an active keep-alive Run.
        const keepRun = await foxhound.startCrawl(catalogCrawl, {
          seeds: [seed(catalogStart, listingLabel, { delayMs: 80 })],
          settings: { crawl: { handler: { concurrency: 1 }, keepAlive: true } },
          workers: { count: 3, module: FIXTURE_MODULE },
        });
        const workerPids = new Set<number>();
        for await (const record of keepRun.records(pagesDataset)) {
          if (record.value.pid !== process.pid) {
            workerPids.add(record.value.pid);
          }
          if (workerPids.size >= 2) break;
        }
        expect(workerPids.size).toBe(2);
        await keepRun.close();
        for (const pid of workerPids) {
          expect(isAlive(pid)).toBe(false);
        }
      } finally {
        await foxhound.close();
        rmSync(markerDir, { recursive: true, force: true });
      }
    },
  );
});
