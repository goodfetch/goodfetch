// Golden scenario 07 on Crawlee 3.18.2: resume a killed crawl, retry only its failures, and say so
// when there is nothing left to do. Usage: node 07-resume-and-retry.crawlee.ts [crawl-name]
//
// State lives in named storages (they are not purged on start, unlike the default ones):
//   - a RequestQueue called <name>: which requests are handled, which are still waiting; a request
//     in flight when the process died is not marked handled, so it is fetched again on restart.
//   - a KeyValueStore called <name>: the "failed" record, the requests that ran out of retries.
//   - the "books" Dataset of scenario 05: a request killed after its pushData but before it was marked
//     handled runs again, so the dataset is deduplicated by book URL after every crawl.
import { CheerioCrawler, Dataset, KeyValueStore, RequestQueue } from "crawlee";
import { BOOKS, router, skipped, startUrls } from "./05-two-page-crawl.crawlee.js";

const name = process.argv[2] ?? process.env["CRAWL_NAME"] ?? "books-crawl";
const STATUS_EVERY_MS = 5000;
const RETRY_SUFFIX = "#retry";

type FailedEntry = {
  uniqueKey: string;
  url: string;
  label?: string;
  userData: Record<string, unknown>;
  reason: string;
  // 0: never retried yet; 1: the retry was queued (and has failed again if the entry is still here).
  retries: number;
};
type FailedRecord = Record<string, FailedEntry>;

type Queue = Awaited<ReturnType<typeof RequestQueue.open>>;
type InFlight = { url: string; label: string | undefined; attempt: number; since: number };

const queue: Queue = await RequestQueue.open(name);
const store = await KeyValueStore.open(name);
const failed: FailedRecord = (await store.getValue<FailedRecord>("failed")) ?? {};

let saving: Promise<void> = Promise.resolve();
const saveFailed = (): Promise<void> => {
  saving = saving.then(() => store.setValue("failed", failed));
  return saving;
};

let interrupted = false;
process.once("SIGINT", () => {
  interrupted = true;
  process.exitCode = 130;
});

const counts = async () => {
  const info = await queue.getInfo();
  return {
    total: info?.totalRequestCount ?? 0,
    handled: info?.handledRequestCount ?? 0,
    pending: info?.pendingRequestCount ?? 0,
  };
};

// One crawl over whatever the queue holds (plus `seed`, which the queue deduplicates by unique key).
async function crawlOnce(phase: string, seed?: string[]): Promise<void> {
  const inFlight = new Map<string, InFlight>();

  const crawler = new CheerioCrawler({
    requestQueue: queue,
    requestHandler: async (context) => {
      const { request } = context;
      inFlight.set(request.uniqueKey, {
        url: request.url,
        label: request.label,
        attempt: request.retryCount + 1,
        since: Date.now(),
      });
      try {
        await router(context);
      } finally {
        inFlight.delete(request.uniqueKey);
      }
      // A retried request that now succeeded is no longer a failure.
      if (request.uniqueKey.endsWith(RETRY_SUFFIX)) {
        delete failed[request.uniqueKey.slice(0, -RETRY_SUFFIX.length)];
        await saveFailed();
      }
    },
    failedRequestHandler: async ({ request }, error) => {
      inFlight.delete(request.uniqueKey);
      const key = request.uniqueKey.endsWith(RETRY_SUFFIX)
        ? request.uniqueKey.slice(0, -RETRY_SUFFIX.length)
        : request.uniqueKey;
      const previous = failed[key];
      failed[key] = {
        uniqueKey: key,
        url: request.url,
        ...(request.label !== undefined ? { label: request.label } : {}),
        userData: { ...request.userData },
        reason: error.message,
        retries: previous?.retries ?? 0,
      };
      await saveFailed();
    },
    onSkippedRequest: ({ url, reason }) => {
      skipped.push({ url, reason });
    },
  });

  // What is in flight, what is waiting, and why it waits.
  const describe = async (): Promise<string> => {
    const { pending, handled } = await counts();
    const pool = crawler.autoscaledPool;
    const running = [...inFlight.values()];
    const waiting = Math.max(0, pending - running.length);
    const lines = [
      `[${phase}] ${new Date().toISOString().slice(11, 19)} handled ${handled}, ` +
        `in flight ${running.length}, waiting ${waiting}`,
    ];
    for (const r of running.slice(0, 10)) {
      const age = ((Date.now() - r.since) / 1000).toFixed(1);
      lines.push(`  in flight: ${r.label ?? "start"} ${r.url} (attempt ${r.attempt}, ${age}s)`);
    }
    if (running.length > 10) lines.push(`  in flight: ... and ${running.length - 10} more`);
    if (waiting > 0 && pool) {
      const { currentConcurrency: current, desiredConcurrency: desired, maxConcurrency: max } = pool;
      const why =
        current >= desired
          ? `all ${desired} desired slots are busy (maximum ${max}); the autoscaler holds concurrency until CPU, memory and event loop stay idle`
          : `only ${current} of ${desired} desired slots are used: the pool is ramping up or the queue head is not ready yet`;
      lines.push(`  waiting ${waiting}: ${why}`);
    } else if (waiting > 0) {
      lines.push(`  waiting ${waiting}: the crawler has not started its pool yet`);
    }
    const retrying = running.filter((r) => r.attempt > 1).length;
    if (retrying > 0) {
      lines.push(`  ${retrying} in flight are retries of a request that failed before (Crawlee retries up to 3 times)`);
    }
    return lines.join("\n");
  };

  const report = () => {
    describe().then(
      (text) => console.log(text),
      (error: unknown) => console.log(`[${phase}] status unavailable: ${String(error)}`),
    );
  };
  const timer = setInterval(report, STATUS_EVERY_MS);
  timer.unref();
  process.on("SIGUSR1", report);

  try {
    await crawler.run(seed, { purgeRequestQueue: false });
  } finally {
    clearInterval(timer);
    process.off("SIGUSR1", report);
  }
}

// Every book exactly once: drop the second and later copies of a URL, keeping the first.
async function dedupeBooks(): Promise<{ kept: number; removed: number }> {
  const dataset = await Dataset.open(BOOKS);
  const seen = new Set<string>();
  const unique: Record<string, unknown>[] = [];
  let total = 0;
  await dataset.forEach((item) => {
    total++;
    const id = typeof item["url"] === "string" ? item["url"] : JSON.stringify(item);
    if (seen.has(id)) return;
    seen.add(id);
    unique.push(item);
  });
  if (unique.length === total) return { kept: total, removed: 0 };
  await dataset.drop();
  const fresh = await Dataset.open(BOOKS);
  await fresh.pushData(unique);
  return { kept: unique.length, removed: total - unique.length };
}

async function main(): Promise<void> {
  const before = await counts();
  const unretried = Object.values(failed).filter((f) => f.retries === 0);
  const exhausted = Object.values(failed).filter((f) => f.retries > 0);
  const seeded = before.total > 0;

  if (seeded && before.pending === 0 && unretried.length === 0) {
    console.log(`Nothing was done for crawl "${name}".`);
    console.log(
      `Why: its queue holds ${before.total} requests, ${before.handled} handled and 0 waiting, so the crawl is complete; ` +
        `no recorded failure is left to retry.`,
    );
    if (exhausted.length > 0) {
      console.log(`${exhausted.length} requests still failed after their one retry and are not tried again:`);
      for (const f of exhausted) console.log(`  failed ${f.url}: ${f.reason}`);
    } else {
      console.log("No request failed in this crawl.");
    }
    const books = await dedupeBooks();
    console.log(`The "${BOOKS}" dataset holds ${books.kept} books, each once.`);
    return;
  }

  // Phase 1: the crawl itself, fresh or resumed from the requests still waiting in the queue.
  if (!seeded || before.pending > 0) {
    console.log(
      seeded
        ? `Resuming crawl "${name}": ${before.handled} requests handled, ${before.pending} still waiting.`
        : `Starting crawl "${name}".`,
    );
    await crawlOnce("crawl", seeded ? undefined : startUrls);
    if (interrupted) {
      console.log(`Interrupted: run again with the name "${name}" to continue.`);
      return;
    }
  }

  // Phase 2: retry only the requests that failed, once, under a new unique key
  // (a failed request counts as handled, so the queue would drop it as a duplicate).
  const toRetry = Object.values(failed).filter((f) => f.retries === 0);
  if (toRetry.length > 0 && (await counts()).pending >= 0) {
    console.log(`Retrying the ${toRetry.length} requests that failed in crawl "${name}".`);
    for (const f of toRetry) {
      await queue.addRequest({
        url: f.url,
        uniqueKey: f.uniqueKey + RETRY_SUFFIX,
        ...(f.label !== undefined ? { label: f.label } : {}),
        userData: f.userData,
      });
      f.retries = 1;
    }
    await saveFailed();
    await crawlOnce("retry");
    if (interrupted) {
      console.log(`Interrupted during the retry: run again with the name "${name}" to continue.`);
      return;
    }
  }

  const books = await dedupeBooks();
  const dataset = await Dataset.open(BOOKS);
  // Written to ./storage/key_value_stores/default/books.json.
  await dataset.exportToJSON(BOOKS);

  const after = await counts();
  const stillFailed = Object.values(failed);
  for (const f of stillFailed) console.log(`failed ${f.url}: ${f.reason}`);
  for (const { url, reason } of skipped) console.log(`skipped ${url}: ${reason}`);
  console.log(
    `crawl "${name}": ${after.handled} requests handled, ${after.pending} waiting, ` +
      `${stillFailed.length} failed after retry, ${skipped.length} skipped; ` +
      `${books.kept} books in the dataset${books.removed > 0 ? ` (${books.removed} duplicates removed)` : ""}`,
  );
}

await main();
