/// <reference types="node" />
// Offline tests of scenario 05's crawl and scenario 08's AI extraction. They run with no network and
// no model: the testing kit's engine replays an empty Response recording unless a Run names one, and
// it has no Model provider unless the test registers one. A request it cannot answer fails with
// `ResponseNotRecordedError`, which names the address the program tried.
import assert from "node:assert/strict";
import { test } from "node:test";
import { TestEngine, scriptedModel } from "@goodfetch/foxhound/testing";
import * as scenario05 from "./05-two-page-crawl.js";
import * as scenario08 from "./08-ai-in-crawl-handler.js";

const BOOK_URL = "https://books.toscrape.com/catalogue/a-light-in-the-attic_1000/index.html";

/** A saved copy of one product page of books.toscrape.com. */
const BOOK_PAGE = `<!DOCTYPE html>
<html lang="en-us">
<head><meta charset="utf-8"><title>A Light in the Attic | Books to Scrape</title></head>
<body>
<div class="page_inner">
<div class="row">
<div class="col-sm-6 product_main">
<h1>A Light in the Attic</h1>
<p class="star-rating Three"><i class="icon-star"></i><i class="icon-star"></i><i class="icon-star"></i><i class="icon-star"></i><i class="icon-star"></i></p>
<p class="price_color">£51.77</p>
<p class="instock availability">
<i class="icon-ok"></i>
In stock (22 available)
</p>
</div>
</div>
</div>
</body>
</html>`;

/** The Response recording that one earlier live run of scenario 05 wrote (`mode: "record"`). */
const RECORDING = "recordings/scenario-05.warc.gz";

/** books.toscrape.com lists 1000 books across its 50 categories. */
const EXPECTED_BOOKS = 1000;

test("the book handler stores the book on the page, with the category it was given", async () => {
  await using engine = await TestEngine.start();
  const attempt = await engine.runHandler(scenario05.crawl, scenario05.book, {
    url: BOOK_URL,
    response: { body: BOOK_PAGE, contentType: "text/html; charset=utf-8" },
    payload: { category: "Poetry" },
  });

  assert.equal(attempt.ended.kind, "returned");
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
});

test("the whole crawl, replayed from one earlier live run, stores every book", async () => {
  await using engine = await TestEngine.start();
  await using run = await engine.connection.startCrawl(scenario05.crawl, {
    seeds: scenario05.seeds,
    recording: { mode: "replay", path: RECORDING },
  });
  await run.finish();

  // A request the recording lacks is never sent: it fails here, and its message names the address.
  const failed: string[] = [];
  for await (const outcome of run.outcomes({ kinds: ["failed"] })) {
    if (outcome.kind === "failed") failed.push(`${outcome.request.url}: ${outcome.error.message}`);
  }
  assert.deepEqual(failed, []);

  const stored: scenario05.Book[] = [];
  for await (const record of run.records(scenario05.books)) stored.push(record.value);
  assert.equal(stored.length, EXPECTED_BOOKS);
});

test("scenario 08's AI extraction runs on a scripted model reply, and calls no other model", async () => {
  const model = scriptedModel([
    {
      text: JSON.stringify({ title: "A Light in the Attic", price: "51.77", inStock: 22, rating: 3 }),
      finishReason: "stop",
      usage: { inputTokens: 1200, outputTokens: 40 },
    },
  ]);
  await using engine = await TestEngine.start({
    settings: { ai: { model: "vercel/openai/gpt-5-mini" }, extract: { mode: "model" } },
    modelProviders: { vercel: model },
  });
  const attempt = await engine.runHandler(scenario08.crawl, scenario08.book, {
    url: BOOK_URL,
    response: { body: BOOK_PAGE, contentType: "text/html; charset=utf-8" },
  });

  assert.equal(attempt.ended.kind, "returned");
  assert.equal(model.requests.length, 1);
  assert.deepEqual(attempt.records(scenario08.books), [
    {
      title: "A Light in the Attic",
      price: "51.77",
      inStock: 22,
      rating: 3,
      url: BOOK_URL,
    },
  ]);
});
