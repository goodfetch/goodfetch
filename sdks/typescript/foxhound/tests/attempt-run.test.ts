// `ctx.run` on an Attached engine of Foxhound's executable of the same
// commit (ADR 0069), crawling the Proving ground's `catalog` family: it
// reaches the crawl's Key-value store bound to the attempt, so a write
// through it commits with the attempt's Records and enqueues or is
// discarded with them, while a write through the program's own Run value,
// captured by the handler, commits at once, on every attempt that runs it
// (ADRs 0093 and 0103).
import { Crawl, Foxhound, seed, type CrawlRun, type DatasetRecord } from "@goodfetch/foxhound";
import { judgeTask, taskStartUrl, test } from "@goodfetch/test-support";
import { describe, expect } from "vitest";
import { crawlListing, type Product } from "./programs/catalog.ts";

/** The settings every crawl here runs with: its pages over `http`, one handler call at a time, a failed handler retried. */
const settings = {
  fetch: { tier: "http" },
  crawl: { handler: { concurrency: 1 }, retry: { handlerFailed: true } },
} as const;

/** What the `number`th attempt sees of its own writes and of its captured Run's, having first found `found`. */
function sees(number: number, found: unknown[]): unknown {
  return {
    found,
    throughAttempt: [number, { content: new Uint8Array([number]), contentType: "image/png" }],
    throughProgram: [null, null, number],
    deleted: [true, null],
  };
}

describe("a handler's `ctx.run` on an Attached engine", () => {
  test.override("product", "foxhound");
  test.override("provingGroundSeed", 1486);
  test.override("withBrowser", true);

  test(
    "commits each retried handler's outputs once and discards its failed attempt's Records, enqueue and Key-value write",
    { tags: ["attached", "story:11-1058", "story:11-1064"] },
    async ({ env, provingGround }) => {
      const start = await taskStartUrl(provingGround, "catalog/1");
      const shop = new Crawl();
      const listing = shop.label("listing");
      const decoy = shop.label("decoy");
      const products = shop.dataset<Product>("products", { key: ["id"] });
      const tries = new Map<string, number>();
      shop.handle(listing, async (context) => {
        // The page is named by the first product it lists.
        const { values } = await context.page.read({
          first: { locator: { kind: "css", selector: "li.product" }, attribute: "data-id" },
        });
        const name = String(values["first"]);
        const attempt = (tries.get(name) ?? 0) + 1;
        tries.set(name, attempt);
        await crawlListing(context, listing, products);
        if (attempt === 1) {
          await context.enqueue(start, decoy, { key: `decoy of ${name}` });
          await context.run.keyValueStore.set(`failed:${name}`, attempt);
          throw new Error("the first attempt fails after its outputs");
        }
        await context.run.keyValueStore.set(`done:${name}`, attempt);
      });
      let decoys = 0;
      shop.handle(decoy, () => {
        decoys += 1;
      });
      // oxlint-disable-next-line no-restricted-properties -- `Foxhound.connect` is the SDK's own factory (ADR 0078), not a browser's.
      const foxhound = await Foxhound.connect({ env });

      const run = await foxhound.startCrawl(shop, { seeds: [seed(start, listing)], settings });
      await run.finish();
      const records: DatasetRecord[] = [];
      for await (const record of run.records(products)) records.push(record);
      const names = [...tries.keys()];
      const stored = await Promise.all(
        names.map(async (name) => [
          await run.keyValueStore.get(`failed:${name}`),
          await run.keyValueStore.get(`done:${name}`),
        ]),
      );
      await foxhound.close();
      const answer = await judgeTask(provingGround, {
        task: "catalog/1",
        records: records.map((record) => record.value),
      });

      expect(answer.outcome).toBe("verified");
      expect(names.length).toBeGreaterThan(1);
      expect([...tries.values()]).toEqual(names.map(() => 2));
      expect(decoys).toBe(0);
      expect(stored).toEqual(names.map(() => [null, 2]));
    },
  );

  test(
    "stages a Key-value write through `ctx.run` with its attempt, where a captured program Run's write commits at once on every attempt",
    {
      tags: [
        "attached",
        "story:11-1064",
        "operation:key_value_store.get",
        "operation:key_value_store.set",
        "operation:key_value_store.get_bytes",
        "operation:key_value_store.set_bytes",
        "operation:key_value_store.delete",
      ],
    },
    async ({ env, provingGround }) => {
      const start = await taskStartUrl(provingGround, "catalog/1");
      const crawl = new Crawl();
      const page = crawl.label("page");
      const { promise: started, resolve: opened } = Promise.withResolvers<CrawlRun>();
      const seen: unknown[] = [];
      const ids: string[] = [];
      let attempt = 0;
      crawl.handle(page, async (context) => {
        attempt += 1;
        const staged = context.run.keyValueStore;
        // The trap: the program's own Run value, captured by its handler.
        const captured = (await started).keyValueStore;
        ids.push(context.run.id());
        const found = [await staged.get("staged"), await staged.get("captured")];
        await staged.set("staged", attempt);
        await staged.setBytes("bytes", new Uint8Array([attempt]), "image/png");
        await captured.set("captured", attempt);
        seen.push({
          found,
          throughAttempt: [await staged.get("staged"), await staged.getBytes("bytes")],
          throughProgram: [
            await captured.get("staged"),
            await captured.getBytes("bytes"),
            await captured.get("captured"),
          ],
          deleted: [await staged.delete("bytes"), await staged.getBytes("bytes")],
        });
        if (attempt === 1) throw new Error("the first attempt fails after its writes");
      });
      // oxlint-disable-next-line no-restricted-properties -- `Foxhound.connect` is the SDK's own factory (ADR 0078), not a browser's.
      const foxhound = await Foxhound.connect({ env });

      const run = await foxhound.startCrawl(crawl, { seeds: [seed(start, page)], settings });
      opened(run);
      await run.finish();
      const committed = [
        await run.keyValueStore.get("staged"),
        await run.keyValueStore.getBytes("bytes"),
        await run.keyValueStore.get("captured"),
      ];
      await foxhound.close();

      // The failed attempt's staged write is gone, and its captured one stays.
      expect(seen).toEqual([sees(1, [null, null]), sees(2, [null, 1])]);
      expect(ids).toEqual([run.id(), run.id()]);
      expect(committed).toEqual([2, null, 2]);
    },
  );
});
