// Snippet for Testing a user's crawl or automation (#482, ADR 0105), on golden scenario 10: the crawl of
// scenario 05 and the AI extraction of scenario 08, tested with no network and no model.
import { z } from "zod";
import { Crawl, ModelFundsExhaustedError, ResponseNotRecordedError, seed } from "@goodfetch/foxhound";
import { TestEngine, fakeError, scriptedModel } from "@goodfetch/foxhound/testing";

// Any runner: Vitest, Jest, node:test, Mocha. The kit needs none of them, and none of their plugins.
declare function test(name: string, body: () => Promise<void>): void;
declare const assert: {
  equal<T>(actual: T, expected: T): void;
  deepEqual<T>(actual: T, expected: T): void;
  ok(value: unknown): asserts value;
};

// The crawl under test, as scenario 05 declares it (a module of the program).
const Book = z.object({
  title: z.string(),
  price: z.string().regex(/^\d+\.\d{2}$/),
  inStock: z.number().int(),
  url: z.url(),
  category: z.string(),
});
const crawl = new Crawl();
const books = crawl.dataset("books", { schema: Book, key: ["url"] });
const home = crawl.label("home");
const category = crawl.label("category", { payload: z.object({ name: z.string().min(1) }) });
const book = crawl.label("book", { payload: z.object({ category: z.string().min(1) }) });
crawl.handle(home, async ({ page, enqueue }) => {
  for (const link of await page.locator(".side_categories ul ul a").all()) {
    const href = await link.getAttribute("href");
    if (href !== null) await enqueue(new URL(href, page.url()), category, { name: (await link.textContent()).trim() });
  }
});
crawl.handle(category, async ({ page, request, enqueueLinks }) => {
  await enqueueLinks(page.locator("article.product_pod h3 a"), book, { category: request.payload.name });
  await enqueueLinks(page.locator("li.next a"), request.label, { name: request.payload.name });
});
crawl.handle(book, async ({ page, request, push, skip }) => {
  const price = /£(\d+\.\d{2})/.exec(await page.locator(".product_main .price_color").textContent())?.[1];
  if (price === undefined) return skip("no price");
  const stock = /(\d+) available/.exec(await page.locator(".product_main .availability").textContent());
  await push(books, {
    title: await page.locator(".product_main h1").textContent(),
    price,
    inStock: Number(stock?.[1] ?? 0),
    url: page.url(),
    category: request.payload.category,
  });
});

const BOOK = "https://books.toscrape.com/catalogue/a-light-in-the-attic_1000/index.html";

// 1. One handler, one saved page: no network, no browser, no Frontier. A page saved from a browser
//    or with curl is the document, given as `route.fulfill` gives a response.
test("a book page gives its book, with the category passed to it", async () => {
  await using engine = await TestEngine.start();
  const attempt = await engine.runHandler(crawl, book, {
    url: BOOK,
    payload: { category: "Poetry" },
    response: { path: "tests/fixtures/a-light-in-the-attic.html" },
  });
  assert.deepEqual(attempt.ended, { kind: "returned" });
  assert.deepEqual(attempt.records(books), [
    { title: "A Light in the Attic", price: "51.77", inStock: 22, url: BOOK, category: "Poetry" },
  ]);
  assert.equal(attempt.enqueued.length, 0);
});

// A missing price is a skip, reported as a value: what the attempt staged is there too.
test("a book page with no price is skipped", async () => {
  await using engine = await TestEngine.start();
  const attempt = await engine.runHandler(crawl, book, {
    url: BOOK,
    payload: { category: "Poetry" },
    response: { body: "<html><h1>No price</h1></html>" },
  });
  assert.deepEqual(attempt.ended, { kind: "skipped", reason: "handler", message: "no price" });
});

// 2. The whole crawl, from the responses one live run recorded. That run was started once with
//    `recording: { mode: "record", path: "tests/books.warc.gz" }`, and the file is committed.
test("the crawl collects every book from the recording", async () => {
  await using engine = await TestEngine.start();
  await using run = await engine.connection.startCrawl(crawl, {
    seeds: [seed("https://books.toscrape.com/", home)],
    recording: { mode: "replay", path: "tests/books.warc.gz" },
  });
  const report = await run.finish(); // no Politeness waits: no request reaches the site
  assert.equal(report.requests.failed, 0);
  let n = 0;
  for await (const _ of run.records(books)) n++;
  assert.equal(n, 1000);
  // The recording's age is in the report, never a reason to fail: a test depends only on its inputs.
  assert.ok(report.recording !== null && report.recording.misses.length === 0);
});

// Any request the recording lacks is never sent: the error names it, and the nearest recorded URLs.
test("a page the recording lacks fails, naming its address", async () => {
  await using engine = await TestEngine.start();
  await using run = await engine.connection.start({ recording: { mode: "replay", path: "tests/books.warc.gz" } });
  const page = await run.newPage();
  try {
    await page.goto("https://books.toscrape.com/catalogue/page-99.html");
    assert.ok(false);
  } catch (error) {
    assert.ok(error instanceof ResponseNotRecordedError);
    assert.equal(error.details.url, "https://books.toscrape.com/catalogue/page-99.html");
  }
});

// 3. Scenario 08's AI extraction, learned once on the live run: its Plan file is committed beside the
//    recording. Frozen, it never calls a model; and the test engine has none to call.
const aiCrawl = new Crawl();
const aiBooks = aiCrawl.dataset("books", { schema: Book.omit({ inStock: true, category: true }) });
const aiBook = aiCrawl.label("book");
aiCrawl.handle(aiBook, async ({ page, push }) => {
  const { output } = await page.extract("the book's title and price in pounds", Book.pick({ title: true, price: true }));
  await push(aiBooks, { ...output, url: page.url() });
});

test("AI extraction replays its Plan with no model call", async () => {
  await using engine = await TestEngine.start();
  await using run = await engine.connection.startCrawl(aiCrawl, {
    seeds: [seed(BOOK, aiBook)],
    plan: "tests/books.plan.json",
    recording: { mode: "replay", path: "tests/books.warc.gz" },
    settings: { plan: { frozen: true } }, // a step the Plan lacks throws PlanStepMissingError instead of calling a model
  });
  const report = await run.finish();
  assert.equal(report.usage.model.calls, 0);
});

// Or a Scripted model: here, a provider whose funds are spent, an error no retry helps.
test("the handler fails cleanly when the model funds are spent", async () => {
  const model = scriptedModel([fakeError(ModelFundsExhaustedError, { message: "credit balance too low" })]);
  await using engine = await TestEngine.start({
    modelProviders: { scripted: model },
    settings: { ai: { model: "scripted/any" }, extract: { mode: "model" } },
  });
  const attempt = await engine.runHandler(aiCrawl, aiBook, { url: BOOK, response: { path: "tests/fixtures/a-light-in-the-attic.html" } });
  assert.ok(attempt.ended.kind === "failed" && attempt.ended.error.code === "model_funds_exhausted");
  assert.equal(model.requests.length, 1); // closing the engine fails if a reply is left unused
});
