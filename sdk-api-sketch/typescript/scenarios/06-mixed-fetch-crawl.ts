// Golden scenario 06, "Crawl with HTTP and a browser only where needed" (Foxhound, TypeScript).
import { z } from "zod";
import { Crawl, Foxhound, seed, type Handler, type Label } from "@goodfetch/foxhound";

const Quote = z.object({
  text: z.string(),
  author: z.string(),
  tags: z.array(z.string()),
  version: z.string(),
});
type Quote = z.output<typeof Quote>;
/** One page of the scrolling version's JSON API. */
const QuotesPage = z.object({
  has_next: z.boolean(),
  quotes: z.array(
    z.object({
      text: z.string(),
      author: z.object({ name: z.string() }),
      tags: z.array(z.string()),
    }),
  ),
});

const crawl = new Crawl();
const quotes = crawl.dataset("quotes", { schema: Quote });
// The tier is learned per Label on a domain (`fetch.tier: auto`, the default), so each version that
// may need another tier is its own Label; the two share one handler.
const html = crawl.label("html");
const js = crawl.label("js");
// Scrolling and reading the JSON the page loads need a browser: this Label forces it, and keeps the
// API's responses from the start of its navigation, so page 1, loaded before the handler runs, is read
// without a reload.
const scroll = crawl.label("scroll", {
  settings: { fetch: { tier: "browser", keptResponses: { urls: [/\/api\/quotes\?page=\d+$/] } } },
});

const readQuotes: Handler<Label> = async ({ page, request, push, enqueueLinks }) => {
  // What the page must hold: over HTTP, /js/'s document has no quote, so this wait escalates the
  // request to the browser; it never ends as an empty success.
  await page.locator(".quote").first().waitFor({ state: "attached" });
  for (const quote of await page.locator(".quote").all()) {
    await push(quotes, {
      text: await quote.locator(".text").textContent(),
      author: await quote.locator(".author").textContent(),
      tags: await quote.locator(".tag").allTextContents(),
      version: request.label.name,
    });
  }
  // The Frontier fetches each URL once, so each page's quotes are pushed once.
  await enqueueLinks(page.locator("li.next a"), request.label);
};
crawl.handle(html, readQuotes);
crawl.handle(js, readQuotes);

/** The JSON page `n` the scrolling version loads. */
const apiPage = (n: number) => new RegExp(`/api/quotes\\?page=${n}$`);

crawl.handle(scroll, async ({ page, push, keptResponse }) => {
  // Page 1 loaded while the engine navigated: the Label kept it.
  let loaded = await keptResponse(apiPage(1));
  for (let n = 2; ; n++) {
    const body = QuotesPage.parse(await loaded.json()); // a body that is not JSON throws ResponseNotJSONError
    for (const quote of body.quotes) {
      await push(quotes, {
        text: quote.text,
        author: quote.author.name,
        tags: quote.tags,
        version: "scroll",
      });
    }
    if (!body.has_next) return;
    // Each JSON page is awaited once, by its number; one that never loads fails the request with
    // DeadlineExceededError, retried, then `failed`.
    [loaded] = await Promise.all([page.waitForResponse(apiPage(n)), page.mouse.wheel(0, 10_000)]);
  }
});

export async function collectQuotes(): Promise<Quote[]> {
  // No browser on the machine: the pinned one is fetched on first use.
  await using foxhound = await Foxhound.connect();
  await using run = await foxhound.startCrawl(crawl, {
    seeds: [
      seed("https://quotes.toscrape.com/", html),
      seed("https://quotes.toscrape.com/js/", js),
      seed("https://quotes.toscrape.com/scroll", scroll),
    ],
    // Blocked by the engine on every browser page before it navigates, a sample comparison of an HTTP
    // Label's page included; the HTTP tiers fetch no images or fonts anyway (ADRs 0047, 0117).
    settings: { browser: { blockResources: ["image", "font"] } },
  });
  const report = await run.finish();
  const { succeeded, failed, byErrorCode } = report.requests;
  console.log(`${succeeded} pages succeeded, ${failed} failed`, byErrorCode);
  const collected: Quote[] = [];
  for await (const record of run.records(quotes)) collected.push(record.value);
  return collected;
}

for (const quote of await collectQuotes())
  console.log(`[${quote.version}] ${quote.text} (${quote.author})`);
