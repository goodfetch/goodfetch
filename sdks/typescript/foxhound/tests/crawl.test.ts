// A program's crawls on an Attached engine of Foxhound's executable of the
// same commit (ADR 0069), which fetches the Proving ground's pages over
// `http` through the test's home (`withBrowser`): the catalog program crawls
// the `catalog` family and the oracle judges its Records, and a handler
// that fails its first attempt shows the Crawl request delivered again and
// the failed attempt's Record never committed (ADRs 0003 and 0093).
import {
  Crawl,
  Foxhound,
  PayloadInvalidError,
  seed,
  type CrawlSettings,
  type DatasetRecord,
} from "@goodfetch/foxhound";
import { judgeTask, taskStartUrl, test } from "@goodfetch/test-support";
import { describe, expect } from "vitest";
import { z } from "zod";
import { catalog, listing, products } from "./programs/catalog.ts";
import { failsOnce } from "./programs/fails-once.ts";

/** The settings every crawl here runs with: its pages over `http`, one handler call at a time. */
const settings: CrawlSettings = { fetch: { tier: "http" }, crawl: { handler: { concurrency: 1 } } };

/** What a `payload_invalid` names: its Label, each issue's path, and the setting a Payload too long passes. */
function refusal(error: unknown): {
  readonly label: unknown;
  readonly paths: unknown[];
  readonly limit: unknown;
} {
  if (!(error instanceof PayloadInvalidError)) {
    throw new Error(`expected a PayloadInvalidError, got ${String(error)}`, { cause: error });
  }
  const { details } = error;
  const issues = Array.isArray(details["issues"]) ? (details["issues"] as unknown[]) : [];
  return {
    label: details["label"],
    paths: issues.map((issue) =>
      typeof issue === "object" && issue !== null && "path" in issue ? issue.path : undefined,
    ),
    limit: details["limit"] ?? undefined,
  };
}

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

  test(
    "refuses a Payload its Label's schema refuses, one not JSON and one longer than crawl.payload.max_size with payload_invalid, on a seed and at enqueue",
    {
      tags: [
        "attached",
        "story:11-1018",
        "error:payload_invalid",
        "setting:crawl.payload.max_size",
        "operation:crawl.start",
        "operation:crawl_request.enqueue",
      ],
    },
    async ({ env, provingGround }) => {
      const start = await taskStartUrl(provingGround, "catalog/1");
      const crawl = new Crawl();
      const page = crawl.label("page");
      // Zod's own parse checks this one in the program, before anything is sent.
      const shelf = crawl.label("shelf", { payload: z.object({ number: z.number().min(1) }) });
      // A hand-written JSON Schema has no parse in the program: the engine checks it.
      const book = crawl.label("book", {
        payload: { type: "object", properties: { isbn: { type: "string" } }, required: ["isbn"] },
      });
      const refused: unknown[] = [];
      crawl.handle(page, async (context) => {
        const long = { isbn: "9".repeat(65 * 1024) };
        for (const enqueued of [
          context.enqueue("/shelf", shelf, { number: 0 }),
          context.enqueue("/isbn", book, { isbn: 7 }),
          context.enqueue("/long", book, long),
        ]) {
          await enqueued.catch((error: unknown) => refused.push(error));
        }
      });
      crawl.handle(shelf, () => undefined);
      crawl.handle(book, () => undefined);
      // oxlint-disable-next-line no-restricted-properties -- `Foxhound.connect` is the SDK's own factory (ADR 0078), not a browser's.
      const foxhound = await Foxhound.connect({ env });

      // `seed` itself refuses a Payload that is not JSON; `startCrawl` the others.
      const seeds = [
        () => seed(start, shelf, { number: 0 }),
        () => seed(start, shelf, { number: Number.NaN }),
        () => seed(start, book, { isbn: 7 }),
      ];
      const seedsRefused: unknown[] = [];
      for (const refusedSeed of seeds) {
        await Promise.resolve()
          .then(async () => foxhound.startCrawl(crawl, { seeds: [refusedSeed()], settings }))
          .catch((error: unknown) => seedsRefused.push(error));
      }
      const run = await foxhound.startCrawl(crawl, { seeds: [seed(start, page)], settings });
      const report = await run.finish();
      await foxhound.close();

      expect(seedsRefused.map(refusal)).toEqual([
        { label: "shelf", paths: ["/number"], limit: undefined },
        { label: "shelf", paths: ["/number"], limit: undefined },
        { label: "book", paths: ["/isbn"], limit: undefined },
      ]);
      expect(refused.map(refusal)).toEqual([
        { label: "shelf", paths: ["/number"], limit: undefined },
        { label: "book", paths: ["/isbn"], limit: undefined },
        { label: "book", paths: [], limit: "crawl.payload.max_size" },
      ]);
      expect(report.lifecycle).toBe("finished");
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
