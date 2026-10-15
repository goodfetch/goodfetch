// Snippet for Results, outcomes and errors (#466): catch by group or code, read outcomes as values.
import {
  DeadlineExceededError,
  GoodfetchError,
  PageError,
  TargetNotUniqueError,
  type AIWorkResult,
  type Logger,
  type CrawlRequestOutcome,
} from "@goodfetch/foxhound";

declare const log: Logger; // the program's own logger

declare function clickNext(): Promise<void>; // a page call: the page API is #472's
declare const outcomes: AsyncIterable<CrawlRequestOutcome>; // a crawl's `run.outcomes()` (ADR 0102)
declare const task: AIWorkResult<{ title: string; price: number }>; // the Agent's API is #474's

export async function failuresThrow(): Promise<void> {
  try {
    await clickNext();
  } catch (e) {
    if (e instanceof DeadlineExceededError) {
      log.error(e.message, { limit: e.limit }); // the setting, its value and its layer
    } else if (e instanceof TargetNotUniqueError) {
      log.error(e.message, { matches: e.details.matches }); // rendered in this SDK's language
    } else if (e instanceof PageError) {
      log.error(e.message, { observed: e.observed, url: e.url }); // any page problem, by group
    } else if (e instanceof GoodfetchError && e.retryable) {
      await clickNext(); // the SDK never retries on its own
    } else {
      throw e;
    }
  }
}

export async function outcomesAreValues(): Promise<Record<string, number>> {
  const counts: Record<string, number> = {};
  for await (const outcome of outcomes) {
    counts[outcome.kind] = (counts[outcome.kind] ?? 0) + 1;
    if (outcome.kind === "failed") log.error(outcome.error.message, { code: outcome.error.code });
    if (outcome.kind === "skipped") log.info("skipped", { reason: outcome.reason });
    if (outcome.kind === "blocked") log.warn("blocked", { cause: outcome.blockCause });
  }
  return counts;
}

export function aiWork(): { title: string; price: number } {
  if (task.ended.kind === "stopped") log.warn("stopped", { reason: task.ended.reason, partial: task.partial });
  log.info(task.verdict, { claim: task.reported?.claim }); // the model's claim is a signal, never proof; absent if it made none
  return task.requireVerified(); // the output, or a NotVerifiedError carrying the Verdict
}
