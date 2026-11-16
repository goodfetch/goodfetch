// A program's crawls on an Attached engine of Foxhound's executable of the
// same commit (ADR 0069), which fetches the Proving ground's pages over
// `http` through the test's home (`withBrowser`): the catalog program crawls
// the `catalog` family and the oracle judges its Records, and a handler
// that fails its first attempt shows the Crawl request delivered again and
// the failed attempt's Record never committed (ADRs 0003 and 0093).
import { Foxhound, seed, type DatasetRecord } from "@goodfetch/foxhound";
import { judgeTask, taskStartUrl, test } from "@goodfetch/test-support";
import { describe, expect } from "vitest";
import { catalog, listing, products } from "./programs/catalog.ts";
import { failsOnce } from "./programs/fails-once.ts";

/** The settings every crawl here runs with: its pages over `http`, one handler call at a time. */
const settings = { fetch: { tier: "http" }, crawl: { handler: { concurrency: 1 } } };

/** Runs a crawl from `start` whose handler fails its first attempt, then reads its Dataset. */
async function crawlFailingOnce(
  env: Readonly<Record<string, string>>,
  start: string,
): Promise<{ readonly attempts: number; readonly records: DatasetRecord[] }> {
  const program = failsOnce();
  // oxlint-disable-next-line no-restricted-properties -- `Foxhound.connect` is the SDK's own factory (ADR 0078), not a browser's.
  const foxhound = await Foxhound.connect({ env });
  const run = await foxhound.startCrawl(program.crawl, {
    seeds: [seed(start, program.page)],
    settings: program.settings,
  });
  await run.finish();
  const records: DatasetRecord[] = [];
  for await (const record of run.records(program.attempts)) records.push(record);
  await foxhound.close();
  return { attempts: program.attemptsMade(), records };
}

describe("a program's crawls on an Attached engine", () => {
  test.override("product", "foxhound");
  test.override("provingGroundSeed", 1467);
  test.override("withBrowser", true);

  test(
    "crawls the catalog family with one Label, one handler, one keyed Dataset and one seed, and the oracle accepts its Records",
    { tags: ["client_case/crawl-catalog-program", "attached", "story:11-1013", "story:11-1015"] },
    async ({ env, provingGround, clientCase }) => {
      const start = await taskStartUrl(provingGround, "catalog/1");
      // oxlint-disable-next-line no-restricted-properties -- `Foxhound.connect` is the SDK's own factory (ADR 0078), not a browser's.
      const foxhound = await Foxhound.connect({ env });

      const run = await foxhound.startCrawl(catalog, { seeds: [seed(start, listing)], settings });
      const report = await run.finish();
      const records: DatasetRecord[] = [];
      for await (const record of run.records(products)) records.push(record);
      await foxhound.close();
      const answer = await judgeTask(provingGround, {
        task: "catalog/1",
        records: records.map((record) => record.value),
      });
      clientCase.oracleAnswer(answer);

      await clientCase.check("finish() returns the finished Run's report", () =>
        expect(report.lifecycle).toBe("finished"),
      );
      await clientCase.check("the oracle accepts the Records", () =>
        expect(answer.outcome).toBe("verified"),
      );
    },
  );

  describe("a handler whose first attempt fails, retried", () => {
    test(
      "delivers the Crawl request again after the attempt that failed",
      { tags: ["client_case/handler-at-least-once", "attached", "story:3-41"] },
      async ({ env, provingGround, clientCase }) => {
        const { attempts } = await crawlFailingOnce(
          env,
          await taskStartUrl(provingGround, "catalog/1"),
        );

        await clientCase.check("the handler is called once per attempt: twice", () =>
          expect(attempts).toBe(2),
        );
      },
    );

    test(
      "commits only the Record of the attempt that succeeded",
      { tags: ["client_case/handler-exactly-once", "attached", "story:3-41"] },
      async ({ env, provingGround, clientCase }) => {
        const { records } = await crawlFailingOnce(
          env,
          await taskStartUrl(provingGround, "catalog/1"),
        );

        await clientCase.check("the Dataset holds the second attempt's Record alone", () =>
          expect(records.map((record) => record.value)).toEqual([{ attempt: 2 }]),
        );
      },
    );
  });
});
