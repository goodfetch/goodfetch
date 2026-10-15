// Collects every quote (text, author, tags) from the three versions of quotes.toscrape.com:
//   - html:   https://quotes.toscrape.com/        plain HTML, fetched over HTTP, paginated by "Next"
//   - js:     https://quotes.toscrape.com/js/     rendered by JavaScript, so the HTTP fetch has no quotes
//                                                 and the request escalates to the browser tier
//   - scroll: https://quotes.toscrape.com/scroll  infinite scroll, read from the JSON API responses
//
// One crawl, one handler for the html and js versions: the handler waits for the quotes, which an HTTP
// document without them cannot serve, so the engine runs the same request again in the browser.
import { Crawl, Foxhound, seed } from "@goodfetch/foxhound";
import type { CrawlContext, Handler, Label, Page } from "@goodfetch/foxhound";
import { z } from "zod";

const ORIGIN = "https://quotes.toscrape.com";
const WAIT_TIMEOUT = 30_000;

const VERSIONS = ["html", "js", "scroll"] as const;
type Version = (typeof VERSIONS)[number];

const QuoteRecord = z.object({
  version: z.enum(VERSIONS),
  text: z.string().min(1),
  author: z.string().min(1),
  tags: z.array(z.string()),
});

// What the infinite-scroll page loads from /api/quotes?page=N.
const ApiPage = z.object({
  has_next: z.boolean(),
  page: z.number(),
  quotes: z.array(
    z.object({
      text: z.string().min(1),
      author: z.object({ name: z.string().min(1) }),
      tags: z.array(z.string()),
    }),
  ),
});

const crawl = new Crawl();

// A quote is identified by its version and its text: it is recorded once per version.
const quotes = crawl.dataset("quotes", { schema: QuoteRecord, key: ["version", "text"] });

const htmlPages = crawl.label("html_page");
const jsPages = crawl.label("js_page");
// The scrolling version has no quotes in any document: it always needs a browser.
const scrollPage = crawl.label("scroll_page", { settings: { fetch: { tier: "browser" } } });

// Browser tiers only: no image and no font is downloaded. A route on every URL, so that the decision
// rests on the request's resource type rather than on a file extension.
crawl.beforeNavigation(
  ({ page }) => {
    page.route(/.*/, async (route) => {
      const type = route.request().resourceType();
      if (type === "image" || type === "font") {
        await route.abort("blockedbyclient");
      } else {
        await route.fallback();
      }
    });
  },
  { name: "block-images-and-fonts" },
);

// The handler of the two paginated versions, the same on the HTTP tier and in the browser.
function paginatedHandler(version: Version, label: Label<undefined>): Handler<Label<undefined>> {
  return async (ctx: CrawlContext<Label<undefined>>) => {
    const { page } = ctx;
    const cards = page.locator("div.quote");

    // A document that lacks the quotes cannot serve this wait: the request escalates to the browser,
    // and under a forced HTTP tier the call throws. Either way, a page with no quotes is no success.
    await cards.waitForCount({ min: 1 }, { timeout: WAIT_TIMEOUT });

    for (const card of await cards.all()) {
      const text = (await card.locator("span.text").textContent()).trim();
      const author = (await card.locator("small.author").textContent()).trim();
      const tags = (await card.locator("a.tag").allTextContents()).map((tag) => tag.trim());
      await ctx.push(quotes, { version, text, author, tags });
    }

    // Pagination: the "Next" link, absent on the last page.
    await ctx.enqueueLinks(page.locator("li.next > a"), label);
  };
}

// Reads the quotes from the JSON responses the infinite-scroll page loads while it is scrolled.
async function readScrollApi(page: Page): Promise<z.infer<typeof ApiPage>[]> {
  const apiCall = /\/api\/quotes\?page=\d+/;

  const nextCall = async (trigger: () => Promise<unknown>): Promise<z.infer<typeof ApiPage>> => {
    // Armed before the trigger, so the response cannot be missed.
    const finished = page.waitForRequestFinished(apiCall, { timeout: WAIT_TIMEOUT });
    await trigger();
    const response = await (await finished).response({ timeout: WAIT_TIMEOUT });
    if (response === null) {
      throw new Error("the quotes API request failed");
    }
    return ApiPage.parse(await response.json({ timeout: WAIT_TIMEOUT }));
  };

  // The engine has loaded the page before the handler runs, so its first API call is behind us:
  // load it again with the wait armed.
  const pages: z.infer<typeof ApiPage>[] = [];
  let current = await nextCall(() => page.reload());
  pages.push(current);
  while (current.has_next) {
    current = await nextCall(() => page.evaluate("window.scrollTo(0, document.body.scrollHeight)"));
    pages.push(current);
  }
  return pages;
}

crawl.handle(htmlPages, paginatedHandler("html", htmlPages));
crawl.handle(jsPages, paginatedHandler("js", jsPages));
crawl.handle(scrollPage, async (ctx) => {
  const seen = new Set<number>();
  for (const apiPage of await readScrollApi(ctx.page)) {
    if (seen.has(apiPage.page)) {
      continue;
    }
    seen.add(apiPage.page);
    for (const quote of apiPage.quotes) {
      await ctx.push(quotes, {
        version: "scroll",
        text: quote.text.trim(),
        author: quote.author.name.trim(),
        tags: quote.tags,
      });
    }
  }
  if (seen.size === 0) {
    ctx.retry("the scrolling page loaded no quotes");
  }
});

await using foxhound = await Foxhound.connect();

const run = await foxhound.startCrawl(crawl, {
  seeds: [seed(`${ORIGIN}/`, htmlPages), seed(`${ORIGIN}/js/`, jsPages), seed(`${ORIGIN}/scroll`, scrollPage)],
});

// Read the Records while the Run works, then let it finish.
const collecting = (async () => {
  const all: z.infer<typeof QuoteRecord>[] = [];
  for await (const record of run.records(quotes)) {
    all.push(record.value);
  }
  return all;
})();
const [records, report] = await Promise.all([collecting, run.finish()]);

// Each quote once per version.
const unique = new Map<string, z.infer<typeof QuoteRecord>>();
for (const record of records) {
  unique.set(`${record.version}\u0000${record.text}`, record);
}
const collected = [...unique.values()];

const counts: Record<Version, number> = { html: 0, js: 0, scroll: 0 };
for (const quote of collected) {
  counts[quote.version] += 1;
}

console.log(JSON.stringify({ counts, requests: report.requests, quotes: collected }, null, 2));

const empty = VERSIONS.filter((version) => counts[version] === 0);
if (report.requests.failed > 0 || report.requests.blocked > 0 || empty.length > 0) {
  throw new Error(
    `crawl incomplete: ${report.requests.failed} failed, ${report.requests.blocked} blocked, no quotes for: ${empty.join(", ") || "none"}`,
  );
}
