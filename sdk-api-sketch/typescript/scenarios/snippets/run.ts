// Snippet for The Run in code (#478, ADR 0102), on golden scenario 07 with scenario 05's crawl.
import { Goodfetch, seed, type Crawl, type CrawlRun, type CrawlRunReport, type Dataset, type Label, type Logger, type Run } from "goodfetch";

declare const log: Logger;
// Scenario 05's declaration (crawl.ts): Labels `home`, `category` and `book`, one handler each.
declare const books: Crawl;
declare const home: Label<undefined>;
declare const bookRecords: Dataset<{ readonly title: string; readonly price: string }>;

// Scenario 07: one program, run again under the same ID after a kill, then with `--retry-failed`,
// then once more with nothing left to do.
export async function scenario07(argv: readonly string[]): Promise<CrawlRunReport> {
  await using gf = await Goodfetch.connect({ logger: log });
  await using run = await gf.foxhound.startCrawl(books, {
    id: "books-2026-10-15", // a start under this ID resumes the Run where it stopped
    seeds: [seed("https://books.toscrape.com/", home)],
  });
  if (run.startedAs === "resumed") log.info("resuming", { run: run.id });
  for (const answer of run.seeds) if (!answer.added) log.warn("seed not added", { url: answer.url, reason: answer.reason });
  if (argv.includes("--retry-failed")) {
    const readmitted = await run.retryFailed(); // only `failed`, in this same Run and its Dataset
    log.info("retrying", { readmitted });
  }
  const report = await run.finish(); // lets the crawl finish; at once if it had already ended
  if (report.nothingDone !== null) log.warn("nothing done", { reason: report.nothingDone.reason });
  return report;
}

// While it runs: what is in flight, what waits, and why.
export async function follow(run: CrawlRun): Promise<void> {
  for await (const progress of run.progress()) {
    const { running, limit } = progress.concurrency.handlers;
    log.info("progress", { pending: progress.requests.pending, inFlight: progress.inFlight.length, handlers: `${running} of ${String(limit.value)}` });
    for (const wait of progress.waiting) {
      log.info("waiting", { reason: wait.reason, requests: wait.requests, setting: wait.setting?.setting, domains: wait.domains.map((d) => d.domain) });
    }
  }
}

// One value: leave the loop.
export async function now(run: Run): Promise<void> {
  for await (const progress of run.progress()) {
    log.info("now", { lifecycle: progress.lifecycle, pauses: progress.pauses.length });
    break;
  }
}

// Failures as they happen, in place of a failed-request callback (ADR 0093's R4).
export async function onFailure(run: CrawlRun): Promise<void> {
  for await (const outcome of run.outcomes({ kinds: ["failed", "blocked"], after: "now" })) {
    log.warn("lost", { url: outcome.request.url, kind: outcome.kind });
  }
}

// The failed URLs, from the Run's committed state, also once it has ended.
export async function failedUrls(run: CrawlRun): Promise<string[]> {
  const urls: string[] = [];
  for await (const outcome of run.outcomes({ kinds: ["failed"] })) urls.push(outcome.request.url);
  return urls;
}

// Yesterday's crawl, read by its ID from another program: nothing resumed, nothing driven.
export async function exportYesterday(gf: Goodfetch): Promise<void> {
  await using view = await gf.foxhound.openCrawlRun("books-2026-10-14");
  const report = await view.report(); // at once: it has ended
  log.info("books", { committed: report.records["books"]?.committed, sinks: report.sinks });
  const csv = await view.export(bookRecords, { format: "csv" });
  await csv.saveAs("books.csv");
}

// A Collie Run under an ID: run again after a crash, it finds its Sessions and its Effect journal, so an
// order placed before the crash is checked rather than placed again; once finished, it runs nothing.
export async function placeOrder(orderId: string): Promise<void> {
  await using gf = await Goodfetch.connect({ logger: log });
  await using run = await gf.collie.start({ id: `order-${orderId}` });
  if (run.startedAs === "ended") return;
  await run.setProgressMessage(`order ${orderId}`);
  const page = await run.newPage();
  await page.goto("https://shop.example/cart");
  await page.act("place the order");
  const report = await run.finish(); // `finished` and `complete`; the Verdict comes from its checks
  log.info("ordered", { verdict: report.verdict, price: report.usage.price, weakened: report.weakened.map((w) => w.setting) });
}

// A keep-alive crawl fed while it runs, finished by the program (ADR 0037's RT-12).
export async function fed(urls: AsyncIterable<string>): Promise<void> {
  await using gf = await Goodfetch.connect({ logger: log });
  await using run = await gf.foxhound.startCrawl(books, {
    seeds: [seed("https://books.toscrape.com/", home)],
    settings: { crawl: { keepAlive: true }, checkpoint: { interval: 10 * 60_000 } },
  });
  for await (const url of urls) await run.addSeed(url, home);
  const checkpoint = await run.checkpoint(); // where the engine keeps the Run's, on its own machine
  await checkpoint.saveAs("/mnt/backup/books.checkpoint"); // copied here, wherever the engine runs
  log.info("checkpoint", { digest: checkpoint.digest, size: checkpoint.size });
  await run.finish(); // ends once what is pending is done
}

// Typed mistakes, caught before anything runs.
export async function mistakes(gf: Goodfetch, run: CrawlRun): Promise<void> {
  // @ts-expect-error a crawl's start needs its seeds, even to resume: they are an input that must match
  await gf.foxhound.startCrawl(books, { id: "books-2026-10-15" });
  // @ts-expect-error `detached` is only for Runs with no handlers in the program
  await gf.collie.start({ detached: true });
  // @ts-expect-error a skip is a decision: only failed and blocked requests are re-admitted
  await run.retryFailed({ skipped: true });
}
