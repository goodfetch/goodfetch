/// <reference types="node" />
// Golden scenario 05: crawl two page types into a Dataset. Scenarios 07 and 10 import its crawl;
// it runs only as the entry point (`import.meta.main`).
import { z } from "zod";
import { Crawl, Foxhound, seed } from "@goodfetch/foxhound";

export const Book = z.object({
  title: z.string().min(1),
  price: z.string().regex(/^\d+\.\d{2}$/), // pounds, an exact decimal kept as text
  inStock: z.number().int().min(0),
  rating: z.number().int().min(1).max(5),
  url: z.url(),
  category: z.string().min(1),
});
export type Book = z.infer<typeof Book>;

// The declaration does no I/O, so a module may hold it.
export const crawl = new Crawl();
export const books = crawl.dataset("books", { schema: Book, key: ["url"] });
export const home = crawl.label("home");
export const category = crawl.label("category", { payload: z.object({ name: z.string().min(1) }) });
export const book = crawl.label("book", { payload: z.object({ category: z.string().min(1) }) });
// Staying on books.toscrape.com is the default scope (the seed's host), and robots.txt, one request
// at a time with a second between starts and backing off on 429 are the default Politeness.
export const seeds = [seed("https://books.toscrape.com/", home)];

const RATINGS = ["One", "Two", "Three", "Four", "Five"];

crawl.handle(home, async ({ page, enqueueLinks }) => {
  // The nested list leaves out the top-level "Books"; each link's own text is its Payload.
  await enqueueLinks(page.locator(".side_categories ul ul a"), category, (link) => ({
    name: link.text.trim(),
  }));
});

crawl.handle(category, async ({ page, request, enqueueLinks }) => {
  const { name } = request.payload;
  await enqueueLinks(page.locator("article.product_pod h3 a"), book, { category: name });
  await enqueueLinks(page.locator("li.next a"), request.label, { name });
});

crawl.handle(book, async ({ page, request, push, skip }) => {
  const main = page.locator(".product_main");
  const text = await main.textContent();
  const price = /£(\d+\.\d{2})/.exec(text)?.[1];
  const inStock = /\((\d+) available\)/.exec(text)?.[1];
  const stars = (await main.locator(".star-rating").getAttribute("class")) ?? "";
  const rating = RATINGS.findIndex((word) => stars.includes(word)) + 1;
  if (price === undefined || inStock === undefined || rating === 0) {
    return skip(
      `no price, stock or rating on the page (price ${price}, stock ${inStock}, stars "${stars}")`,
    );
  }
  await push(books, {
    title: await main.locator("h1").textContent(),
    price,
    inStock: Number(inStock),
    rating,
    url: page.url(), // the final URL, after redirects
    category: request.payload.category,
  });
});

export async function main(): Promise<void> {
  await using foxhound = await Foxhound.connect();
  await using run = await foxhound.startCrawl(crawl, { seeds });
  const report = await run.finish();
  const exported = await run.export(books, { format: "json" });
  await exported.saveAs("books.json");
  console.log(`exported ${exported.records()} books to books.json`);

  for await (const outcome of run.outcomes({ kinds: ["failed", "skipped", "blocked"] })) {
    const { url } = outcome.request;
    if (outcome.kind === "failed")
      console.log(`failed ${url}: ${outcome.error.code}: ${outcome.error.message}`);
    else if (outcome.kind === "skipped")
      console.log(`skipped ${url}: ${outcome.reason} ${outcome.message ?? ""}`);
    else if (outcome.kind === "blocked") console.log(`blocked ${url}: ${outcome.blockCause}`);
  }
  const { succeeded, failed, skipped, blocked } = report.requests;
  console.log(`${succeeded} succeeded, ${failed} failed, ${skipped} skipped, ${blocked} blocked`);
  if (report.nothingDone !== null) console.log(`nothing done: ${report.nothingDone.reason}`);
}

if (import.meta.main) await main();
