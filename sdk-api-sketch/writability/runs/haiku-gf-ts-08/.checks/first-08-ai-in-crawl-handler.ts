// Crawls the first three categories of the side list of https://books.toscrape.com/ and every book
// they list, following each category's "next" pages. Each book's fields are extracted by a language
// model through Collie's compiled `extract`: the model is asked once for the book layout, its locators
// are kept in the Run's Plan, and later books replay from it with no model call. A page that no
// longer matches the learned layout drifts and is repaired, which calls the model again. The run's
// model allowance stops every model call once it is spent: the crawl then finishes without the model,
// and the books that could not be extracted are reported with their reason.

import { Crawl, Foxhound, seed } from "@goodfetch/foxhound";
import type { CrawlRequestOutcome, ModelReference, Money, Page, StepResult } from "@goodfetch/foxhound";
import { z } from "zod";

const START_URL = "https://books.toscrape.com/";
const MODEL: ModelReference = "anthropic/claude-haiku-4-5";
const MODEL_BUDGET: Money = { amount: "0.50", currency: "USD" };
const CATEGORY_LIMIT = 3;

const BOOK_INSTRUCTION =
  "Read the book shown on this page: its title; its price in pounds as a number, without the currency sign; " +
  "the number of copies in stock, the N in 'In stock (N available)', or 0 when it is out of stock; and its " +
  "star rating as a whole number from 1 to 5, where the rating is given in words from One to Five.";

const BookSchema = z.object({
  title: z.string(),
  priceGbp: z.number(),
  inStock: z.number().int().min(0),
  starRating: z.number().int().min(1).max(5),
});
type Book = z.infer<typeof BookSchema>;

const crawl = new Crawl();
const home = crawl.label("home");
const category = crawl.label("category");
const book = crawl.label("book");
const books = crawl.dataset("books", { schema: BookSchema });

// The first book of the layout is learned alone, so the model is asked once; the books after it wait
// for that answer and then replay the Plan. A failed first attempt also releases them.
let learning: Promise<unknown> | undefined;

async function extractBook(page: Page): Promise<StepResult<Book>> {
  if (learning === undefined) {
    const first = page.extract(BOOK_INSTRUCTION, BookSchema, { mode: "compiled" });
    learning = first.catch(() => undefined);
    return first;
  }
  await learning;
  return page.extract(BOOK_INSTRUCTION, BookSchema, { mode: "compiled" });
}

crawl.handle(home, async (ctx) => {
  await ctx.enqueueLinks(ctx.page.locator("ul.nav-list ul a"), category, { limit: CATEGORY_LIMIT });
});

crawl.handle(category, async (ctx) => {
  await ctx.enqueueLinks(ctx.page.locator("article.product_pod h3 a"), book);
  await ctx.enqueueLinks(ctx.page.locator("li.next a"), category);
});

const stepSources = new Map<string, number>();

crawl.handle(book, async (ctx) => {
  // An escaping error ends the request: a spent model allowance ends it `skipped` with the reason
  // `model_allowance`, any other failure `failed` with its code. Both are read from the outcomes below.
  const result = await extractBook(ctx.page);
  stepSources.set(result.step.source, (stepSources.get(result.step.source) ?? 0) + 1);
  await ctx.push(books, result.output);
});

function describeOutcome(outcome: CrawlRequestOutcome): string {
  switch (outcome.kind) {
    case "succeeded":
      return "succeeded";
    case "failed":
      return `failed (${outcome.error.code})`;
    case "skipped":
      return `skipped (${outcome.reason}${outcome.message === null ? "" : `: ${outcome.message}`})`;
    case "blocked":
      return `blocked (${outcome.blockCause})`;
    case "unknown":
      return `unknown (${outcome.raw})`;
  }
}

await using foxhound = await Foxhound.connect();

const run = await foxhound.startCrawl(crawl, {
  seeds: [seed(START_URL, home)],
  settings: { ai: { model: MODEL, allowance: { price: MODEL_BUDGET } } },
});

const bookOutcomes = new Map<string, CrawlRequestOutcome>();
const collectOutcomes = (async () => {
  for await (const outcome of run.outcomes({ labels: [book] })) {
    bookOutcomes.set(outcome.request.id, outcome);
  }
})();

const extracted: Book[] = [];
const collectRecords = (async () => {
  for await (const record of run.records(books)) {
    extracted.push(record.value);
  }
})();

const report = await run.finish();
await Promise.all([collectOutcomes, collectRecords]);

console.log(`Run ${report.lifecycle}, completeness ${report.completeness}.`);
console.log(`Extracted ${extracted.length} books.`);
console.log(`Extraction steps by source: ${JSON.stringify(Object.fromEntries(stepSources))}.`);

const unextracted = [...bookOutcomes.values()].filter((outcome) => outcome.kind !== "succeeded");
console.log(`Left unextracted: ${unextracted.length}.`);
for (const outcome of unextracted) {
  console.log(`  ${outcome.request.url}: ${describeOutcome(outcome)}`);
}

const modelUsage = report.usage.model;
const price = modelUsage.price === null ? "not priced" : `${modelUsage.price.amount} ${modelUsage.price.currency}`;
console.log(
  `Model: ${modelUsage.calls} calls, ${modelUsage.tokens} tokens ` +
    `(${modelUsage.inputTokens} input, ${modelUsage.outputTokens} output), cost ${price}.`,
);
const budget = report.usage.modelAllowance.find((allowance) => allowance.kind === "price");
if (budget !== undefined) {
  console.log(
    budget.reachedAt === null
      ? `Model budget of ${MODEL_BUDGET.amount} ${MODEL_BUDGET.currency} was not reached.`
      : `Model budget of ${MODEL_BUDGET.amount} ${MODEL_BUDGET.currency} was reached at ${budget.reachedAt.toISOString()}.`,
  );
}
