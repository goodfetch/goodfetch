/// <reference types="node" />
// Golden scenario 07: resume a crawl after it was killed, retry only its failed requests, and say so
// when there is nothing left to do. It runs the crawl of scenario 05 under a Run ID of our choosing:
//
//   node 07-resume-and-retry.ts [run-id]      (default run ID: "books-crawl-07")
//
// Run it, kill it part-way (Ctrl-C, SIGTERM or even kill -9), and run it again with the same ID: the
// start under an existing ID resumes the Run from its committed state. A Record commits with the
// attempt that pushed it, so a request interrupted mid-handler leaves no half result and the Dataset
// (keyed by `url`) holds every book exactly once. When the crawl has finished, the requests that
// failed are re-admitted once, in the same Run. A third start finds the Run ended and the report says
// what was not done and why.
import { Foxhound } from "@goodfetch/foxhound";
import type { CrawlRun, CrawlRunProgress, CrawlRunReport, NothingDone } from "@goodfetch/foxhound";
import { books, crawl, seeds } from "./05-two-page-crawl.js";

const RUN_ID = process.argv[2] ?? "books-crawl-07";
/** Set in the Run's Key-value store once the failed requests were re-admitted, so a later start does not retry them again. */
const RETRIED_KEY = "retried-failed";
const PROGRESS_EVERY_MS = 2000;

/** What is in flight and what waits, and why (the `waiting` groups carry the reason and the setting behind it). */
function describeProgress(p: CrawlRunProgress): string {
  const r = p.requests;
  const lines = [
    `[progress] ${p.lifecycle}: ${r.fetching} fetching, ${r.handling} handling, ${r.pending} pending, ` +
      `${r.paused} paused, ${r.succeeded} succeeded, ${r.failed} failed, ${r.skipped} skipped, ${r.blocked} blocked`,
  ];
  for (const f of p.inFlight) {
    const waiting = f.waiting === null ? "" : ` (waiting: ${f.waiting.reason})`;
    lines.push(
      `  in flight: ${f.phase} ${f.request.label} ${f.request.url} on ${f.tier}, attempt ${f.attempt}, since ${f.since.toISOString()}${waiting}`,
    );
  }
  for (const g of p.waiting) {
    const domains = g.domains
      .map((d) => `${d.domain} x${d.requests}${d.until === null ? "" : ` until ${d.until.toISOString()}`}`)
      .join(", ");
    const more = g.moreDomains > 0 ? ` and ${g.moreDomains} more domains` : "";
    const setting = g.setting === null ? "" : ` [${g.setting.setting} = ${JSON.stringify(g.setting.value)}]`;
    lines.push(`  waiting: ${g.requests} held by ${g.reason}${setting}: ${domains}${more}`);
  }
  if (p.idle) lines.push("  idle: waiting for seeds");
  return lines.join("\n");
}

/** Prints the progress now and then while the Run works; the returned function stops the printing. */
function watchProgress(run: CrawlRun): () => Promise<void> {
  const progress = run.progress();
  const reading = (async () => {
    let last = 0;
    for await (const p of progress) {
      const now = Date.now();
      if (now - last < PROGRESS_EVERY_MS) continue;
      last = now;
      console.log(describeProgress(p));
    }
  })();
  return async () => {
    progress.close();
    await reading;
  };
}

/** Lets the Run finish while printing its progress. */
async function finishWatched(run: CrawlRun): Promise<CrawlRunReport> {
  const stopWatching = watchProgress(run);
  try {
    return await run.finish();
  } finally {
    await stopWatching();
  }
}

function explainNothingDone(nothing: NothingDone, runId: string): string {
  switch (nothing.reason) {
    case "already_ended":
      return `the Run "${runId}" had already ended on ${nothing.endedAt.toISOString()}: nothing was fetched, and no request failed that is left to retry`;
    case "nothing_pending":
      return `no request of the Run "${runId}" is left pending, and none failed that is left to retry`;
    case "no_seed_handled":
      return `no seed reached a handler (refused: ${JSON.stringify(nothing.refused)}, skipped: ${JSON.stringify(nothing.skipped)})`;
    default:
      return `the engine reported "${nothing.raw}"`;
  }
}

export async function main(): Promise<void> {
  await using foxhound = await Foxhound.connect();
  // The same ID resumes the Run if it exists, returns it ended if it finished, and starts it otherwise.
  // The seeds must match the Run's, which they do: they are scenario 05's.
  await using run = await foxhound.startCrawl(crawl, { id: RUN_ID, seeds });
  console.log(`crawl "${run.id()}": ${run.startedAs()}`);

  // Killed by a signal, the program stops the Run at a safe point with a Checkpoint, resumable by its ID.
  const onSignal = (signal: NodeJS.Signals): void => {
    console.log(`${signal}: stopping the crawl "${run.id()}"; start it again with the same name to resume`);
    void run.close().finally(() => process.exit(130));
  };
  process.once("SIGINT", onSignal);
  process.once("SIGTERM", onSignal);

  let report = await finishWatched(run);

  // Retry only what failed in this crawl (not the blocked ones, not the skips, which are decisions),
  // once, in the same Run: each request keeps its ID, Label, Payload and depth.
  if (run.startedAs() !== "ended" && report.requests.failed > 0 && (await run.keyValueStore.get(RETRIED_KEY)) === null) {
    const readmitted = await run.retryFailed();
    await run.keyValueStore.set(RETRIED_KEY, true);
    console.log(`retrying ${readmitted} failed requests`);
    if (readmitted > 0) report = await finishWatched(run);
  }

  // The Dataset is append-only and keyed by url; count the books to prove each is there exactly once.
  const urls = new Set<string>();
  let total = 0;
  for await (const record of run.records(books)) {
    total += 1;
    urls.add(record.value.url);
  }
  console.log(`${total} records, ${urls.size} distinct books${total === urls.size ? "" : ": DUPLICATES"}`);

  const exported = await run.export(books, { format: "json" });
  await exported.saveAs("books.json");
  console.log(`exported ${exported.records()} books to books.json`);

  // The latest outcome of each request wins: a retried request's new outcome supersedes the old.
  const failed = new Map<string, string>();
  for await (const outcome of run.outcomes({ kinds: ["failed"] })) {
    if (outcome.kind === "failed")
      failed.set(outcome.request.id, `${outcome.request.url}: ${outcome.error.code}: ${outcome.error.message}`);
    else failed.delete(outcome.request.id);
  }
  for (const line of failed.values()) console.log(`still failed ${line}`);

  const { succeeded, failed: nFailed, skipped, blocked, pending } = report.requests;
  console.log(`${succeeded} succeeded, ${nFailed} failed, ${skipped} skipped, ${blocked} blocked, ${pending} pending`);
  console.log(`run ${report.lifecycle}, ${report.completeness}${report.completenessReasons.length > 0 ? ` (${report.completenessReasons.join("; ")})` : ""}`);
  if (report.nothingDone !== null) {
    console.log(`nothing done: ${report.nothingDone.reason}: ${explainNothingDone(report.nothingDone, report.id)}`);
  }
}

if (import.meta.main) await main();
