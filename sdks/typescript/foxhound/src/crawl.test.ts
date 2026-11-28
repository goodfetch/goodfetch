// A Crawl on the Scripted engine (ADR 0069): the program declares its
// Labels, handlers and Dataset on a `Crawl` value, starts it on a
// Connection, and serves the engine's `crawl_request.handle` requests on
// the event loop. The script fails the test on a message it does not
// expect and on a step left unused.
import { playScript, protocolCounts, test } from "@goodfetch/test-support";
import { describe, expect, vi } from "vitest";
import { z } from "zod";
import { crawl, listing, products } from "../tests/programs/two-page-types.ts";
import {
  ClosedError,
  Crawl,
  LabelDuplicateError,
  LabelUnknownError,
  RecordInvalidError,
  seed,
  sitemap,
  type CrawlRun,
  type DatasetRecord,
  type EnqueuedLinks,
} from "./index.ts";
import { crawlDeclaration } from "./crawl.ts";
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
      const records: DatasetRecord<unknown>[] = [];
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
    "enqueues the links a locator matches with one Payload in one call, after a read and a push: four exchanges a page, the Label's Zod schema sent as JSON Schema",
    { tags: ["operation:crawl_request.enqueue_links", "story:11-1018"] },
    async () => {
      const played = playScript("run/foxhound-crawl-enqueue-links.json");
      const foxhound = await connectOver(played.transport);
      const pages = new Crawl();
      const page = pages.label("page", { payload: z.object({ from: z.string() }) });
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
        seeds: [seed("https://shop.test/a", page, { from: "start" })],
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
    "answers a handler's own exception with handler_failed, naming its type and message and carrying its stack",
    { tags: ["error:handler_failed"] },
    async () => {
      const played = playScript("run/foxhound-crawl-handler-threw.json");
      const foxhound = await connectOver(played.transport);
      const pages = new Crawl();
      const page = pages.label("page");
      pages.handle(page, () => {
        const thrown = new TypeError("the price is missing");
        thrown.stack = "TypeError: the price is missing\n    at handler (shop.ts:3:11)";
        throw thrown;
      });

      await foxhound.startCrawl(pages, {
        seeds: [seed("https://shop.test/a", page)],
        runId: "nightly",
      });
      await vi.waitFor(() => {
        expect(protocolCounts(played.messages).replies).toBe(1);
      });
      await foxhound.close();

      expect(played.done()).toEqual({ kind: "finished" });
    },
  );

  test(
    "throws closed from records() once its Run is closed, sending nothing",
    { tags: ["story:11-1015", "error:closed"] },
    async () => {
      const played = playScript("run/foxhound-crawl-records-after-close.json");
      const foxhound = await connectOver(played.transport);

      const run = await foxhound.startCrawl(crawl, {
        seeds: [seed("https://shop.test/a", listing)],
        runId: "nightly",
      });
      await run.close();
      expect(() => run.records(products)).toThrow(ClosedError);
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
    "reads Records past `after`, from the Record numbered after it",
    { tags: ["story:11-1060"] },
    async () => {
      const played = playScript("run/foxhound-crawl-records-parsed.json");
      const foxhound = await connectOver(played.transport);
      const run = await foxhound.startCrawl(crawl, {
        seeds: [seed("https://shop.test/a", listing)],
        runId: "nightly",
      });
      const reading = run.records(products, { after: 1 });

      const first = await reading.next();
      await reading.return();
      await foxhound.close();

      expect(first.value?.number).toBe(2);
      expect(played.done()).toEqual({ kind: "finished" });
    },
  );

  test(
    "reads each Record as its Dataset's own type, a top-level field its schema does not declare kept and one it renames left renamed",
    { tags: ["story:11-1070"] },
    async () => {
      const played = playScript("run/foxhound-crawl-records-parsed.json");
      const foxhound = await connectOver(played.transport);
      const run = await foxhound.startCrawl(crawl, {
        seeds: [seed("https://shop.test/a", listing)],
        runId: "nightly",
      });
      // A reader's own declaration of the Dataset, whose schema parses each
      // price and reads `name` as `title`.
      const typed = new Crawl().dataset("products", {
        schema: z
          .object({ id: z.string(), name: z.string(), price: z.string().transform(Number) })
          .transform(({ name, ...rest }) => ({ ...rest, title: name })),
        key: ["id"],
      });
      const reading = run.records(typed, { after: 1 });

      const first = await reading.next();
      await reading.return();
      await foxhound.close();

      expect(first.value?.value).toEqual({
        id: "P-0002",
        title: "Red kettle",
        price: 23.45,
        colour: "red",
      });
      expect(played.done()).toEqual({ kind: "finished" });
    },
  );

  test(
    "ends a typed read with record_invalid naming the number of the first Record its schema refuses",
    { tags: ["story:11-1070", "error:record_invalid"] },
    async () => {
      const played = playScript("run/foxhound-crawl-records-parsed.json");
      const foxhound = await connectOver(played.transport);
      const run = await foxhound.startCrawl(crawl, {
        seeds: [seed("https://shop.test/a", listing)],
        runId: "nightly",
      });
      const typed = new Crawl().dataset("products", {
        schema: z.object({ id: z.string(), name: z.string(), price: z.string() }),
        key: ["id"],
      });
      const reading = run.records(typed, { after: 1 });

      await reading.next();
      const refused = await reading.next().then(
        () => undefined,
        (error: unknown) => error,
      );
      await foxhound.close();

      expect(refused).toBeInstanceOf(RecordInvalidError);
      expect(refused).toMatchObject({
        typedDetails: {
          dataset: "products",
          number: 3,
          records: [{ position: 0, issues: [{ path: "/price", message: expect.any(String) }] }],
        },
      });
      expect(await reading.next()).toEqual({ done: true, value: undefined });
      expect(played.done()).toEqual({ kind: "finished" });
    },
  );

  test(
    "reads a Dataset's rejected Records page after page past `after`, each as JSON with its reason, Hook, request ID and URL",
    { tags: ["story:11-1061"] },
    async () => {
      const played = playScript("run/foxhound-crawl-rejected-records.json");
      const foxhound = await connectOver(played.transport);
      const run = await foxhound.startCrawl(crawl, {
        seeds: [seed("https://shop.test/a", listing)],
        runId: "nightly",
      });

      const rejected = [];
      for await (const entry of run.rejectedRecords(products, { after: 1 })) rejected.push(entry);
      await foxhound.close();

      expect(rejected).toEqual([
        {
          number: 2,
          data: { id: "P-0002", price: 23.45 },
          reason: 'the Dataset products refuses the Record: /price: 23.45 is not of type "string"',
          issues: [{ path: "/price", message: '23.45 is not of type "string"' }],
          requestId: "2",
          url: "https://shop.test/products/P-0002",
        },
        {
          number: 3,
          data: { id: "P-0003", name: "Sample kettle", price: "0.00" },
          reason: "a free sample",
          issues: [],
          hook: "no-samples",
          requestId: "3",
          url: "https://shop.test/products/P-0003",
        },
      ]);
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
      let first: CrawlRun | undefined;
      // The first handler call closes the first Run value again before the
      // engine sends the second; the engine leases nothing before the Run
      // is started again.
      pages.handle(page, async (context) => {
        await first?.close();
        return context.enqueue("/next", page);
      });
      first = await foxhound.startCrawl(pages, options);

      await first.close();
      await foxhound.startCrawl(pages, options);
      await vi.waitFor(() => {
        expect(protocolCounts(played.messages).replies).toBe(2);
      });
      await foxhound.close();

      expect(played.done()).toEqual({ kind: "finished" });
    },
  );

  test(
    "pushes a list of fifty Records with one request",
    { tags: ["operation:crawl_request.push"] },
    async () => {
      const played = playScript("run/foxhound-crawl-push-list.json");
      const foxhound = await connectOver(played.transport);
      const pages = new Crawl();
      const page = pages.label("page");
      const ids = pages.dataset<{ readonly id: string }>("products", { key: ["id"] });
      const fifty = Array.from({ length: 50 }, (_, index) => ({
        id: `P-${String(index + 1).padStart(4, "0")}`,
      }));
      pages.handle(page, async (context) => {
        await context.push(ids, fifty);
      });

      await foxhound.startCrawl(pages, {
        seeds: [seed("https://shop.test/a", page)],
        runId: "nightly",
      });
      await vi.waitFor(() => {
        expect(protocolCounts(played.messages).replies).toBe(1);
      });
      await foxhound.close();

      expect(played.done()).toEqual({ kind: "finished" });
      expect(protocolCounts(played.messages)).toMatchInlineSnapshot(`
        {
          "answers": 4,
          "events": 0,
          "replies": 1,
          "requests": [
            "hello",
            "crawl.start",
            "crawl_request.push",
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
    "keeps the Records a handler pushed when it catches a list's record_invalid, naming the failing Record by its position",
    { tags: ["story:6-267", "story:11-35", "error:record_invalid"] },
    async () => {
      const played = playScript("run/foxhound-crawl-push-list-record-invalid.json");
      const foxhound = await connectOver(played.transport);
      const pages = new Crawl();
      const page = pages.label("page");
      const kettles = pages.dataset<{ readonly id?: string; readonly name: string }>("products", {
        key: ["id"],
      });
      let failure: unknown;
      pages.handle(page, async (context) => {
        await context.push(kettles, { id: "P-0001", name: "Blue kettle" });
        try {
          await context.push(kettles, [
            { id: "P-0002", name: "Red kettle" },
            { name: "Kettle lid" },
            { id: "P-0003", name: "Green kettle" },
          ]);
        } catch (error) {
          failure = error;
        }
      });

      const run = await foxhound.startCrawl(pages, {
        seeds: [seed("https://shop.test/a", page)],
        runId: "nightly",
      });
      await vi.waitFor(() => {
        expect(protocolCounts(played.messages).replies).toBe(1);
      });
      await run.finish();
      const records: DatasetRecord<unknown>[] = [];
      for await (const record of run.records(kettles)) records.push(record);
      await foxhound.close();

      expect(failure).toBeInstanceOf(RecordInvalidError);
      expect(failure).toMatchObject({
        code: "record_invalid",
        details: {
          dataset: "products",
          records: [{ position: 1, issues: [{ path: "/id" }] }],
        },
      });
      expect(records.map((record) => record.value)).toEqual([
        { id: "P-0001", name: "Blue kettle" },
        { id: "P-0002", name: "Red kettle" },
        { id: "P-0003", name: "Green kettle" },
      ]);
      expect(played.done()).toEqual({ kind: "finished" });
    },
  );

  test(
    "sends nothing for an empty list, which pushes no Record",
    { tags: ["operation:crawl_request.push"] },
    async () => {
      const played = playScript("run/foxhound-crawl-push-empty-list.json");
      const foxhound = await connectOver(played.transport);
      const pages = new Crawl();
      const page = pages.label("page");
      const ids = pages.dataset<{ readonly id: string }>("products", { key: ["id"] });
      pages.handle(page, async (context) => {
        await context.push(ids, []);
      });

      await foxhound.startCrawl(pages, {
        seeds: [seed("https://shop.test/a", page)],
        runId: "nightly",
      });
      await vi.waitFor(() => {
        expect(protocolCounts(played.messages).replies).toBe(1);
      });
      await foxhound.close();

      expect(played.done()).toEqual({ kind: "finished" });
    },
  );

  test("types each Record of a pushed list from its Dataset alone", () => {
    const pages = new Crawl();
    const kettles = pages.dataset<{ readonly id: string }>("products");

    // Compiles only while `push` takes a list of the Dataset's Records,
    // and refuses one whose Record is not of the Dataset's type.
    pages.handle(pages.label("page"), async (context) => {
      await context.push(kettles, [{ id: "P-0001" }, { id: "P-0002" }]);
      // @ts-expect-error -- the second Record lacks the Dataset's `id`.
      await context.push(kettles, [{ id: "P-0001" }, { name: "Kettle lid" }]);
      // @ts-expect-error -- a Record's type is the Dataset's, never inferred from the Record.
      await context.push(kettles, { id: 1 });
    });
  });

  test("takes a handler whose expression body enqueues to its own Label", () => {
    const pages = new Crawl();
    const page = pages.label("page");

    // Compiles only while a handler may return its context's promise.
    pages.handle(page, (context) => context.enqueue("/next", page));
  });
});

// Wiring mistakes fail at once (ADRs 0093 and 0104): a Label declared or
// handled twice at declaration, a Label left without a handler when the
// Run starts, and a Label the crawl does not declare where it is named,
// each before anything is sent.
describe("a Crawl's wiring mistakes", () => {
  test(
    "refuses a second Label of one name and a second handler for one Label with label_duplicate, with no connection",
    { tags: ["error:label_duplicate"] },
    () => {
      const pages = new Crawl();
      const page = pages.label("page");
      pages.handle(page, () => undefined);

      expect(() => pages.label("page")).toThrow(
        expect.objectContaining({ code: "label_duplicate", details: { label: "page" } }),
      );
      expect(() => {
        pages.handle(page, () => undefined);
      }).toThrow(LabelDuplicateError);
    },
  );

  test(
    "refuses a handler for another crawl's Label of the same name with label_unknown",
    { tags: ["error:label_unknown"] },
    () => {
      const pages = new Crawl();
      pages.label("page");
      const otherPage = new Crawl().label("page");

      expect(() => {
        pages.handle(otherPage, () => undefined);
      }).toThrow(
        expect.objectContaining({
          code: "label_unknown",
          details: { label: "page", declared: ["page"] },
        }),
      );
    },
  );

  test(
    "refuses a second Dataset of one name with dataset_duplicate, and a key on a field its schema does not require with dataset_key_invalid, as they are declared",
    { tags: ["story:11-1057", "error:dataset_duplicate", "error:dataset_key_invalid"] },
    () => {
      const shelf = new Crawl();
      const edition = z.object({ isbn: z.string(), title: z.string().optional() });
      shelf.dataset("books");

      expect(() => shelf.dataset("books")).toThrow(
        expect.objectContaining({ code: "dataset_duplicate", details: { dataset: "books" } }),
      );
      expect(() => shelf.dataset("editions", { schema: edition, key: ["title"] })).toThrow(
        expect.objectContaining({
          code: "dataset_key_invalid",
          details: { dataset: "editions", reason: "not_required", field: "title" },
        }),
      );
      const editions = shelf.dataset("editions", { schema: edition, key: ["isbn"] });
      // Without a schema, a Dataset takes any JSON object, so its key is checked on each Record.
      const notes = shelf.dataset("notes", { key: ["page"] });

      expect([editions.name, notes.name]).toEqual(["editions", "notes"]);
      expect(crawlDeclaration(shelf).datasets).toEqual([
        { name: "books" },
        {
          name: "editions",
          schema: expect.objectContaining({ required: ["isbn"] }),
          key: ["isbn"],
        },
        { name: "notes", key: ["page"] },
      ]);
    },
  );

  test("types a Dataset's Records from its schema, refusing a missing field and a key on no field", () => {
    const shelf = new Crawl();
    const editions = shelf.dataset("editions", {
      schema: z.object({ isbn: z.string(), pages: z.number() }),
      key: ["isbn"],
    });
    const shelves = z.object({ isbn: z.string() });
    expect(() =>
      // @ts-expect-error -- `shelf` is no field of an edition, so it keys no Dataset of them.
      shelf.dataset("shelves", { schema: shelves, key: ["shelf"] }),
    ).toThrow(expect.objectContaining({ code: "dataset_key_invalid" }));

    // Compiles only while a Dataset's Records take its schema's input type.
    shelf.handle(shelf.label("page"), async (context) => {
      await context.push(editions, { isbn: "978-0", pages: 12 });
      // @ts-expect-error -- the Record lacks the schema's `pages`.
      await context.push(editions, { isbn: "978-0" });
      // @ts-expect-error -- no Dataset handle of that name exists: a misspelled one fails to compile.
      await context.push(editoins, { isbn: "978-0", pages: 12 });
    });
  });

  test(
    "refuses to start a crawl with a Label left without a handler with label_unhandled, sending nothing",
    { tags: ["error:label_unhandled"] },
    async () => {
      const played = playScript("connect/foxhound-accepted.json");
      const foxhound = await connectOver(played.transport);
      const pages = new Crawl();
      const page = pages.label("page");
      pages.label("detail");
      pages.handle(page, () => undefined);

      const starting = foxhound.startCrawl(pages, {
        seeds: [seed("https://shop.test/a", page)],
      });

      await expect(starting).rejects.toThrow(
        expect.objectContaining({ code: "label_unhandled", details: { label: "detail" } }),
      );
      await foxhound.close();
      expect(protocolCounts(played.messages).requests).toEqual(["hello"]);
      expect(played.done()).toEqual({ kind: "finished" });
    },
  );

  test(
    "refuses to start a crawl from a seed of a Label it does not declare with label_unknown, sending nothing",
    { tags: ["error:label_unknown"] },
    async () => {
      const played = playScript("connect/foxhound-accepted.json");
      const foxhound = await connectOver(played.transport);
      const pages = new Crawl();
      pages.handle(pages.label("page"), () => undefined);
      const otherDetail = new Crawl().label("detail");

      const starting = foxhound.startCrawl(pages, {
        seeds: [seed("https://shop.test/a", otherDetail)],
      });

      await expect(starting).rejects.toThrow(
        expect.objectContaining({
          code: "label_unknown",
          details: { label: "detail", declared: ["page"] },
        }),
      );
      await foxhound.close();
      expect(protocolCounts(played.messages).requests).toEqual(["hello"]);
      expect(played.done()).toEqual({ kind: "finished" });
    },
  );

  test(
    "refuses to start a crawl from a sitemap rule of another crawl's Label with label_unknown, sending nothing",
    { tags: ["story:11-1019", "error:label_unknown"] },
    async () => {
      const played = playScript("connect/foxhound-accepted.json");
      const foxhound = await connectOver(played.transport);
      const pages = new Crawl();
      const page = pages.label("page");
      pages.handle(page, () => undefined);
      const otherDetail = new Crawl().label("detail");

      const starting = foxhound.startCrawl(pages, {
        seeds: [
          sitemap("https://shop.test/sitemap.xml", [
            ["**", page],
            ["**/d/*", otherDetail],
          ]),
        ],
      });

      await expect(starting).rejects.toThrow(
        expect.objectContaining({
          code: "label_unknown",
          details: { label: "detail", declared: ["page"] },
        }),
      );
      await foxhound.close();
      expect(protocolCounts(played.messages).requests).toEqual(["hello"]);
      expect(played.done()).toEqual({ kind: "finished" });
    },
  );

  test(
    "refuses enqueue and enqueueLinks to another crawl's Label with label_unknown, sending nothing",
    {
      tags: [
        "error:label_unknown",
        "operation:crawl_request.enqueue",
        "operation:crawl_request.enqueue_links",
      ],
    },
    async () => {
      const played = playScript("run/foxhound-crawl-push-empty-list.json");
      const foxhound = await connectOver(played.transport);
      const pages = new Crawl();
      const page = pages.label("page");
      const ids = pages.dataset<{ readonly id: string }>("products", { key: ["id"] });
      const otherPage = new Crawl().label("page");
      const refused: unknown[] = [];
      pages.handle(page, async (context) => {
        await context.enqueue("/next", otherPage).catch((error: unknown) => refused.push(error));
        await context
          .enqueueLinks(context.page.locator("a"), otherPage)
          .catch((error: unknown) => refused.push(error));
        await context.push(ids, []);
      });

      await foxhound.startCrawl(pages, {
        seeds: [seed("https://shop.test/a", page)],
        runId: "nightly",
      });
      await vi.waitFor(() => {
        expect(protocolCounts(played.messages).replies).toBe(1);
      });
      await foxhound.close();

      expect(refused).toEqual([expect.any(LabelUnknownError), expect.any(LabelUnknownError)]);
      expect(refused[0]).toMatchObject({ details: { label: "page", declared: ["page"] } });
      expect(played.done()).toEqual({ kind: "finished" });
    },
  );

  test(
    "addSeeds of 100 seeds streams under credits (protocol-count snapshot) and keeps a 1,000,000-URL generator bounded in flight with flat memory",
    {
      tags: ["operation:run.add_seeds"],
      timeout: 60_000,
    },
    async () => {
      // 1. Protocol-count snapshot for `addSeeds` of 100 seeds.
      const small = seedEngine(true);
      const foxhound100 = await connectOver(small.transport);
      const smallCrawl = new Crawl();
      const page100 = smallCrawl.label("page");
      smallCrawl.handle(page100, async () => undefined);
      const run100 = await foxhound100.startCrawl(smallCrawl, {
        seeds: [seed("https://shop.test/0", page100)],
      });
      async function* gen100() {
        for (let i = 1; i <= 100; i += 1) {
          yield seed(`https://shop.test/${String(i)}`, page100);
        }
      }
      const answers100: string[] = [];
      for await (const ans of run100.addSeeds(gen100())) {
        if (ans.kind === "added") answers100.push(ans.url);
      }
      const counts = protocolCounts(small.messages);
      await foxhound100.close();
      expect(answers100).toHaveLength(100);
      expect(answers100[0]).toBe("https://shop.test/1");
      expect(answers100[99]).toBe("https://shop.test/100");
      expect(small.getMaxInFlight()).toBeLessThanOrEqual(256);
      expect(counts).toMatchInlineSnapshot(`
        {
          "answers": 3,
          "events": 5,
          "replies": 0,
          "requests": [
            "hello",
            "crawl.start",
            "run.add_seeds",
            "$/seeds",
            "$/seeds",
          ],
          "served": [],
        }
      `);

      // 2. 1,000,000-URL generator stays bounded in flight (<= 256) with flat memory.
      const large = seedEngine(false);
      const foxhoundMillion = await connectOver(large.transport);
      const largeCrawl = new Crawl();
      const pageMillion = largeCrawl.label("page");
      largeCrawl.handle(pageMillion, async () => undefined);
      const runMillion = await foxhoundMillion.startCrawl(largeCrawl, {
        seeds: [seed("https://shop.test/0", pageMillion)],
      });
      function* genMillion() {
        for (let i = 1; i <= 1_000_000; i += 1) {
          yield seed(`https://shop.test/${String(i)}`, pageMillion);
        }
      }
      let count = 0;
      let heapAfterWarmup = 0;
      let peakHeapDelta = 0;
      let firstAnswer: unknown;
      let lastAnswer: unknown;
      for await (const ans of runMillion.addSeeds(genMillion())) {
        count += 1;
        if (count === 1) firstAnswer = ans;
        if (count === 1_000_000) lastAnswer = ans;
        if (count === 50_000) {
          heapAfterWarmup = process.memoryUsage().heapUsed;
        } else if (count > 50_000 && count % 100_000 === 0) {
          const delta = process.memoryUsage().heapUsed - heapAfterWarmup;
          if (delta > peakHeapDelta) peakHeapDelta = delta;
        }
      }
      await foxhoundMillion.close();
      expect(count).toBe(1_000_000);
      expect(firstAnswer).toMatchObject({ kind: "added", url: "https://shop.test/1" });
      expect(lastAnswer).toMatchObject({ kind: "added", url: "https://shop.test/1000000" });
      expect(large.getMaxInFlight()).toBeLessThanOrEqual(256);
      // 1M seeds buffered in memory would require >200 MB; bounded streaming stays flat (< 64 MB delta).
      expect(peakHeapDelta).toBeLessThan(64 * 1024 * 1024);
    },
  );

  test(
    "export and saveAs protocol-count snapshot (ADR 0071)",
    { tags: ["operation:run.export", "operation:export.save_as"] },
    async () => {
      const engine = seedEngine(true);
      const foxhound = await connectOver(engine.transport);
      const c = new Crawl();
      const page = c.label("page");
      const items = c.dataset("items");
      c.handle(page, async () => undefined);
      const run = await foxhound.startCrawl(c, {
        seeds: [seed("https://shop.test/0", page)],
      });
      const exported = await run.export(items, { format: "jsonl" });
      const saved = await exported.saveAs("/tmp/export-snapshot.jsonl");
      const counts = protocolCounts(engine.messages);
      await foxhound.close();

      expect(exported.id).toBe("exp-1");
      expect(saved.size).toBe(12);
      expect(counts).toMatchInlineSnapshot(`
        {
          "answers": 4,
          "events": 0,
          "replies": 0,
          "requests": [
            "hello",
            "crawl.start",
            "run.export",
            "export.save_as",
            "$/credit",
          ],
          "served": [],
        }
      `);
    },
  );

  test(
    "outcomes() open and retryFailed protocol-count snapshots (ADR 0071)",
    { tags: ["operation:run.outcomes", "operation:run.retry_failed"] },
    async () => {
      const engine = seedEngine(true);
      const foxhound = await connectOver(engine.transport);
      const c = new Crawl();
      const page = c.label("page");
      c.handle(page, async () => undefined);
      const run = await foxhound.startCrawl(c, {
        seeds: [seed("https://shop.test/0", page)],
      });
      const seen: string[] = [];
      for await (const outcome of run.outcomes()) {
        seen.push(outcome.kind);
      }
      const retried = await run.retryFailed();
      const counts = protocolCounts(engine.messages);
      await foxhound.close();

      expect(seen).toEqual(["failed"]);
      expect(retried).toBe(1);
      expect(counts).toMatchInlineSnapshot(`
        {
          "answers": 4,
          "events": 0,
          "replies": 0,
          "requests": [
            "hello",
            "crawl.start",
            "run.outcomes",
            "run.retry_failed",
          ],
          "served": [],
        }
      `);
    },
  );
});

/** Type guard for a JSON object parsed from a wire line. */
function isObjectRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

/**
 * Minimal in-memory engine speaking `hello`, `crawl.start`, and `run.add_seeds`
 * (`$/credit` -> `$/seeds` -> `$/enqueued`) so we can snapshot exact protocol counts
 * for 100 seeds AND verify bounded in-flight & flat memory over 1,000,000 seeds.
 */
function seedEngine(recordMessages: boolean) {
  const encoder = new TextEncoder();
  const decoder = new TextDecoder();
  const messages: {
    readonly from: "client" | "engine";
    readonly message: Record<string, unknown>;
  }[] = [];
  let maxInFlight = 0;
  let currentCredits = 0;
  let addSeedsId: number | undefined;
  let totalAdded = 0;
  let buffered = "";
  let engineController: ReadableStreamDefaultController<Uint8Array> | undefined;

  const emit = (message: Record<string, unknown>): void => {
    if (recordMessages) messages.push({ from: "engine", message });
    engineController?.enqueue(encoder.encode(`${JSON.stringify(message)}\n`));
  };

  const readable = new ReadableStream<Uint8Array>({
    start(controller) {
      engineController = controller;
    },
  });

  const handleLine = (line: string): void => {
    if (line.trim() === "") return;
    const parsed: unknown = JSON.parse(line);
    if (!isObjectRecord(parsed)) return;
    if (recordMessages) messages.push({ from: "client", message: parsed });
    const method = parsed["method"];
    const id = parsed["id"];
    if (method === "hello") {
      emit({
        jsonrpc: "2.0",
        id,
        result: {
          protocol_version: "0.1",
          product: "foxhound",
          product_version: "0.1.0",
          engine_instance: "0123456789abcdef",
        },
      });
      return;
    }
    if (method === "crawl.start") {
      emit({
        jsonrpc: "2.0",
        id,
        result: {
          run_id: "stream-bench",
          lifecycle: "running",
          started_as: "new",
          needs: "foxhound",
          seeds: [{ kind: "added", crawl_request_id: "r-0", url: "https://shop.test/0" }],
        },
      });
      return;
    }
    if (method === "run.add_seeds" && typeof id === "number") {
      addSeedsId = id;
      currentCredits = 256;
      totalAdded = 0;
      emit({
        jsonrpc: "2.0",
        method: "$/credit",
        params: { id: addSeedsId, credits: 256 },
      });
      return;
    }
    if (method === "$/seeds") {
      const rawParams = parsed["params"];
      if (!isObjectRecord(rawParams)) return;
      const paramId = rawParams["id"];
      const rawSeeds = rawParams["seeds"];
      if (typeof paramId !== "number" || !Array.isArray(rawSeeds)) return;
      const batchLen = rawSeeds.length;
      currentCredits -= batchLen;
      const inFlight = 256 - currentCredits;
      if (inFlight > maxInFlight) maxInFlight = inFlight;
      if (batchLen > 0) {
        const answers = rawSeeds.map((entry) => {
          totalAdded += 1;
          const url = isObjectRecord(entry) && typeof entry["url"] === "string" ? entry["url"] : "";
          return {
            kind: "added",
            crawl_request_id: `r-${String(totalAdded)}`,
            url,
          };
        });
        emit({
          jsonrpc: "2.0",
          method: "$/enqueued",
          params: { id: paramId, answers },
        });
        currentCredits += batchLen;
        emit({
          jsonrpc: "2.0",
          method: "$/credit",
          params: { id: paramId, credits: batchLen },
        });
      }
      if (rawParams["end"] === true) {
        emit({
          jsonrpc: "2.0",
          id: paramId,
          result: { added: totalAdded, not_added: 0 },
        });
      }
      return;
    }
    if (method === "run.export") {
      emit({
        jsonrpc: "2.0",
        id,
        result: {
          export_id: "exp-1",
          format: "jsonl",
          compression: "none",
          records: 1,
          last_number: 1,
          size: 12,
          sha256: "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
          run_epoch: 1,
        },
      });
      return;
    }
    if (method === "export.save_as") {
      emit({
        jsonrpc: "2.0",
        id,
        result: {
          size: 12,
          sha256: "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
          run_epoch: 1,
          written: true,
        },
      });
      return;
    }
    if (method === "run.outcomes") {
      emit({
        jsonrpc: "2.0",
        id,
        result: {
          outcomes: [
            {
              position: 1,
              kind: "failed",
              request: {
                id: "r-0",
                url: "https://shop.test/0",
                label: "page",
                method: "GET",
                depth: 0,
              },
              attempts: 1,
              errors: [
                {
                  attempt: 1,
                  code: "handler_failed",
                  message: "boom",
                  retryable: true,
                },
              ],
            },
          ],
          next: 1,
          ended: true,
        },
      });
      return;
    }
    if (method === "run.retry_failed") {
      emit({
        jsonrpc: "2.0",
        id,
        result: { readmitted: 1 },
      });
      return;
    }
    if (method === "run.abort") {
      emit({
        jsonrpc: "2.0",
        id,
        result: {
          run_id: "stream-bench",
          lifecycle: "stopped",
          stop_reason: { kind: "abort", by: "client" },
          ended_at: "2026-11-27T00:00:00Z",
          started_at: "2026-11-27T00:00:00Z",
          needs: "foxhound",
          completeness: "incomplete",
        },
      });
      return;
    }
    if (method === "shutdown" || method === "close") {
      emit({ jsonrpc: "2.0", id, result: {} });
    }
  };

  const writable = new WritableStream<Uint8Array>({
    write(chunk) {
      buffered += decoder.decode(chunk, { stream: true });
      let newline = buffered.indexOf("\n");
      while (newline !== -1) {
        const line = buffered.slice(0, newline);
        buffered = buffered.slice(newline + 1);
        handleLine(line);
        newline = buffered.indexOf("\n");
      }
    },
    close() {
      try {
        engineController?.close();
      } catch {
        // Already closed.
      }
    },
  });

  return {
    transport: {
      readable,
      writable,
      async close() {
        try {
          engineController?.close();
        } catch {
          // Already closed.
        }
      },
    },
    messages,
    getMaxInFlight: () => maxInFlight,
  };
}
