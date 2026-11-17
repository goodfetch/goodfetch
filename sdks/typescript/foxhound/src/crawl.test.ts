// A Crawl on the Scripted engine (ADR 0069): the program declares its
// Labels, handlers and Dataset on a `Crawl` value, starts it on a
// Connection, and serves the engine's `crawl_request.handle` requests on
// the event loop. The script fails the test on a message it does not
// expect and on a step left unused.
import { playScript, protocolCounts, test } from "@goodfetch/test-support";
import { describe, expect, vi } from "vitest";
import { crawl, listing, products } from "../tests/programs/two-page-types.ts";
import { ClosedError, Crawl, seed, type DatasetRecord, type EnqueuedLinks } from "./index.ts";
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
    "starts two Runs from one Crawl value that share nothing: each handler call enqueues and pushes under its own Run",
    { tags: ["story:11-1013"] },
    async () => {
      const played = playScript("run/foxhound-crawl-two-runs.json");
      const foxhound = await connectOver(played.transport);
      const pages = new Crawl();
      const page = pages.label("page");
      const seen = pages.dataset<{ readonly page: string }>("pages");
      pages.handle(page, async (context) => {
        await context.enqueue("/next", page);
        await context.push(seen, { page: "seen" });
      });

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
    "enqueues the links a locator matches with one Payload in one call, after a read and a push: four exchanges a page",
    { tags: ["operation:crawl_request.enqueue_links"] },
    async () => {
      const played = playScript("run/foxhound-crawl-enqueue-links.json");
      const foxhound = await connectOver(played.transport);
      const pages = new Crawl();
      const page = pages.label<{ readonly from: string } | undefined>("page");
      const titles = pages.dataset<{ readonly title: unknown }>("pages");
      let enqueued: EnqueuedLinks | undefined;
      pages.handle(page, async (context) => {
        const { values } = await context.page.read({
          title: { locator: { kind: "css", selector: "h1" } },
        });
        await context.push(titles, { title: values["title"] });
        enqueued = await context.enqueueLinks(
          context.page.locator("ul.pages a"),
          page,
          { from: "listing" },
          { exclude: ["*/logout"], limit: 2 },
        );
      });

      await foxhound.startCrawl(pages, {
        seeds: [seed("https://shop.test/a", page)],
        runId: "nightly",
      });
      await vi.waitFor(() => {
        expect(protocolCounts(played.messages).replies).toBe(1);
      });
      await foxhound.close();

      expect(enqueued?.links).toEqual([
        { kind: "added", id: "2", url: "https://shop.test/b" },
        { kind: "not_added", reason: "duplicate", url: "https://shop.test/a" },
        { kind: "no_link" },
      ]);
      expect(played.done()).toEqual({ kind: "finished" });
      expect(protocolCounts(played.messages)).toMatchInlineSnapshot(`
        {
          "answers": 6,
          "events": 0,
          "replies": 1,
          "requests": [
            "hello",
            "crawl.start",
            "page.read",
            "crawl_request.push",
            "crawl_request.enqueue_links",
            "run.abort",
          ],
          "served": [
            "crawl_request.handle",
          ],
        }
      `);
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

  test(
    "fails a records() read with closed once its Run is closed, sending nothing",
    { tags: ["story:11-1015", "error:closed"] },
    async () => {
      const played = playScript("run/foxhound-crawl-records-after-close.json");
      const foxhound = await connectOver(played.transport);

      const run = await foxhound.startCrawl(crawl, {
        seeds: [seed("https://shop.test/a", listing)],
        runId: "nightly",
      });
      await run.close();
      const read = run.records(products).next();

      await expect(read).rejects.toBeInstanceOf(ClosedError);
      await foxhound.close();
      expect(played.done()).toEqual({ kind: "finished" });
    },
  );

  test(
    "stops a records() read once its signal aborts, cancelling its stream and leaving the Records already received unread",
    { tags: ["story:11-1015"] },
    async () => {
      const played = playScript("run/foxhound-crawl-records-stopped.json");
      const foxhound = await connectOver(played.transport);
      const run = await foxhound.startCrawl(crawl, {
        seeds: [seed("https://shop.test/a", listing)],
        runId: "nightly",
      });
      const stop = new AbortController();
      const reading = run.records(products, { signal: stop.signal });

      const first = await reading.next();
      // Answered after both Records, so both are received once it returns.
      await run.finish();
      stop.abort(new Error("the program has read enough"));

      expect(first.value?.number).toBe(1);
      await expect(reading.next()).rejects.toThrow("the program has read enough");
      await foxhound.close();
      expect(played.done()).toEqual({ kind: "finished" });
    },
  );

  test(
    "keeps serving a Run started again by its Run ID when the first value of it is closed twice",
    { tags: ["story:11-1025"] },
    async () => {
      const played = playScript("run/foxhound-crawl-closed-then-started-again.json");
      const foxhound = await connectOver(played.transport);
      const pages = new Crawl();
      const page = pages.label("page");
      const options = { seeds: [seed("https://shop.test/a", page)] as const, runId: "nightly" };
      const first = await foxhound.startCrawl(pages, options);
      // The first handler call closes the first Run value again before the
      // engine sends the second.
      pages.handle(page, async (context) => {
        await first.close();
        return context.enqueue("/next", page);
      });

      await first.close();
      await foxhound.startCrawl(pages, options);
      await vi.waitFor(() => {
        expect(protocolCounts(played.messages).replies).toBe(2);
      });
      await foxhound.close();

      expect(played.done()).toEqual({ kind: "finished" });
    },
  );

  test("takes a handler whose expression body enqueues to its own Label", () => {
    const pages = new Crawl();
    const page = pages.label("page");

    // Compiles only while a handler may return its context's promise.
    pages.handle(page, (context) => context.enqueue("/next", page));
  });
});
