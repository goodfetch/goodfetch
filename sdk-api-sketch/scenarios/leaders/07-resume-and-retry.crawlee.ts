// Golden scenario 07 on Crawlee 3.18.2: scenario 05's router over a request queue named by the
// program, in storage kept between runs (`purgeOnStart: false`). Stop it with Ctrl+C and run it
// again to resume; once crawled, it re-adds the failed requests once; a third run finds the queue
// handled and says so.
import { writeFile } from "node:fs/promises";
import { CheerioCrawler, Configuration, Dataset, KeyValueStore, RequestQueue } from "crawlee";
import { BOOKS, router, startUrls } from "./05-two-page-crawl.crawlee.js";

const NAME = "books-toscrape-2026-10-15";
const config = new Configuration({ purgeOnStart: false });
const requestQueue = await RequestQueue.open(NAME, { config });
const state = await KeyValueStore.open(NAME, { config });
type Failed = { url: string; label: string | undefined; userData: Record<string, unknown> };
const failed = (await state.getValue<Failed[]>("failed")) ?? [];

const crawler = new CheerioCrawler(
  {
    requestQueue,
    requestHandler: router,
    failedRequestHandler: async ({ request }, error) => {
      console.log(`failed ${request.url}: ${error.message}`);
      failed.push({ url: request.url, label: request.label, userData: request.userData });
      await state.setValue("failed", failed);
    },
    // What is in flight and what waits, every 5 s; Crawlee does not say why a request waits.
    statusMessageLoggingInterval: 5,
    statusMessageCallback: async ({ crawler, state: stats }) => {
      const info = await requestQueue.getInfo();
      const pool = crawler.autoscaledPool;
      console.log(
        `${info?.pendingRequestCount ?? 0} pending, ${pool?.currentConcurrency ?? 0} in flight ` +
          `(desired ${pool?.desiredConcurrency ?? 0}), ${stats.requestsFinished} finished, ${stats.requestsFailed} failed`,
      );
    },
  },
  config,
);

const stats = await crawler.run(startUrls);
if (stats.requestsTotal === 0) {
  console.log(`nothing done: every request in queue ${NAME} was already handled`);
} else if (failed.length > 0 && (await state.getValue<boolean>("retried")) !== true) {
  await state.setValue("retried", true);
  // A failed request stays handled under its uniqueKey: only a new key queues it again.
  const retries = failed.splice(0).map((f) => ({ ...f, uniqueKey: `${f.url}#retry` }));
  await state.setValue("failed", failed);
  await crawler.run(retries, { purgeRequestQueue: false });
}

// pushData wrote each book at once, so a request re-run after a stop may have written it twice.
const books = await (await Dataset.open<{ url: string }>(BOOKS, { config })).getData();
const unique = [...new Map(books.items.map((book) => [book.url, book])).values()];
await writeFile("books.json", JSON.stringify(unique, null, 2));
console.log(`exported ${unique.length} books to books.json`);
