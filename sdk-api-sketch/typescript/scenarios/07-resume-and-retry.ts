/// <reference types="node" />
// Golden scenario 07: the crawl of scenario 05 under an ID of the program's choosing. Run it, kill
// it, run it again: the start resumes the Run; once crawled, it retries only the failed requests;
// a third run finds the Run ended and says so.
import { Foxhound, type CrawlRun } from "@goodfetch/foxhound";
import { books, crawl, seeds } from "./05-two-page-crawl.js";

const RUN_ID = "books-toscrape-2026-10-15";

// What is in flight and what waits, and why, every five seconds at most.
async function printProgress(run: CrawlRun): Promise<void> {
  let printedAt = 0;
  for await (const progress of run.progress()) {
    if (Date.now() - printedAt < 5_000) continue;
    printedAt = Date.now();
    const { pending, succeeded, failed, skipped } = progress.requests;
    console.log(`${pending} pending, ${succeeded} succeeded, ${failed} failed, ${skipped} skipped`);
    for (const { request, phase, tier, attempt } of progress.inFlight) {
      console.log(`  in flight: ${request.url} (${phase} over ${tier}, attempt ${attempt})`);
    }
    for (const { reason, requests, setting } of progress.waiting) {
      console.log(
        `  waiting: ${requests} for ${reason}${setting ? ` (${setting.setting} = ${JSON.stringify(setting.value)})` : ""}`,
      );
    }
  }
}

async function main(): Promise<void> {
  await using foxhound = await Foxhound.connect();
  // The same ID begins the Run, resumes it after a kill, or returns it ended. A changed program
  // (seeds, a Label gone, a Payload schema changed) or a damaged run file throws a typed error.
  await using run = await foxhound.startCrawl(crawl, { id: RUN_ID, seeds });
  console.log(`run ${run.id()}: ${run.startedAs()}`);
  // Node's Ctrl+C would end the process without disposing: stop instead, so the requests in flight
  // return to pending; `stop()` is bounded by `timeout.close`.
  const stop = (): void => void run.stop().catch((error: unknown) => console.error("stop:", error));
  process.once("SIGINT", stop).once("SIGTERM", stop);

  let [report] = await Promise.all([run.finish(), printProgress(run)]);
  if (report.lifecycle === "finished" && run.startedAs() !== "ended") {
    const readmitted = await run.retryFailed(); // only the failed requests, in this Run and its Dataset
    console.log(`retrying ${readmitted} failed requests`);
    if (readmitted > 0) [report] = await Promise.all([run.finish(), printProgress(run)]);
  }
  if (report.nothingDone !== null) console.log(`nothing done: ${report.nothingDone.reason}`);
  const { succeeded, failed, skipped } = report.requests;
  console.log(`${report.lifecycle}: ${succeeded} succeeded, ${failed} failed, ${skipped} skipped`);
  if (report.lifecycle === "finished") {
    const exported = await run.export(books, { format: "json" }); // each book once: Records commit with their attempt
    await exported.saveAs("books.json");
    console.log(`exported ${exported.records()} books to books.json`);
  }
}

await main();
