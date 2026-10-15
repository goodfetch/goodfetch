// Scenario 07 on Crawlee 3.18.2: the scenario 05 crawl with resume and retry.
//
// Usage: node 07-resume-and-retry.crawlee.js <crawl-name>
//
// Everything a run needs is stored under the crawl name, so a later run with the same name continues:
//   <name>            request queue of the crawl; handled requests are never fetched again
//   <name>-failures   key-value store, one record per request that failed (in the crawl or in a retry)
//   <name>-retry      request queue of the failed requests being retried; each retry round has new uniqueKeys
// The books dataset is BOOKS, the name the scenario's router writes to. A book URL is recorded once,
// even when its request ran again because a killed run had pushed the book but not yet marked it handled.
//
// Each run does two phases and says why a phase did nothing: the crawl (resumed, if unfinished), then a
// retry of only the requests that failed. While a phase runs, a status line shows the requests in flight
// and the requests waiting in the queue, with the reason they wait.
import { createHash } from "node:crypto";
import { CheerioCrawler, Dataset, KeyValueStore, RequestQueue } from "crawlee";
import type { Request as CrawleeRequest } from "crawlee";
import { BOOKS, router, startUrls } from "./05-two-page-crawl.crawlee.js";

const STATUS_EVERY_MS = 5_000;
const MAX_LISTED_IN_FLIGHT = 10;

interface FailureRecord {
  url: string;
  uniqueKey: string;
  userData: CrawleeRequest["userData"];
  reason: string;
}

type Phase = "crawl" | "retry";
type Pool = NonNullable<CheerioCrawler["autoscaledPool"]>;

function failureKey(uniqueKey: string): string {
  return `failed-${createHash("sha256").update(uniqueKey).digest("hex")}`;
}

// The retry round of a request: 0 for the crawl, n for the n-th retry.
function roundOf(uniqueKey: string): number {
  return Number(/#retry(\d+)$/.exec(uniqueKey)?.[1] ?? 0);
}

// The book URL a pushed record carries, if it is a single record with a url.
function bookUrl(record: unknown): string | undefined {
  if (typeof record !== "object" || record === null || Array.isArray(record)) return undefined;
  const url = (record as { url?: unknown }).url;
  return typeof url === "string" ? url : undefined;
}

function explainWaiting(waiting: number, pool: Pool | undefined): string {
  if (waiting === 0) return "nothing is queued behind them, so the phase ends when they finish";
  if (pool === undefined) return "the crawler has not started its pool yet";
  if (pool.currentConcurrency >= pool.desiredConcurrency) {
    return `all ${pool.desiredConcurrency} concurrent slots are busy, so the rest wait for one to free up`;
  }
  return "the pool has free slots and is starting the next queued request";
}

async function runPhase(
  phase: Phase,
  queue: RequestQueue,
  failures: KeyValueStore,
  bookUrls: Set<string>,
): Promise<{ requestsFinished: number; requestsFailed: number }> {
  const inFlight = new Map<string, string>();
  const crawler = new CheerioCrawler({
    requestQueue: queue,
    requestHandler: async (context) => {
      const { uniqueKey, url } = context.request;
      inFlight.set(uniqueKey, url);
      // Record each book once: skip a push whose URL is already in the dataset.
      const push = context.pushData.bind(context);
      context.pushData = async (data, datasetIdOrName) => {
        const recorded = bookUrl(data);
        if (datasetIdOrName !== BOOKS || recorded === undefined) return push(data, datasetIdOrName);
        if (bookUrls.has(recorded)) return;
        await push(data, datasetIdOrName);
        bookUrls.add(recorded);
      };
      try {
        await router(context);
      } finally {
        inFlight.delete(uniqueKey);
      }
    },
    failedRequestHandler: async ({ request }, error) => {
      console.log(`failed ${request.url}: ${error.message}`);
      const record: FailureRecord = {
        url: request.url,
        uniqueKey: request.uniqueKey,
        userData: request.userData,
        reason: error.message,
      };
      await failures.setValue(failureKey(request.uniqueKey), record);
    },
  });

  const status = setInterval(() => {
    queue
      .getInfo()
      .then((info) => {
        const waiting = info?.pendingRequestCount ?? 0;
        const urls = [...inFlight.values()];
        const listed = urls.slice(0, MAX_LISTED_IN_FLIGHT).join(", ");
        const more = urls.length > MAX_LISTED_IN_FLIGHT ? ` and ${urls.length - MAX_LISTED_IN_FLIGHT} more` : "";
        console.log(
          `[${phase}] in flight ${urls.length}: ${listed || "none"}${more} | waiting ${waiting}: ${explainWaiting(waiting, crawler.autoscaledPool)}`,
        );
      })
      .catch((error: unknown) => console.log(`[${phase}] status unavailable: ${String(error)}`));
  }, STATUS_EVERY_MS);

  try {
    const stats = await crawler.run(phase === "crawl" ? startUrls : undefined);
    return { requestsFinished: stats.requestsFinished, requestsFailed: stats.requestsFailed };
  } finally {
    clearInterval(status);
  }
}

// Moves every stored failure into the retry queue under a new uniqueKey, so a request that fails again
// can be queued again. Adding is idempotent for a given round, so a killed run can repeat this step safely.
async function queueFailuresForRetry(retryQueue: RequestQueue, failures: KeyValueStore): Promise<number> {
  const keys: string[] = [];
  for await (const key of failures.keys()) keys.push(key);

  let queued = 0;
  for (const key of keys) {
    const record = await failures.getValue<FailureRecord>(key);
    if (record === null) continue;
    await retryQueue.addRequest({
      url: record.url,
      uniqueKey: `${record.url}#retry${roundOf(record.uniqueKey) + 1}`,
      userData: record.userData,
    });
    await failures.setValue(key, null);
    queued += 1;
  }
  return queued;
}

async function main(): Promise<void> {
  const name = process.argv[2];
  if (!name || !/^[A-Za-z0-9-]+$/.test(name)) {
    console.error("usage: node 07-resume-and-retry.crawlee.js <crawl-name>  (letters, digits and dashes)");
    process.exitCode = 2;
    return;
  }

  const queue = await RequestQueue.open(name);
  const retryQueue = await RequestQueue.open(`${name}-retry`);
  const failures = await KeyValueStore.open(`${name}-failures`);
  const books = await Dataset.open(BOOKS);

  const bookUrls = new Set<string>();
  await books.forEach((item) => {
    const recorded = bookUrl(item);
    if (recorded !== undefined) bookUrls.add(recorded);
  });

  let didSomething = false;

  // Phase 1: the crawl, resumed where it stopped.
  const crawlInfo = await queue.getInfo();
  if (crawlInfo && crawlInfo.totalRequestCount > 0 && crawlInfo.pendingRequestCount === 0) {
    console.log(
      `Nothing crawled: crawl "${name}" has no request waiting; all ${crawlInfo.handledRequestCount} requests it holds are already handled, so there is nothing left to fetch.`,
    );
  } else {
    if (crawlInfo && crawlInfo.totalRequestCount > 0) {
      console.log(
        `Resuming crawl "${name}": ${crawlInfo.pendingRequestCount} requests waiting, ${crawlInfo.handledRequestCount} already handled.`,
      );
    } else {
      console.log(`Starting crawl "${name}".`);
    }
    const stats = await runPhase("crawl", queue, failures, bookUrls);
    didSomething = true;
    console.log(`Crawl "${name}": ${stats.requestsFinished} finished, ${stats.requestsFailed} failed in this run.`);
  }

  // Phase 2: retry only the requests that failed.
  const queued = await queueFailuresForRetry(retryQueue, failures);
  const retryInfo = await retryQueue.getInfo();
  const retryWaiting = retryInfo?.pendingRequestCount ?? 0;
  if (retryWaiting === 0) {
    console.log(
      `Nothing retried: crawl "${name}" has no failed request waiting for a retry, so no request was run again.`,
    );
  } else {
    console.log(`Retrying ${retryWaiting} failed request(s) of crawl "${name}" (${queued} newly queued).`);
    const stats = await runPhase("retry", retryQueue, failures, bookUrls);
    didSomething = true;
    console.log(`Retry "${name}": ${stats.requestsFinished} finished, ${stats.requestsFailed} failed again.`);
  }

  await books.exportToJSON(BOOKS);
  const itemCount = (await books.getInfo())?.itemCount ?? 0;
  console.log(`Dataset "${BOOKS}": ${itemCount} books, ${bookUrls.size} distinct book URLs.`);
  if (!didSomething) console.log("Nothing was done in this run.");
}

await main();
