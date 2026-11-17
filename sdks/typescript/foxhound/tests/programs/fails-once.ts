// A crawl whose handler pushes its attempt's number into the Dataset
// `attempts`, then fails its first attempt, as a program writes it: its
// settings turn on the retry of a failed handler (`crawl.retry`), so the
// engine delivers the Crawl request again (ADR 0093). Each call declares a
// fresh crawl, with its own count of attempts.
import { Crawl, type Dataset, type Label, type StartCrawlOptions } from "@goodfetch/foxhound";

/** One attempt's Record: its number, from 1. */
export interface Attempt {
  readonly attempt: number;
}

/** The crawl, what it declares, the settings it runs with and its handler's count of attempts. */
export interface FailsOnce {
  readonly crawl: Crawl;
  readonly page: Label;
  readonly attempts: Dataset<Attempt>;
  /** Its pages over `http`, one handler call at a time, a failed handler retried. */
  readonly settings: NonNullable<StartCrawlOptions["settings"]>;
  /** How many times the handler was called. */
  attemptsMade(): number;
}

/** Declares a crawl whose handler fails its first attempt after its push. */
export function failsOnce(): FailsOnce {
  const crawl = new Crawl();
  const page = crawl.label("page");
  const attempts = crawl.dataset<Attempt>("attempts");
  let attempt = 0;
  crawl.handle(page, async (context) => {
    attempt += 1;
    await context.push(attempts, { attempt });
    if (attempt === 1) throw new Error("the first attempt fails after its push");
  });
  return {
    crawl,
    page,
    attempts,
    settings: {
      fetch: { tier: "http" },
      crawl: { handler: { concurrency: 1 }, retry: { handler_failed: true } },
    },
    attemptsMade: () => attempt,
  };
}
