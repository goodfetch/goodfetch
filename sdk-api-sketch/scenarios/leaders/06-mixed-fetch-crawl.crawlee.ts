// Golden scenario 06, "Crawl with HTTP and a browser only where needed": the leader's program,
// Crawlee 3.18.2's AdaptivePlaywrightCrawler (TypeScript).
import { z } from "zod";
import {
  AdaptivePlaywrightCrawler,
  createAdaptivePlaywrightRouter,
  playwrightUtils,
  type AdaptivePlaywrightCrawlerContext,
} from "crawlee";

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

const router = createAdaptivePlaywrightRouter();

// The plain and the JavaScript versions share one handler: over HTTP, a missing `.quote` makes
// `waitForSelector` throw, and the crawler runs the request again in a browser.
async function readQuotes({
  request,
  waitForSelector,
  parseWithCheerio,
  pushData,
  enqueueLinks,
}: AdaptivePlaywrightCrawlerContext) {
  await waitForSelector(".quote");
  const $ = await parseWithCheerio();
  await pushData(
    $(".quote")
      .map((_, quote) => ({
        text: $(quote).find(".text").text(),
        author: $(quote).find(".author").text(),
        tags: $(quote)
          .find(".tag")
          .map((_, tag) => $(tag).text())
          .get(),
        version: request.label,
      }))
      .get(),
  );
  // In the browser, `enqueueLinks` waits 5 s for its selector and throws when it never appears, as
  // on the last page (3.18.2 source), so it is called only when there is a next page.
  if ($("li.next a").length > 0)
    await enqueueLinks({ selector: "li.next a", label: request.label });
}
router.addHandler("html", readQuotes);
router.addHandler("js", readQuotes);

// Reading `page` over HTTP throws, and the crawler runs the request again in a browser.
router.addHandler("scroll", async ({ page, pushData }) => {
  const apiPage = (n: number) => new RegExp(`/api/quotes\\?page=${n}$`);
  // Page 1 loaded during the navigation, before the handler could wait for it.
  let [loaded] = await Promise.all([page.waitForResponse(apiPage(1)), page.reload()]);
  for (let n = 2; ; n++) {
    const body = QuotesPage.parse(await loaded.json());
    await pushData(
      body.quotes.map((quote) => ({
        text: quote.text,
        author: quote.author.name,
        tags: quote.tags,
        version: "scroll",
      })),
    );
    if (!body.has_next) return;
    [loaded] = await Promise.all([page.waitForResponse(apiPage(n)), page.mouse.wheel(0, 10_000)]);
  }
});

const crawler = new AdaptivePlaywrightCrawler({
  requestHandler: router,
  preNavigationHooks: [
    async (context) => {
      // Over HTTP, reading `page` throws, and an uncaught throw would send the request to the browser.
      let page;
      try {
        page = context.page;
      } catch {
        return;
      }
      if (page !== undefined) {
        await playwrightUtils.blockRequests(page, {
          urlPatterns: [
            ".jpg",
            ".jpeg",
            ".png",
            ".gif",
            ".svg",
            ".webp",
            ".ico",
            ".woff",
            ".woff2",
            ".ttf",
            ".otf",
          ],
        });
      }
    },
  ],
});

const { requestsFinished, requestsFailed } = await crawler.run([
  { url: "https://quotes.toscrape.com/", label: "html" },
  { url: "https://quotes.toscrape.com/js/", label: "js" },
  { url: "https://quotes.toscrape.com/scroll", label: "scroll" },
]);
console.log(`${requestsFinished} pages succeeded, ${requestsFailed} failed`);
const { items } = await crawler.getData();
for (const quote of items) console.log(`[${quote.version}] ${quote.text} (${quote.author})`);
