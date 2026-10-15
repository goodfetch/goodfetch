/// <reference types="node" />
// Golden scenario 08: extract with AI inside a crawl, learning once per page type.
// The declaration needs no connection, so scenario 10's offline tests import it; `main` runs only
// when this file is the entry point.
import { z } from "zod";
import { Crawl, Foxhound, seed } from "@goodfetch/foxhound";

/** What the model extracts from a book's page. */
export const Book = z.object({
  title: z.string().min(1),
  price: z.string().regex(/^\d+\.\d{2}$/), // pounds, exact
  in_stock: z.number().int().min(0),
  rating: z.number().int().min(1).max(5),
});

/** The Plan file: one entry for the book's extraction, compiled once per page layout (ADR 0091). */
export const PLAN = "books.plan.json";

export const crawl = new Crawl();
export const books = crawl.dataset("books", {
  schema: Book.extend({ url: z.url() }),
  key: ["url"],
});
export const home = crawl.label("home");
export const category = crawl.label("category");
// A first compile runs inside its handler, so it gets more than `timeout.handler`'s 60 s.
export const book = crawl.label("book", { settings: { timeout: { handler: 180_000 } } });

crawl.handle(home, async ({ page, enqueueLinks }) => {
  await enqueueLinks(page.locator(".side_categories ul ul a"), category, { limit: 3 });
});

crawl.handle(category, async ({ page, enqueueLinks }) => {
  await enqueueLinks(page.locator("article.product_pod h3 a"), book);
  await enqueueLinks(page.locator("li.next a"), category);
});

crawl.handle(book, async ({ page, push }) => {
  // The first book compiles the Plan entry; the next replay it with no model call, and a page whose
  // layout drifted gets one Repair. Past the Model allowance, a step that needs the model throws
  // ModelAllowanceReachedError: escaping, it ends this request `skipped` (`model_allowance`).
  const { output } = await page.extract("the book on this page", Book);
  await push(books, { ...output, url: page.url() });
});

async function main(): Promise<void> {
  await using foxhound = await Foxhound.connect({
    settings: { ai: { model: "vercel/openai/gpt-5-mini" } },
  });
  await using run = await foxhound.startCrawl(crawl, {
    seeds: [seed("https://books.toscrape.com/", home)],
    plan: PLAN,
    // The Model allowance: not a Limit, so the crawl goes on once it is spent.
    settings: { ai: { allowance: { price: { amount: "0.50", currency: "USD" } } } },
  });
  for await (const outcome of run.outcomes({ kinds: ["failed", "skipped"], labels: [book] })) {
    if (outcome.kind === "skipped")
      console.log("unextracted", outcome.request.url, outcome.reason, outcome.message);
    if (outcome.kind === "failed")
      console.log("unextracted", outcome.request.url, outcome.error.code, outcome.error.fix);
  }
  const report = await run.finish(); // `incomplete` when a book was skipped for the allowance
  const { tokens, price } = report.usage.model;
  console.log("model use", {
    tokens,
    price: price === null ? "unpriced" : `${price.amount} ${price.currency}`,
  });
}

if (import.meta.main) await main();
