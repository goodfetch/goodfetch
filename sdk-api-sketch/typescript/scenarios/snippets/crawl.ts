// Snippet for Crawl definition and handler context (#477, ADR 0093), on golden scenarios 05, 06 and 08.
import { z } from "zod";
import { Crawl, Goodfetch, seed, type CrawlRequestOutcome, type Logger } from "goodfetch";

// Provisional, its owner's to decide: reading outcomes is #478's. `page.extract` is ADR 0091's;
// Datasets and `push` are ADR 0103's (output.ts).
declare const log: Logger;

// Scenario 05: three kinds of page, the category name passed as a typed Payload.
const Book = z.object({
  title: z.string(),
  price: z.string().regex(/^\d+\.\d{2}$/), // pounds, exact
  inStock: z.number().int(),
  rating: z.number().int().min(1).max(5),
  url: z.url(),
  category: z.string(),
});
// The declaration needs no connection: a module may hold it.
const crawl = new Crawl();
const books = crawl.dataset("books", { schema: Book, key: ["url"] });
const home = crawl.label("home");
const category = crawl.label("category", { payload: z.object({ name: z.string().min(1) }) });
const book = crawl.label("book", {
  payload: z.object({ category: z.string().min(1) }),
  settings: { timeout: { handler: 30_000 } },
});

crawl.handle(home, async ({ page, enqueue }) => {
  // Every category but the top-level "Books": the name read here travels with each request.
  for (const link of await page.locator(".side_categories ul ul a").all()) {
    const href = await link.getAttribute("href");
    if (href !== null) await enqueue(new URL(href, page.url()), category, { name: (await link.textContent()).trim() });
  }
});

crawl.handle(category, async ({ page, request, enqueueLinks }) => {
  const added = await enqueueLinks(page.locator("article.product_pod h3 a"), book, { category: request.payload.name });
  if (added.added.length === 0) log.warn("no book links", { url: page.url(), notAdded: added.notAdded });
  // Pagination: the request's own Label, so no reference to `category` inside its own declaration.
  await enqueueLinks(page.locator("li.next a"), request.label, { name: request.payload.name });
});

crawl.handle(book, async ({ page, request, skip, push }) => {
  const price = await page.locator(".product_main .price_color").textContent(); // waits; over HTTP, a missing price escalates
  const match = /£(\d+\.\d{2})/.exec(price);
  if (match?.[1] === undefined) return skip(`no price in "${price}"`); // ends the request `skipped`, reason `handler`
  const stock = /(\d+) available/.exec(await page.locator(".product_main .availability").textContent());
  await push(books, {
    title: await page.locator(".product_main h1").textContent(),
    price: match[1],
    inStock: Number(stock?.[1] ?? 0),
    rating: ratingOf(await page.locator(".product_main .star-rating").getAttribute("class")),
    url: page.url(), // the final URL, after redirects
    category: request.payload.category,
  });
});

function ratingOf(cls: string | null): number {
  return ["One", "Two", "Three", "Four", "Five"].findIndex((w) => cls?.includes(w) === true) + 1;
}

export async function scenario05(): Promise<Map<string, number>> {
  await using gf = await Goodfetch.connect({ logger: log });
  await using run = await gf.foxhound.start(crawl, {
    seeds: [seed("https://books.toscrape.com/", home)],
    settings: { crawl: { scope: { strategy: "same_hostname" } } }, // the built-in, written out
  });
  const counts = new Map<string, number>();
  for await (const outcome of outcomesOf(run.id)) {
    counts.set(outcome.kind, (counts.get(outcome.kind) ?? 0) + 1);
    if (outcome.kind === "failed") log.warn("failed", { url: outcome.request.url, code: outcome.error.code, fix: outcome.error.fix });
    if (outcome.kind === "skipped") log.info("skipped", { url: outcome.request.url, reason: outcome.reason, message: outcome.message });
  }
  return counts;
}
declare function outcomesOf(runId: string): AsyncIterable<CrawlRequestOutcome>; // #478's

// Typed mistakes, caught before anything runs.
export function mistakes(): void {
  crawl.handle(book, async ({ enqueue }) => {
    // @ts-expect-error a wrong Payload field is a type error, never a silent undefined in the handler
    await enqueue("https://books.toscrape.com/x", book, { name: "Travel" });
    // @ts-expect-error a Label with a Payload needs it
    await enqueue("https://books.toscrape.com/x", book);
    // @ts-expect-error a Label without one takes none
    await enqueue("https://books.toscrape.com/", home, { name: "x" });
  }); // a second handler: throws LabelDuplicateError at once, at run time
  // @ts-expect-error Politeness belongs to the site, never to a Label
  crawl.label("listing", { settings: { politeness: { domain: { concurrency: 4 } } } });
}

// Scenario 06: one handler over HTTP, the JavaScript version and the scrolling one; no second crawler.
const quotesCrawl = new Crawl();
const quotes = quotesCrawl.dataset("quotes", {
  schema: z.object({ text: z.string(), author: z.string(), tags: z.array(z.string()), version: z.string() }),
});
const quotePage = quotesCrawl.label("quotes");
// The scrolling page reads the JSON the page loads, which only a browser does: its own Label forces it.
const scrollPage = quotesCrawl.label("scroll", { settings: { fetch: { tier: "browser" } } });

quotesCrawl.handle(quotePage, async ({ page, request, tier, enqueueLinks, push }) => {
  // Waiting for a quote is what this page must hold: over HTTP, /js/'s empty document escalates the
  // request to the browser instead of passing as an empty success (failure point 1).
  await page.locator(".quote").first().waitFor();
  for (const q of await page.locator(".quote").all()) {
    await push(quotes, {
      text: await q.locator(".text").textContent(),
      author: await q.locator(".author").textContent(),
      tags: await q.locator(".tag").allTextContents(),
      version: new URL(request.url).pathname.startsWith("/js") ? "js" : "html",
    });
  }
  log.debug("fetched", { url: request.url, tier }); // `http` for /; `browser` for /js/ once learned, if area 6 splits the Label by path
  await enqueueLinks(page.locator("li.next a"), request.label);
});

quotesCrawl.handle(scrollPage, async ({ page }) => {
  using _media = page.route("**/*.{png,jpg,woff,woff2}", (route) => route.abort("blockedbyclient"));
  const last = page.waitForRequestFinished(/\/api\/quotes\?page=\d+/);
  await page.mouse.wheel(0, 10_000);
  const response = await (await last).response();
  if (response === null || !response.ok()) return;
  log.info("scrolled", { body: await response.json() });
});

export async function scenario06(): Promise<void> {
  await using gf = await Goodfetch.connect({ logger: log });
  await using run = await gf.foxhound.start(quotesCrawl, {
    seeds: [
      seed("https://quotes.toscrape.com/", quotePage),
      seed("https://quotes.toscrape.com/js/", quotePage),
      seed("https://quotes.toscrape.com/scroll", scrollPage),
    ],
  });
  log.info("started", { run: run.id });
}

// Scenario 08: an AI extraction on the crawl's own page, counted in the crawl's Run, learned once
// per Label on a domain.
const aiCrawl = new Crawl();
const aiBook = aiCrawl.label("book");
const aiBooks = aiCrawl.dataset("books", { schema: Book, key: ["url"] }); // its own: a crawl pushes only to Datasets it declares
aiCrawl.handle(aiBook, async ({ page, retry, push }) => {
  const { output: fields } = await page.extract("the book on this page", Book.omit({ url: true, category: true }));
  if (fields.title === "") return retry("the title had not rendered"); // `retry_requested`, under crawl.retry.max
  await push(aiBooks, { ...fields, url: page.url(), category: "unknown" });
});

// A Label with Sessions of its own (ADRs 0092, 0093): the account's orders need the saved login, the
// catalogue does not. A page that is not the expected page escalates over HTTP, and fails with
// `page_check_failed` in the browser; a login page is a lost login, which `login` brings back.
const shop = new Crawl();
const catalogue = shop.label("catalogue");
const orders = shop.label("orders", {
  settings: {
    session: {
      source: { kind: "saved_identity", name: "shop-account" },
      pageCheck: { expectedPage: { selector: "table.orders" }, loginPage: { url: "**/login*" } },
    },
  },
  login: async (page) => {
    await page.getByRole("button", { name: "Sign in" }).click();
  },
});
shop.handle(catalogue, async ({ page, enqueueLinks }) => {
  await enqueueLinks(page.locator("a.product"), catalogue);
});
shop.handle(orders, async ({ page }) => {
  log.info("orders", { rows: await page.locator("table.orders tr").count() });
});
// @ts-expect-error the Session's identity is the Run's, the same in every pool: a Label refuses it
shop.label("localized", { settings: { browser: { locale: "fr-FR" } } });

export const definitions = { crawl, quotesCrawl, aiCrawl, shop };
