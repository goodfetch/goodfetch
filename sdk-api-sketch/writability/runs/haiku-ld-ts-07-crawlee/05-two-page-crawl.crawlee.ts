// Golden scenario 05 on Crawlee 3.18.2: a CheerioCrawler with a router whose labels validate their
// `userData` with a schema (Crawlee 3.18's route schemas). Scenario 07's program imports the router;
// this one runs only as the entry point, as Crawlee's template splits routes.ts from main.ts.
import { CheerioCrawler, Dataset, createCheerioRouter } from "crawlee";
import { z } from "zod";

export const BOOKS = "books";
export const startUrls = ["https://books.toscrape.com/"];
const RATINGS = ["One", "Two", "Three", "Four", "Five"];

// Pages a handler chose not to record: Crawlee counts them as finished.
export const skipped: { url: string; reason: string }[] = [];

export const router = createCheerioRouter({
  CATEGORY: z.object({ category: z.string().min(1) }),
  BOOK: z.object({ category: z.string().min(1) }),
});

router.addDefaultHandler(async ({ $, request, addRequests }) => {
  // The nested list leaves out the top-level "Books"; each link's text travels in its userData.
  const links = $(".side_categories ul ul a").toArray();
  await addRequests(
    links.map((a) => ({
      url: new URL($(a).attr("href") ?? "", request.loadedUrl).href,
      label: "CATEGORY",
      userData: { category: $(a).text().trim() },
    })),
  );
});

router.addHandler("CATEGORY", async ({ request, enqueueLinks }) => {
  const { category } = request.userData;
  // enqueueLinks keeps to the same hostname by default.
  await enqueueLinks({
    selector: "article.product_pod h3 a",
    label: "BOOK",
    userData: { category },
  });
  await enqueueLinks({ selector: "li.next a", label: "CATEGORY", userData: { category } });
});

router.addHandler("BOOK", async ({ $, request, pushData }) => {
  const main = $(".product_main");
  const price = /£(\d+\.\d{2})/.exec(main.find(".price_color").text())?.[1];
  const inStock = /\((\d+) available\)/.exec(main.find(".availability").text())?.[1];
  const rating = RATINGS.findIndex((word) => main.find(".star-rating").hasClass(word)) + 1;
  if (price === undefined || inStock === undefined || rating === 0) {
    skipped.push({ url: request.url, reason: "handler: no price, stock or rating on the page" });
    return;
  }
  await pushData(
    {
      title: main.find("h1").text(),
      price,
      inStock: Number(inStock),
      rating,
      url: request.loadedUrl,
      category: request.userData.category,
    },
    BOOKS,
  );
});

if (import.meta.main) {
  const failed: { url: string; reason: string }[] = [];
  const crawler = new CheerioCrawler({
    requestHandler: router,
    failedRequestHandler: ({ request }, error) => {
      failed.push({ url: request.url, reason: error.message });
    },
    onSkippedRequest: ({ url, reason }) => {
      skipped.push({ url, reason });
    },
  });
  const stats = await crawler.run(startUrls);
  // Written to ./storage/key_value_stores/default/books.json.
  await (await Dataset.open(BOOKS)).exportToJSON(BOOKS);

  for (const { url, reason } of failed) console.log(`failed ${url}: ${reason}`);
  for (const { url, reason } of skipped) console.log(`skipped ${url}: ${reason}`);
  const skippedByHandler = skipped.filter((s) => s.reason.startsWith("handler")).length;
  const succeeded = stats.requestsFinished - skippedByHandler;
  console.log(`${succeeded} succeeded, ${stats.requestsFailed} failed, ${skipped.length} skipped`);
}
