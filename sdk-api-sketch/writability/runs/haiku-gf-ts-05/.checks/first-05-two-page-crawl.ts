import type { CrawlRequestOutcome } from "@goodfetch/foxhound";
import { Crawl, Foxhound, seed } from "@goodfetch/foxhound";
import { z } from "zod";

const START_URL = "https://books.toscrape.com/";

const CategoryPayload = z.object({ category: z.string().min(1) });

const BookRecord = z.object({
  title: z.string().min(1),
  price: z.number().nonnegative(),
  stock: z.number().int().nonnegative(),
  rating: z.number().int().min(1).max(5),
  url: z.string().min(1),
  category: z.string().min(1),
});

const RATING_WORDS = new Map<string, number>([
  ["One", 1],
  ["Two", 2],
  ["Three", 3],
  ["Four", 4],
  ["Five", 5],
]);

function parsePrice(text: string): number {
  const amount = /(\d+(?:\.\d+)?)/.exec(text)?.[1];
  if (amount === undefined) {
    throw new Error(`no price in "${text}"`);
  }
  return Number(amount);
}

function parseStock(text: string): number {
  const count = /\((\d+) available\)/.exec(text)?.[1];
  if (count === undefined) {
    throw new Error(`no stock count in "${text}"`);
  }
  return Number.parseInt(count, 10);
}

function parseRating(className: string | null): number {
  for (const word of (className ?? "").split(/\s+/)) {
    const rating = RATING_WORDS.get(word);
    if (rating !== undefined) {
      return rating;
    }
  }
  throw new Error(`no star rating in class "${className ?? ""}"`);
}

const crawl = new Crawl();
const homeLabel = crawl.label("home");
const categoryLabel = crawl.label("category", { payload: CategoryPayload });
const bookLabel = crawl.label("book", { payload: CategoryPayload });
const books = crawl.dataset("books", { schema: BookRecord });

crawl.handle(homeLabel, async (ctx) => {
  // Category links of the side list; the top-level "Books" link sits one list level higher.
  await ctx.enqueueLinks(ctx.page.locator(".side_categories ul ul a"), categoryLabel, (link) => {
    const category = link.text.trim();
    return category === "" ? null : { category };
  });
});

crawl.handle(categoryLabel, async (ctx) => {
  const { category } = ctx.request.payload;
  await ctx.enqueueLinks(ctx.page.locator("article.product_pod h3 a"), bookLabel, { category });
  await ctx.enqueueLinks(ctx.page.locator("li.next a"), categoryLabel, { category });
});

crawl.handle(bookLabel, async (ctx) => {
  const product = ctx.page.locator(".product_main");
  const title = (await product.locator("h1").textContent()).trim();
  const price = parsePrice(await product.locator(".price_color").textContent());
  const stock = parseStock(await product.locator(".availability").textContent());
  const rating = parseRating(await product.locator(".star-rating").getAttribute("class"));
  await ctx.push(books, {
    title,
    price,
    stock,
    rating,
    url: ctx.page.url(),
    category: ctx.request.payload.category,
  });
});

await using foxhound = await Foxhound.connect();

const run = await foxhound.startCrawl(crawl, {
  seeds: [seed(START_URL, homeLabel)],
  settings: { crawl: { scope: { strategy: "same_hostname" } } },
});

await run.finish();

// Outcomes are keyed by request ID, so a request's latest outcome is the one counted.
const outcomes = new Map<string, CrawlRequestOutcome>();
for await (const outcome of run.outcomes()) {
  outcomes.set(outcome.request.id, outcome);
}

const counts = { succeeded: 0, failed: 0, skipped: 0, blocked: 0 };
const explanations: string[] = [];
for (const outcome of outcomes.values()) {
  const { url } = outcome.request;
  switch (outcome.kind) {
    case "succeeded":
      counts.succeeded += 1;
      break;
    case "failed":
      counts.failed += 1;
      explanations.push(`failed ${url}: ${outcome.error.code}: ${outcome.error.message}`);
      break;
    case "skipped":
      counts.skipped += 1;
      explanations.push(`skipped ${url}: ${outcome.reason}${outcome.message === null ? "" : `: ${outcome.message}`}`);
      break;
    case "blocked":
      counts.blocked += 1;
      explanations.push(`blocked ${url}: ${outcome.blockCause}`);
      break;
    case "unknown":
      explanations.push(`unknown outcome ${url}: ${outcome.raw}`);
      break;
  }
}

const exported = await run.export(books, { format: "json" });
await exported.saveAs("books.json");

console.log(
  `succeeded ${counts.succeeded}, failed ${counts.failed}, skipped ${counts.skipped}, blocked ${counts.blocked}`,
);
for (const line of explanations) {
  console.log(line);
}
console.log(`exported ${exported.records()} records to books.json`);
