/// <reference types="node" />
// Scenario 07: the scenario 05 crawl under a fixed Run name. Each start with the name resumes the Run
// where it stopped, or returns it as it is when it has ended. A start retries only the requests that
// failed, then says what it did and why when nothing was left. SIGINT and SIGTERM stop the Run with a
// Checkpoint, so the next start with the name resumes it; a SIGKILL is an engine-detected loss and also
// resumes, counted against the crawl's retry maximum.
import { Foxhound } from "@goodfetch/foxhound";
import type {
  CrawlRequestOutcome,
  CrawlRunProgress,
  CrawlRunReport,
  NothingDone,
} from "@goodfetch/foxhound";
import { books, crawl, seeds } from "./05-two-page-crawl.js";

const RUN_ID = "books-two-page-crawl";
const PROGRESS_EVERY_MS = 2000;

function explainNothingDone(nothing: NothingDone): string {
  switch (nothing.reason) {
    case "already_ended":
      return `run "${RUN_ID}" had already ended at ${nothing.endedAt.toISOString()}, so it was returned as it is and no request ran`;
    case "nothing_pending":
      return "no request was pending or left to retry, so the resume and the retry had nothing to do";
    case "no_seed_handled":
      return `no seed reached a handler (refused ${JSON.stringify(nothing.refused)}, skipped ${JSON.stringify(nothing.skipped)})`;
    case "unknown":
      return `the engine reported "${nothing.raw}"`;
  }
}

function printProgress(progress: CrawlRunProgress): void {
  const { requests } = progress;
  console.log(
    `progress ${progress.lifecycle}: ${requests.pending} pending, ${requests.fetching + requests.handling} in flight, ` +
      `${requests.paused} paused, ${requests.succeeded} succeeded, ${requests.failed} failed, ` +
      `${requests.skipped} skipped, ${requests.blocked} blocked`,
  );
  for (const item of progress.inFlight) {
    const waiting = item.waiting === null ? "" : `, waiting for ${item.waiting.reason}`;
    console.log(`  in flight: ${item.request.url} (${item.phase}, attempt ${item.attempt}${waiting})`);
  }
  for (const group of progress.waiting) {
    const domains = group.domains.map(
      (domain) =>
        `${domain.domain} ${domain.requests}${domain.until === null ? "" : ` until ${domain.until.toISOString()}`}`,
    );
    const more = group.moreDomains > 0 ? ` and ${group.moreDomains} more` : "";
    console.log(
      `  waiting: ${group.requests} for ${group.reason} on ${domains.join(", ") || "no listed domain"}${more}`,
    );
  }
}

export async function main(): Promise<void> {
  await using foxhound = await Foxhound.connect();
  await using run = await foxhound.startCrawl(crawl, { id: RUN_ID, seeds });
  console.log(`run "${RUN_ID}" started as ${run.startedAs()}`);

  // A signal closes the Run with a Checkpoint before the process exits; the finish below then waits for it.
  let stopping: Promise<void> | undefined;
  const stopOnSignal = (signal: NodeJS.Signals): void => {
    stopping ??= (async () => {
      console.log(
        `${signal}: stopping the Run with a Checkpoint; start the program with the name "${RUN_ID}" to resume it`,
      );
      await run.stop().catch((error: unknown) => console.error("stop failed:", error));
      process.exit(130);
    })();
  };
  process.once("SIGINT", stopOnSignal);
  process.once("SIGTERM", stopOnSignal);

  const finishRun = async (): Promise<CrawlRunReport> => {
    const report = await run.finish().catch(async (error: unknown): Promise<never> => {
      await stopping;
      throw error;
    });
    await stopping;
    return report;
  };

  // Progress is printed while the crawl runs: what is in flight, and what waits and why.
  let finished = false;
  let lastPrinted = 0;
  void (async () => {
    for await (const progress of run.progress()) {
      if (finished) break;
      const now = Date.now();
      if (now - lastPrinted < PROGRESS_EVERY_MS) continue;
      lastPrinted = now;
      printProgress(progress);
    }
  })().catch((error: unknown) => console.error("progress stopped:", error));

  // The crawl continues where it stopped; then only the failed requests are retried. A Run that had
  // already ended has no new work, so its failed requests are reported, not retried.
  let report = await finishRun();
  if (report.requests.failed > 0 && run.startedAs() !== "ended") {
    const readmitted = await run.retryFailed();
    console.log(`retrying ${readmitted} failed request(s); skipped and blocked requests are not retried`);
    report = await finishRun();
  } else if (report.requests.failed > 0) {
    console.log(`${report.requests.failed} request(s) failed and are not retried: the Run had already ended`);
  }
  finished = true;

  // The latest outcome of each request, so a retried request shows how it finally ended.
  const latest = new Map<string, CrawlRequestOutcome>();
  for await (const outcome of run.outcomes()) latest.set(outcome.request.id, outcome);
  for (const outcome of latest.values()) {
    const { url } = outcome.request;
    if (outcome.kind === "failed")
      console.log(`failed ${url}: ${outcome.error.code}: ${outcome.error.message}`);
    else if (outcome.kind === "skipped")
      console.log(`skipped ${url}: ${outcome.reason} ${outcome.message ?? ""}`);
    else if (outcome.kind === "blocked") console.log(`blocked ${url}: ${outcome.blockCause}`);
  }

  const exported = await run.export(books, { format: "json" });
  await exported.saveAs("books.json");
  console.log(`exported ${exported.records()} books to books.json`);

  // Every book exactly once: each committed Record is read back and counted by its URL.
  const copies = new Map<string, number>();
  let total = 0;
  for await (const { value } of run.records(books)) {
    total += 1;
    copies.set(value.url, (copies.get(value.url) ?? 0) + 1);
  }
  const duplicates = [...copies].filter(([, count]) => count > 1);
  console.log(
    `dataset "${books.name}": ${total} records, ${copies.size} distinct books` +
      (duplicates.length === 0 ? ", each exactly once" : ""),
  );
  for (const [url, count] of duplicates) console.log(`duplicate ${url}: ${count} records`);
  if (duplicates.length > 0) process.exitCode = 1;

  const { succeeded, failed, skipped, blocked } = report.requests;
  console.log(`${succeeded} succeeded, ${failed} failed, ${skipped} skipped, ${blocked} blocked`);
  const nothingDone: NothingDone | null =
    report.nothingDone ??
    (run.startedAs() === "ended" ? { reason: "already_ended", endedAt: report.endedAt } : null);
  if (nothingDone !== null) console.log(`nothing done: ${explainNothingDone(nothingDone)}`);
}

if (import.meta.main) await main();
