/// <reference types="node" />
// Golden scenario 10: the crawl of scenario 05 and the AI extraction of scenario 08, tested with no
// network and no model, on `node:test`. The kit needs no runner of its own. Every Run the test
// engine starts replays a recording, an empty one when it names none, so a request it lacks is never
// sent: it fails with `response_not_recorded`, naming its address.
import assert from "node:assert/strict";
import { test } from "node:test";
import { seed } from "@goodfetch/foxhound";
import { TestEngine } from "@goodfetch/foxhound/testing";
import { book, books, crawl, seeds } from "./05-two-page-crawl.js";
import * as ai from "./08-ai-in-crawl-handler.js";

const BOOK = "https://books.toscrape.com/catalogue/a-light-in-the-attic_1000/index.html";
// One live run of scenario 05 recorded these, with `recording: { mode: "record", path }` on its start.
const RECORDING = { mode: "replay", path: "tests/books.warc.gz" } as const;

test("a book page gives its book, with the category passed to it", async () => {
  await using engine = await TestEngine.start();
  // The handler alone, on a saved page: no Frontier, no browser on the `http` tier.
  const attempt = await engine.runHandler(crawl, book, {
    url: BOOK,
    payload: { category: "Poetry" },
    response: { path: "tests/fixtures/a-light-in-the-attic.html" },
  });
  assert.deepEqual(attempt.ended, { kind: "returned" });
  assert.deepEqual(attempt.records(books), [
    {
      title: "A Light in the Attic",
      price: "51.77",
      in_stock: 22,
      rating: 3,
      url: BOOK,
      category: "Poetry",
    },
  ]);
});

test("the crawl collects every book from the recording", async () => {
  await using engine = await TestEngine.start();
  await using run = await engine.connection.startCrawl(crawl, { seeds, recording: RECORDING });
  const report = await run.finish(); // no Politeness waits: nothing reaches the site
  assert.deepEqual(report.recording?.misses, []); // a miss shows its method and URL here
  assert.equal(report.records["books"]?.committed, 1000);
});

test("the AI extraction replays its Plan with no model call", async () => {
  await using engine = await TestEngine.start(); // no Model provider: a model call would fail
  await using run = await engine.connection.startCrawl(ai.crawl, {
    seeds: [seed(BOOK, ai.book)],
    plan: ai.PLAN, // what scenario 08's live run learned, committed
    recording: RECORDING,
    // A step the Plan lacks fails with `plan_step_missing` instead of calling a model.
    settings: { plan: { frozen: true } },
  });
  const report = await run.finish();
  assert.deepEqual(report.recording?.misses, []); // each miss names its URL
  assert.equal(report.requests.failed, 0);
  assert.equal(report.records["books"]?.committed, 1);
  assert.equal(report.usage.model.calls, 0);
});
