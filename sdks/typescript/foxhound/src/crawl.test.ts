// A Crawl on the Scripted engine (ADR 0069): the program declares its
// Labels, handlers and Dataset on a `Crawl` value, starts it on a
// Connection, and serves the engine's `crawl_request.handle` requests on
// the event loop. The script fails the test on a message it does not
// expect and on a step left unused.
import { playScript, protocolCounts, test } from "@goodfetch/test-support";
import { describe, expect, expectTypeOf, vi } from "vitest";
import { crawl, listing, products } from "../tests/programs/two-page-types.ts";
import {
  ClosedError,
  Crawl,
  seed,
  type CrawlContext,
  type DatasetRecord,
  type Page,
} from "./index.ts";
import { connectOver } from "./testing.ts";

describe("a Crawl", () => {
  test(
    "crawls two page types into a Dataset: each handler call answered, then finish() and a records() read",
    { tags: ["story:11-1013", "story:11-1014", "story:11-1015", "story:11-1025"] },
    async () => {
      const played = playScript("run/foxhound-crawl-two-page-types.json");
      const foxhound = await connectOver(played.transport);

      const run = await foxhound.startCrawl(crawl, {
        seeds: [seed("https://shop.test/catalog/1", listing)],
      });
      // The Scripted engine plays its steps in order, so the program
      // finishes once both handler calls were answered, as a crawl's Run
      // finishes once every work item is done.
      await vi.waitFor(() => {
        expect(protocolCounts(played.messages).replies).toBe(2);
      });
      const report = await run.finish();
      const records: DatasetRecord[] = [];
      for await (const record of run.records(products)) records.push(record);
      await foxhound.close();

      expect(report).toMatchObject({ lifecycle: "finished", verdict: "verified" });
      expect(records.map((record) => record.value)).toEqual([
        { id: "P-0001", name: "Blue kettle", price: "12.34" },
      ]);
      expect(played.done()).toEqual({ kind: "finished" });
      expect(protocolCounts(played.messages)).toMatchInlineSnapshot(`
        {
          "answers": 8,
          "events": 1,
          "replies": 2,
          "requests": [
            "hello",
            "crawl.start",
            "page.read",
            "crawl_request.enqueue",
            "page.read",
            "crawl_request.push",
            "run.finish",
            "run.records",
            "$/credit",
          ],
          "served": [
            "crawl_request.handle",
            "crawl_request.handle",
          ],
        }
      `);
    },
  );

  test(
    "starts two Runs from one Crawl value that share nothing: each handler call enqueues under its own Run",
    { tags: ["story:11-1013"] },
    async () => {
      const played = playScript("run/foxhound-crawl-two-runs.json");
      const foxhound = await connectOver(played.transport);
      const pages = new Crawl();
      const page = pages.label("page");
      pages.handle(page, (context) => context.enqueue("/next", page));

      await foxhound.startCrawl(pages, {
        seeds: [seed("https://shop.test/a", page)],
        runId: "run-a",
      });
      await foxhound.startCrawl(pages, {
        seeds: [seed("https://shop.test/b", page)],
        runId: "run-b",
      });
      await vi.waitFor(() => {
        expect(protocolCounts(played.messages).replies).toBe(2);
      });
      await foxhound.close();

      expect(played.done()).toEqual({ kind: "finished" });
    },
  );

  test(
    "stops a handler's pending call when its Run is closed, and answers its attempt with that error",
    { tags: ["story:11-1025", "error:closed"] },
    async () => {
      const played = playScript("run/foxhound-crawl-closed-mid-attempt.json");
      const foxhound = await connectOver(played.transport);
      const pages = new Crawl();
      const page = pages.label("page");
      let failure: unknown;
      pages.handle(page, async (context) => {
        try {
          await context.page.read({ title: { locator: { kind: "css", selector: "h1" } } });
        } catch (error) {
          failure = error;
          throw error;
        }
      });

      const run = await foxhound.startCrawl(pages, {
        seeds: [seed("https://shop.test/a", page)],
        runId: "nightly",
      });
      await vi.waitFor(() => {
        expect(protocolCounts(played.messages).requests).toContain("page.read");
      });
      await run.close();
      await foxhound.close();

      expect(failure).toBeInstanceOf(ClosedError);
      expect(played.done()).toEqual({ kind: "finished" });
    },
  );

  test("takes a handler whose expression body enqueues to its own Label", () => {
    const pages = new Crawl();
    const page = pages.label("page");

    pages.handle(page, (context) => context.enqueue("/next", page));

    expectTypeOf<Parameters<Parameters<typeof pages.handle>[1]>[0]>().toEqualTypeOf<CrawlContext>();
  });

  test("hands a handler the Page a Collie program holds", () => {
    expectTypeOf<CrawlContext["page"]>().toEqualTypeOf<Page>();
  });
});
