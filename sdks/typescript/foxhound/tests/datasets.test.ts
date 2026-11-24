// A handler's `push` on an Attached engine of Foxhound's executable of the
// same commit (ADR 0069), crawling the Proving ground's `catalog` family:
// each Record is checked in the program before it is sent, so one that is
// not JSON or that its Dataset's own schema refuses is never staged, and
// `record_invalid` names it by its position beside those the engine
// refuses; caught, the attempt keeps the others, and escaped, the request
// fails without a retry (ADR 0103).
import {
  Crawl,
  Foxhound,
  RecordInvalidError,
  seed,
  type Dataset,
  type DatasetRecord,
} from "@goodfetch/foxhound";
import { taskStartUrl, test } from "@goodfetch/test-support";
import { describe, expect } from "vitest";
import { z } from "zod";

/** A product whose price has two decimals, a rule its JSON Schema does not carry. */
const product = z.object({
  id: z.string(),
  name: z.string(),
  price: z.string().refine((price) => /^[0-9]+\.[0-9]{2}$/.test(price), "has no two decimals"),
});

/** What `record_invalid` names: each refused Record's position and the paths of its issues. */
function refused(error: unknown): { readonly position: number; readonly paths: string[] }[] {
  if (!(error instanceof RecordInvalidError)) {
    throw new Error(`expected a RecordInvalidError, got ${String(error)}`, { cause: error });
  }
  return (error.typedDetails?.records ?? []).map(({ position, issues }) => ({
    position,
    paths: issues.map(({ path }) => path),
  }));
}

describe("a handler's push on an Attached engine", () => {
  test.override("product", "foxhound");
  test.override("provingGroundSeed", 1486);
  test.override("withBrowser", true);

  test(
    "refuses a Record its Dataset's schema or JSON refuses before it is sent, keeps the others when caught, and fails its request unretried when it escapes",
    {
      tags: ["attached", "story:11-1058", "story:11-1059", "error:record_invalid"],
    },
    async ({ env, provingGround }) => {
      const start = await taskStartUrl(provingGround, "catalog/1");
      const shop = new Crawl();
      const caught = shop.label("caught");
      const escaped = shop.label("escaped");
      const products = shop.dataset("products", { schema: product, key: ["id"] });
      const notes = shop.dataset<{ readonly id?: string; readonly text: number }>("notes", {
        key: ["id"],
      });
      const failures: unknown[] = [];
      let escapedAttempts = 0;
      shop.handle(caught, async (context) => {
        await context
          .push(products, [
            { id: "P-1", name: "Blue kettle", price: "12.50" },
            { id: "P-2", name: "Red kettle", price: "free" },
            { id: "P-3", name: "Green kettle", price: "9.99" },
          ])
          .catch((error: unknown) => failures.push(error));
        // The program refuses the Record that is not JSON, and the engine
        // the one that lacks the key: each is named by its place in the push.
        await context
          .push(notes, [{ id: "N-1", text: 1 }, { id: "N-2", text: Number.NaN }, { text: 3 }])
          .catch((error: unknown) => failures.push(error));
      });
      shop.handle(escaped, async (context) => {
        escapedAttempts += 1;
        await context.push(products, { id: "P-4", name: "Kettle lid", price: "1.5" });
      });
      // oxlint-disable-next-line no-restricted-properties -- `Foxhound.connect` is the SDK's own factory (ADR 0078), not a browser's.
      const foxhound = await Foxhound.connect({ env });

      const run = await foxhound.startCrawl(shop, {
        seeds: [seed(start, caught), seed(start, escaped, { key: "escaped" })],
        settings: {
          fetch: { tier: "http" },
          crawl: { handler: { concurrency: 1 }, retry: { handlerFailed: true } },
        },
      });
      const report = await run.finish();
      const read = async <I extends object, O>(dataset: Dataset<I, O>): Promise<unknown[]> => {
        const records: DatasetRecord[] = [];
        for await (const record of run.records(dataset)) records.push(record);
        return records.map((record) => record.value);
      };
      const committed = { products: await read(products), notes: await read(notes) };
      await foxhound.close();

      expect(failures.map(refused)).toEqual([
        [{ position: 1, paths: ["/price"] }],
        [
          { position: 1, paths: ["/text"] },
          { position: 2, paths: ["/id"] },
        ],
      ]);
      expect(committed).toEqual({
        products: [
          { id: "P-1", name: "Blue kettle", price: "12.50" },
          { id: "P-3", name: "Green kettle", price: "9.99" },
        ],
        notes: [{ id: "N-1", text: 1 }],
      });
      expect(escapedAttempts).toBe(1);
      // `finish()` answers Collie's Run report until #1492 types Foxhound's,
      // so the failed request's Error code is read from its Completeness reason.
      expect(report.completenessReasons).toEqual([
        expect.objectContaining({
          code: "failed_requests",
          count: 1,
          message: expect.stringContaining("(`record_invalid`: 1)"),
        }),
      ]);
    },
  );
});
