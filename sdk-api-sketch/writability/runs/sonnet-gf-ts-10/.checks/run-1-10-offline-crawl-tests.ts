/// <reference types="node" />
// Golden scenario 10: offline tests of the crawls of scenarios 05 and 08, with Node's own test runner
// (`node --test 10-offline-crawl-tests.ts`). No test reaches the network or a model:
//  - every Run replays a Response recording (the kit's default is an empty one), so a request the
//    recording lacks is refused, and each test fails naming the address it tried;
//  - `fetch` of the test process itself is replaced by a function that fails naming its address;
//  - the model is a Scripted model or the Plan an earlier run learned, replayed `frozen`.
import assert from "node:assert/strict";
import { afterEach, beforeEach, describe, test } from "node:test";
import { ResponseNotRecordedError } from "@goodfetch/foxhound";
import type { CrawlRunReport } from "@goodfetch/foxhound";
import { TestEngine, scriptedModel } from "@goodfetch/foxhound/testing";
import * as scenario05 from "./05-two-page-crawl.ts";
import * as scenario08 from "./08-ai-in-crawl-handler.ts";

// What the earlier live runs left, committed beside the tests (paths relative to the working directory).
/** The Response recording of one live run of scenario 05: books.toscrape.com's 1000 books. */
const RECORDING_05 = "fixtures/scenario-05.warc.gz";
const EXPECTED_BOOKS_05 = 1000;
/** The Response recording of one live run of scenario 08 (its model calls are never recorded). */
const RECORDING_08 = "fixtures/scenario-08.warc.gz";
/** Scenario 08 enqueues three categories: Travel (11 books), Mystery (32) and Historical Fiction (26). */
const EXPECTED_BOOKS_08 = 69;

const BOOK_URL = "https://books.toscrape.com/catalogue/a-light-in-the-attic_1000/index.html";

// One book page, reduced to the parts the handlers read.
const BOOK_PAGE = `<!DOCTYPE html>
<html lang="en">
<head><meta charset="utf-8"><title>A Light in the Attic | Books to Scrape</title></head>
<body>
  <div class="page_inner">
    <article class="product_page">
      <div class="row">
        <div class="col-sm-6 product_main">
          <h1>A Light in the Attic</h1>
          <p class="price_color">£51.77</p>
          <p class="instock availability"><i class="icon-ok"></i> In stock (22 available) </p>
          <p class="star-rating Three"><i class="icon-star"></i></p>
        </div>
      </div>
    </article>
  </div>
</body>
</html>`;

const BOOK_PAGE_WITHOUT_PRICE = BOOK_PAGE.replace('<p class="price_color">£51.77</p>', "");

/** Fails any test that reaches the network from the test process itself, saying which address. */
const realFetch = globalThis.fetch;
function blockNetwork(): void {
  globalThis.fetch = (async (input: Parameters<typeof fetch>[0]) => {
    const address =
      typeof input === "string" ? input : input instanceof URL ? input.href : input.url;
    throw new Error(`offline test tried to reach the network: ${address}`);
  }) as typeof fetch;
}

beforeEach(blockNetwork);
afterEach(() => {
  globalThis.fetch = realFetch;
});

/**
 * Fails, naming every address, when a replaying Run asked for something its recording lacks, or when
 * a request ended other than `succeeded`.
 */
function assertNothingMissed(report: CrawlRunReport): void {
  const misses = report.recording?.misses ?? [];
  assert.deepEqual(
    misses.map((miss) => `${miss.method} ${miss.url}`),
    [],
    `the recording lacks requests the crawl made: ${misses.map((miss) => miss.url).join(", ")}`,
  );
  assert.equal(report.requests.failed, 0, `failed requests: ${JSON.stringify(report.requests.byErrorCode)}`);
  assert.equal(report.requests.blocked, 0);
}

describe("scenario 05, the book page handler", () => {
  test("turns a saved book page into its item, with the category it was given", async () => {
    await using engine = await TestEngine.start();
    const attempt = await engine.runHandler(scenario05.crawl, scenario05.book, {
      url: BOOK_URL,
      payload: { category: "Poetry" },
      response: { body: BOOK_PAGE },
    });

    assert.deepEqual(attempt.ended, { kind: "returned" });
    assert.deepEqual(attempt.records(scenario05.books), [
      {
        title: "A Light in the Attic",
        price: "51.77",
        inStock: 22,
        rating: 3,
        url: BOOK_URL,
        category: "Poetry",
      },
    ]);
    assert.deepEqual(attempt.enqueued, []);
  });

  test("skips a page with no price, and stages no item", async () => {
    await using engine = await TestEngine.start();
    const attempt = await engine.runHandler(scenario05.crawl, scenario05.book, {
      url: BOOK_URL,
      payload: { category: "Poetry" },
      response: { body: BOOK_PAGE_WITHOUT_PRICE },
    });

    assert.equal(attempt.ended.kind, "skipped");
    assert.deepEqual(attempt.records(scenario05.books), []);
  });

  test("fails, naming the address, when the handler's page is not saved", async () => {
    await using engine = await TestEngine.start();
    await assert.rejects(
      engine.runHandler(scenario05.crawl, scenario05.book, {
        url: BOOK_URL,
        payload: { category: "Poetry" },
      }),
      (error: unknown) => {
        assert.ok(error instanceof ResponseNotRecordedError);
        assert.equal(error.details.url, BOOK_URL);
        return true;
      },
    );
  });
});

describe("scenario 05, the whole crawl", () => {
  test("replays the recorded run and holds the expected number of books", async () => {
    await using engine = await TestEngine.start();
    await using run = await engine.connection.startCrawl(scenario05.crawl, {
      seeds: scenario05.seeds,
      recording: { mode: "replay", path: RECORDING_05 },
    });
    const report = await run.finish();

    assertNothingMissed(report);
    assert.equal(report.recording?.mode, "replay");

    const urls = new Set<string>();
    for await (const record of run.records(scenario05.books)) urls.add(record.value.url);
    assert.equal(urls.size, EXPECTED_BOOKS_05);
    assert.equal(report.records["books"]?.committed, EXPECTED_BOOKS_05);
  });
});

describe("scenario 08, the AI extraction", () => {
  test("replays the Plan learned on the earlier run, frozen: no model is called", async () => {
    // No Model provider is registered or configured, so a model call could only fail; the frozen Plan
    // forbids one as well, and a step it lacks throws `PlanStepMissingError` instead of compiling.
    await using engine = await TestEngine.start();
    await using run = await engine.connection.startCrawl(scenario08.crawl, {
      seeds: [scenario05.seeds[0]!],
      plan: scenario08.PLAN,
      recording: { mode: "replay", path: RECORDING_08 },
      settings: { plan: { frozen: true } },
    });
    const report = await run.finish();

    assertNothingMissed(report);
    assert.equal(report.requests.skipped, 0, `skipped: ${JSON.stringify(report.requests.bySkipReason)}`);
    assert.equal(report.usage.model.calls, 0, "a model was called");
    assert.equal(report.plan.status, "frozen");

    const urls = new Set<string>();
    for await (const record of run.records(scenario08.books)) urls.add(record.value.url);
    assert.equal(urls.size, EXPECTED_BOOKS_08);
  });

  test("runs the handler on a scripted model reply, asking it exactly once", async () => {
    const model = scriptedModel([
      {
        text: JSON.stringify({ title: "A Light in the Attic", price: "51.77", inStock: 22, rating: 3 }),
        finishReason: "stop",
        usage: { inputTokens: 900, outputTokens: 40 },
      },
    ]);
    // `model` mode: the scripted reply is the extraction itself, with no Plan to compile.
    await using engine = await TestEngine.start({
      modelProviders: { scripted: model },
      settings: { ai: { model: "scripted/books" }, extract: { mode: "model" } },
    });
    const attempt = await engine.runHandler(scenario08.crawl, scenario08.book, {
      url: BOOK_URL,
      response: { body: BOOK_PAGE },
    });

    assert.deepEqual(attempt.ended, { kind: "returned" });
    assert.deepEqual(attempt.records(scenario08.books), [
      { title: "A Light in the Attic", price: "51.77", inStock: 22, rating: 3, url: BOOK_URL },
    ]);
    assert.equal(model.requests.length, 1);
    assert.equal(model.remaining, 0);
    assert.equal(attempt.usage.calls, 1);
    assert.deepEqual(
      attempt.usage.byModel.map((used) => used.model),
      ["scripted/books"],
    );
  });

  test("fails, rather than calling a real model, when none is configured", async () => {
    await using engine = await TestEngine.start();
    const attempt = await engine.runHandler(scenario08.crawl, scenario08.book, {
      url: BOOK_URL,
      response: { body: BOOK_PAGE },
    });

    assert.equal(attempt.ended.kind, "failed");
    if (attempt.ended.kind === "failed") assert.equal(attempt.ended.error.code, "model_not_configured");
    assert.deepEqual(attempt.records(scenario08.books), []);
    assert.equal(attempt.usage.calls, 0);
  });
});
