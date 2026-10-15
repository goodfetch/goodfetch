import { writeFile } from "node:fs/promises";
import { z } from "zod";
import { Crawl, Foxhound, seed } from "@goodfetch/foxhound";
import type { CrawlContext, Label, Locator, Response } from "@goodfetch/foxhound";

const QUOTES_URL = "https://quotes.toscrape.com";
const QUOTE = ".quote";
const NEXT_PAGE = "li.next a";
const VERSIONS = ["plain", "js", "scroll"] as const;

type Version = (typeof VERSIONS)[number];

const QuoteRecord = z.object({
  version: z.enum(VERSIONS),
  text: z.string(),
  author: z.string(),
  tags: z.array(z.string()),
});

const ApiQuote = z.object({
  text: z.string(),
  author: z.union([z.string(), z.object({ name: z.string() })]),
  tags: z.array(z.string()),
});

const ApiPage = z.object({
  quotes: z.array(ApiQuote),
  has_next: z.boolean(),
});

interface Quote {
  readonly text: string;
  readonly author: string;
  readonly tags: readonly string[];
}

function fromApi(quote: z.infer<typeof ApiQuote>): Quote {
  return {
    text: quote.text,
    author: typeof quote.author === "string" ? quote.author : quote.author.name,
    tags: quote.tags,
  };
}

async function readApiPage(response: Response): Promise<{ quotes: Quote[]; hasNext: boolean }> {
  const parsed = ApiPage.safeParse(await response.json());
  if (!parsed.success) {
    throw new Error(`Unexpected quotes JSON from ${response.url()}: ${parsed.error.message}`);
  }
  return { quotes: parsed.data.quotes.map(fromApi), hasNext: parsed.data.has_next };
}

async function readQuotes(elements: Locator): Promise<Quote[]> {
  const quotes: Quote[] = [];
  for (const element of await elements.all()) {
    quotes.push({
      text: await element.locator(".text").textContent(),
      author: await element.locator(".author").textContent(),
      tags: await element.locator(".tags .tag").allTextContents(),
    });
  }
  return quotes;
}

async function main(): Promise<void> {
  const crawl = new Crawl();
  const plain = crawl.label("plain");
  const js = crawl.label("js");
  // The scrolling page's quotes exist only in the JSON its script loads on scroll, so its requests use a browser.
  const scroll = crawl.label("scroll", { settings: { fetch: { tier: "browser" } } });
  const quotes = crawl.dataset("quotes", { schema: QuoteRecord, key: ["version", "author", "text"] });

  // Quotes already recorded for each version, so a quote is recorded once per version.
  const recorded = new Set<string>();

  // Images and fonts are not downloaded in the browser: the route is installed before each navigation.
  crawl.beforeNavigation(
    ({ page }) => {
      page.route("**/*", (route) => {
        const type = route.request().resourceType();
        return type === "image" || type === "font" ? route.abort() : route.fallback();
      });
    },
    { labels: [js, scroll] },
  );

  async function pushQuotes(ctx: CrawlContext<Label<undefined>>, version: Version, found: readonly Quote[]): Promise<void> {
    for (const quote of found) {
      const identity = JSON.stringify([version, quote.author, quote.text]);
      if (recorded.has(identity)) {
        continue;
      }
      recorded.add(identity);
      await ctx.push(quotes, { version, text: quote.text, author: quote.author, tags: [...quote.tags] });
    }
  }

  // One handler for the HTML versions. On the HTTP tier, a static document without quotes escalates the
  // request to the browser, where the page's script renders them; a page still lacking them fails.
  async function handleListing(ctx: CrawlContext<Label<undefined>>, version: "plain" | "js"): Promise<void> {
    const { page } = ctx;
    if ((await page.locator(QUOTE).count()) === 0) {
      await page.locator(QUOTE).first().waitFor({ state: "attached" });
    }
    const found = await readQuotes(page.locator(QUOTE));
    if (found.length === 0) {
      throw new Error(`No quotes on ${ctx.request.url}`);
    }
    await ctx.enqueueLinks(page.locator(NEXT_PAGE), ctx.request.label);
    await pushQuotes(ctx, version, found);
  }

  // Infinite scroll: the quotes come from the JSON responses the page loads, never from the screen.
  async function handleScroll(ctx: CrawlContext<Label<undefined>>): Promise<void> {
    const { page } = ctx;
    const isQuotesApi = (url: URL): boolean => url.pathname === "/api/quotes";
    // The page's first JSON call runs as its document loads, before this handler: the reload makes the page
    // make that call again while the wait is already armed.
    const [firstResponse] = await Promise.all([page.waitForResponse(isQuotesApi), page.reload()]);
    let batch = await readApiPage(firstResponse);
    const found: Quote[] = [...batch.quotes];
    while (batch.hasNext) {
      // Scrolling to the bottom makes the page request its next batch.
      const [response] = await Promise.all([
        page.waitForResponse(isQuotesApi),
        page.evaluate("window.scrollTo(0, document.body.scrollHeight)"),
      ]);
      batch = await readApiPage(response);
      found.push(...batch.quotes);
    }
    if (found.length === 0) {
      throw new Error(`No quotes in the JSON loaded by ${ctx.request.url}`);
    }
    await pushQuotes(ctx, "scroll", found);
  }

  crawl.handle(plain, (ctx) => handleListing(ctx, "plain"));
  crawl.handle(js, (ctx) => handleListing(ctx, "js"));
  crawl.handle(scroll, handleScroll);

  await using foxhound = await Foxhound.connect();
  const run = await foxhound.startCrawl(crawl, {
    seeds: [
      seed(`${QUOTES_URL}/`, plain),
      seed(`${QUOTES_URL}/js/`, js),
      seed(`${QUOTES_URL}/scroll`, scroll),
    ],
  });
  const report = await run.finish();

  const rows = [];
  for await (const record of run.records(quotes)) {
    rows.push(record.value);
  }
  await writeFile("quotes.json", JSON.stringify(rows, null, 2));
  console.log(`${rows.length} quotes recorded; ${report.requests.failed} requests failed; verdict ${report.verdict}`);
  if (report.requests.failed > 0) {
    process.exitCode = 1;
  }
}

await main();
