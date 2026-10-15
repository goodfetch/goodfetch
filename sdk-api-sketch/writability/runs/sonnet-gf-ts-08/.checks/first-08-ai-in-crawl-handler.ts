// Crawl the first three categories of books.toscrape.com and every book they list, extracting each
// book with a language model that is asked once per page type. The model's spending is capped by the
// Run's Model allowance: once it is reached, books are still extracted from what was learned (the
// Plan), and those that would need the model are left unextracted and reported.

import { z } from "zod";
import { Crawl, Foxhound, seed, type CrawlRequestOutcome, type Money } from "@goodfetch/foxhound";

const HOME_URL = "https://books.toscrape.com/";
const CATEGORY_COUNT = 3;
const MODEL = (process.env["GOODFETCH_MODEL"] ?? "vercel/anthropic/claude-sonnet-4.5") as `${string}/${string}`;
const BUDGET_AMOUNT = process.env["MODEL_BUDGET_USD"] ?? "0.50";
if (!/^\d+(\.\d+)?$/.test(BUDGET_AMOUNT)) {
  throw new Error(`MODEL_BUDGET_USD must be a decimal number, got "${BUDGET_AMOUNT}"`);
}
const BUDGET: Money = { amount: BUDGET_AMOUNT as `${number}`, currency: "USD" };

// What the model reads from a book page, and the Record a book becomes.
const extractedBook = z.object({
  title: z.string().describe("The book's title"),
  price: z.number().describe("The price including tax, as a plain number without the currency symbol"),
  inStock: z.number().int().describe("The number of copies available in stock, 0 if out of stock"),
  rating: z.number().int().min(1).max(5).describe("The star rating from 1 to 5, read from the star-rating element"),
});
const bookRecord = extractedBook.extend({ url: z.string() });

const BOOK_INSTRUCTION = "Extract the book's title, price, number of copies in stock and star rating from this book page.";

// How each extraction was resolved: from the Plan (no model call), compiled now, or repaired on Drift.
const stepSources: Record<string, number> = {};

const crawl = new Crawl();

const home = crawl.label("home");
const category = crawl.label("category");
// One book handler at a time, so the first book teaches the Plan before any other book is read.
const book = crawl.label("book", { settings: { crawl: { handler: { concurrency: 1 } } } });
const books = crawl.dataset("books", { schema: bookRecord, key: ["url"] });

crawl.handle(home, async (ctx) => {
  await ctx.enqueueLinks(ctx.page.locator("div.side-categories ul.nav-list > li > ul > li > a"), category, {
    limit: CATEGORY_COUNT,
  });
});

crawl.handle(category, async (ctx) => {
  await ctx.enqueueLinks(ctx.page.locator("article.product_pod h3 a"), book);
  await ctx.enqueueLinks(ctx.page.locator("li.next a"), category);
});

crawl.handle(book, async (ctx) => {
  // Throws ModelAllowanceReachedError when the Plan holds no entry and the allowance is spent; it
  // escapes the handler and the request ends `skipped` with the reason `model_allowance`.
  const result = await ctx.page.extract(BOOK_INSTRUCTION, extractedBook);
  stepSources[result.step.source] = (stepSources[result.step.source] ?? 0) + 1;
  await ctx.push(books, { ...result.output, url: ctx.request.url });
});

function describeUnextracted(outcome: CrawlRequestOutcome): string {
  switch (outcome.kind) {
    case "succeeded":
      return "extracted";
    case "skipped":
      return `skipped (${outcome.reason})${outcome.message === null ? "" : `: ${outcome.message}`}`;
    case "failed":
      return `failed (${outcome.error.code}): ${outcome.error.message}`;
    case "blocked":
      return `blocked (${outcome.blockCause})`;
    case "unknown":
      return `unknown outcome kind ${outcome.raw}`;
  }
}

await using foxhound = await Foxhound.connect();

const run = await foxhound.startCrawl(crawl, {
  seeds: [seed(HOME_URL, home)],
  settings: {
    ai: { model: MODEL, allowance: { price: BUDGET } },
  },
});

// Read the outcomes and the Records while the crawl runs; a later outcome of a request supersedes
// an earlier one, so they are keyed by request ID.
const finalOutcomes = new Map<string, CrawlRequestOutcome>();
const readOutcomes = (async () => {
  for await (const outcome of run.outcomes()) {
    finalOutcomes.set(outcome.request.id, outcome);
  }
})();
const extracted = new Map<string, z.output<typeof bookRecord>>();
const readRecords = (async () => {
  for await (const record of run.records(books)) {
    extracted.set(record.value.url, record.value);
  }
})();

const report = await run.finish();
await Promise.all([readOutcomes, readRecords]);

console.log(`Run ${report.id}: ${report.lifecycle}, completeness ${report.completeness}`);
for (const reason of report.completenessReasons) {
  console.log(`  incomplete because: ${reason}`);
}

console.log(`\nExtracted ${extracted.size} books:`);
for (const value of extracted.values()) {
  console.log(`  ${value.title} | ${value.price} | ${value.inStock} in stock | ${value.rating} stars | ${value.url}`);
}

const unextracted = [...finalOutcomes.values()].filter((outcome) => outcome.request.label === book.name && outcome.kind !== "succeeded");
console.log(`\nBooks left unextracted: ${unextracted.length}`);
for (const outcome of unextracted) {
  console.log(`  ${outcome.request.url}: ${describeUnextracted(outcome)}`);
}
console.log(`Other requests that did not succeed: ${[...finalOutcomes.values()].filter((o) => o.request.label !== book.name && o.kind !== "succeeded").length}`);

console.log(`\nExtraction steps by source: ${JSON.stringify(stepSources)}`);
const usage = report.usage.model;
const cost = usage.price === null ? "unknown (no price in the price table)" : `${usage.price.amount} ${usage.price.currency}`;
console.log(`Model usage: ${usage.calls} calls, ${usage.tokens} tokens (${usage.inputTokens} in, ${usage.outputTokens} out), cost ${cost}`);
for (const model of usage.byModel) {
  console.log(`  ${model.model}: ${model.calls} calls, ${model.tokens} tokens`);
}
for (const allowance of report.usage.modelAllowance) {
  console.log(`Model allowance ${allowance.kind}: reached at ${allowance.reachedAt === null ? "never" : allowance.reachedAt.toISOString()}`);
}
