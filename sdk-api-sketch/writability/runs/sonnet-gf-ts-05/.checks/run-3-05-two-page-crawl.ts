import { Crawl, Foxhound, seed } from "@goodfetch/foxhound";
import type { CrawlRequestOutcome } from "@goodfetch/foxhound";
import { z } from "zod";

const START_URL = "https://books.toscrape.com/";
const OUTPUT_FILE = "books.json";

const RATINGS: Readonly<Record<string, number>> = { One: 1, Two: 2, Three: 3, Four: 4, Five: 5 };

const crawl = new Crawl();

const categoryPayload = z.object({ category: z.string() });

const home = crawl.label("home");
const category = crawl.label("category", { payload: categoryPayload });
const book = crawl.label("book", { payload: categoryPayload });

const books = crawl.dataset("books", {
  schema: z.object({
    title: z.string(),
    price: z.number(),
    inStock: z.number().int(),
    rating: z.number().int().min(1).max(5),
    url: z.string(),
    category: z.string(),
  }),
  key: ["url"],
});

// Home page: every category link in the side list, except the top-level "Books" one,
// which is the outer list item; the categories are in its nested list.
crawl.handle(home, async (ctx) => {
  await ctx.enqueueLinks(
    ctx.page.locator(".side_categories ul ul a"),
    category,
    (link) => ({ category: link.text.trim() }),
  );
});

// Category page: its book links and its "next" page link, carrying the category name along.
crawl.handle(category, async (ctx) => {
  const { category: name } = ctx.request.payload;
  await ctx.enqueueLinks(ctx.page.locator("article.product_pod h3 a"), book, { category: name });
  await ctx.enqueueLinks(ctx.page.locator("li.next a"), category, { category: name });
});

// Book page: one item, with the category passed from the category page.
crawl.handle(book, async (ctx) => {
  const { page } = ctx;
  const title = (await page.locator("div.product_main h1").textContent()).trim();

  const priceText = await page.locator("div.product_main p.price_color").textContent();
  const priceMatch = /\d+(?:\.\d+)?/.exec(priceText);
  if (priceMatch === null) return ctx.skip(`no price in "${priceText}"`);
  const price = Number(priceMatch[0]);

  const stockText = await page.locator("div.product_main p.availability").textContent();
  const stockMatch = /(\d+)\s+available/.exec(stockText);
  const inStock = stockMatch?.[1] === undefined ? 0 : Number.parseInt(stockMatch[1], 10);

  const ratingClass = (await page.locator("div.product_main p.star-rating").getAttribute("class")) ?? "";
  const ratingWord = ratingClass.split(/\s+/).find((word) => word in RATINGS);
  const rating = ratingWord === undefined ? undefined : RATINGS[ratingWord];
  if (rating === undefined) return ctx.skip(`no star rating in "${ratingClass}"`);

  await ctx.push(books, {
    title,
    price,
    inStock,
    rating,
    url: page.url(),
    category: ctx.request.payload.category,
  });
});

await using foxhound = await Foxhound.connect();
const run = await foxhound.startCrawl(crawl, {
  seeds: [seed(START_URL, home)],
  settings: { crawl: { scope: { strategy: "same_hostname" } } },
});

const report = await run.finish();

// Every request that did not succeed, keyed by request ID so a retried request counts once.
const problems = new Map<string, CrawlRequestOutcome>();
for await (const outcome of run.outcomes({ kinds: ["failed", "skipped", "blocked"] })) {
  problems.set(outcome.request.id, outcome);
}

const exported = await run.export(books, { format: "json" });
await exported.saveAs(OUTPUT_FILE);

console.log(`Run ${report.id}: ${report.lifecycle}, completeness ${report.completeness}`);
console.log(
  `Pages: ${report.requests.succeeded} succeeded, ${report.requests.failed} failed, ` +
    `${report.requests.skipped} skipped, ${report.requests.blocked} blocked`,
);
console.log(`Exported ${exported.records()} items to ${OUTPUT_FILE}`);

for (const outcome of problems.values()) {
  const where = `${outcome.request.url} (${outcome.request.label})`;
  switch (outcome.kind) {
    case "failed":
      console.log(`failed: ${where}: ${outcome.error.code}: ${outcome.error.message}`);
      break;
    case "skipped":
      console.log(`skipped: ${where}: ${outcome.reason}${outcome.message === null ? "" : `: ${outcome.message}`}`);
      break;
    case "blocked":
      console.log(`blocked: ${where}: ${outcome.blockCause}`);
      break;
    default:
      console.log(`unknown outcome: ${where}`);
  }
}
