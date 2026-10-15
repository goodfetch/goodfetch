// Time, waits, deadlines, cancellation and Limits (ADR 0080, decided in #467).
// Every duration in the TypeScript SDK is a `number` of milliseconds, named without a unit suffix.
// A `timeout` nests: a call inside another gets the smaller of its own timeout and what remains of
// the outer one, and never extends it; time spent in a Human pause does not count.
// The page API around the waits (locators, actions, `goto`) is ADR 0085's (page.d.ts).

import type { Recording } from "./recording.js";
import type { Request, Response } from "./page.js";
import type { ProgramError } from "./results.js";
import type { RunSettings } from "./settings.js";

/**
 * How long a call may take, in milliseconds. Omitted: the setting's default (finite for every phase).
 * `null`: no timeout. `0`, a negative number or a non-finite number fails with a `ProgramError`
 * that names `null` (G3).
 */
export type Timeout = number | null;

/** Options every cancellable call takes. A signal and the timeout both apply: whichever comes first ends the call. */
export interface CallOptions {
  readonly timeout?: Timeout;
  /** Aborting sends `$/cancel`; the call rejects with `signal.reason`, as `fetch` does. */
  readonly signal?: AbortSignal;
}

/**
 * Matches a URL. A glob or a regular expression runs in the engine, in the Rust `regex` syntax (no
 * look-around or backreferences, linear time); a predicate runs in the program (ADR 0081).
 */
export type UrlMatcher = string | RegExp | ((url: URL) => boolean);

/** Load states. `networkidle`: no request in flight for 500 ms, WebSockets and server-sent events excluded. */
export type LoadState = "domcontentloaded" | "load" | "networkidle";

/** A count, exact or bounded. */
export type Count = number | { readonly min?: number; readonly max?: number };

/**
 * The page's navigation and app-state waits. Each waiter is armed when the method is called, not
 * when it is awaited, so `const r = page.waitForResponse(u); await click(); await r` and
 * `Promise.all` never miss an event (the engine keeps each page's calls in order).
 */
export interface PageWaits {
  waitForURL(url: UrlMatcher, options?: CallOptions & { readonly waitUntil?: LoadState }): Promise<void>;
  waitForResponse(url: UrlMatcher, options?: CallOptions): Promise<Response>;
  waitForRequest(url: UrlMatcher, options?: CallOptions): Promise<Request>;
  /** Waits until a matching request has finished, its body read. */
  waitForRequestFinished(url: UrlMatcher, options?: CallOptions): Promise<Request>;
  waitForLoadState(state?: LoadState, options?: CallOptions): Promise<void>;

  /**
   * @deprecated Removed: a fixed sleep waits for nothing in particular. Wait for the app's state
   * (`waitForURL`, `waitForResponse`, `waitForLoadState`, `locator.waitFor`, `locator.waitForCount`),
   * or use the host's own timer. Fails with the Error code `removed`.
   */
  waitForTimeout(timeout: never): never;
}

/** A sum of money in a currency, as a decimal string so no float rounding reaches a Limit. */
export interface Money {
  readonly amount: `${number}`;
  /** An ISO 4217 code, such as `EUR`. */
  readonly currency: string;
}

/**
 * Limits (ADR 0041) a program sets for its own work: one per kind of Usage, plus a price. A program
 * may set and lower them, never raise them: only a person raises a Limit. Reaching one follows
 * ADR 0079: a call refused because a Limit is used up throws `LimitReachedError`, and AI work a Limit
 * stops returns `ended: stopped` with the Limit by name; the Run ends `stopped` and `incomplete`.
 */
export interface Limits {
  readonly modelTokens?: number;
  readonly modelCalls?: number;
  /** Browser time in milliseconds of Usage, not elapsed time: that is the Run's `timeout`. */
  readonly browserTime?: number;
  readonly proxyBytes?: number;
  readonly paidSolves?: number;
  /** Only caps: a Paid fetch is made only under a Limit a Client could not set (ADRs 0041, 0045). */
  readonly paidFetches?: number;
  /**
   * A price, not a kind of Usage: priced through the price table (ADR 0012), as `usage.price` is
   * (ADR 0101). A call whose model has no price is refused under it with `model_price_unknown`.
   */
  readonly price?: Money;
}

/** Options of starting a Run (ADR 0102); `timeout` and `limits` are ADR 0080's, `settings` ADR 0083's. */
export interface StartOptions {
  /**
   * The Run's ID, which names it for good in the Run index (ADR 0031). Left out, the engine generates
   * one. A start that names an existing Run resumes it or returns it ended (`run.startedAs`), so a
   * program run again under its ID carries on where it stopped. Its characters are the engine's to
   * check: `RunIdInvalidError`.
   */
  readonly id?: string;
  /**
   * Settings for this Run and everything opened beneath it (ADR 0083). On a resume, a changed value
   * applies from then on, in the Run trace with its layer; a key fixed for a Run's life, such as
   * `crawl.scope.*`, throws `SettingNotApplicableError`.
   */
  readonly settings?: RunSettings;
  /**
   * How long the whole Run may last. Omitted: the setting's value, which has no built-in default;
   * `null`: no timeout. On expiry the Run stops with a Checkpoint, resumable; a resume's clock starts afresh.
   */
  readonly timeout?: Timeout;
  /**
   * On a resume, compared with what the Run began with: equal leaves each Limit as it stands, a
   * person's raise included; lower lowers it; higher throws `LimitRaiseRefusedError` (ADR 0041).
   */
  readonly limits?: Limits;
  /**
   * The Run's Plan file, relative to the working directory: an input of the Run (ADR 0091). The SDK
   * reads it here and writes a promoted version back here. Left out, the Plan lives only in the Run trace.
   */
  readonly plan?: string;
  /**
   * The Run's Response recording (ADR 0105), an input like `plan`, read and written here: `replay`
   * answers every request from it and sends none, `record` writes what the Run received when it ends.
   */
  readonly recording?: Recording;
}

/**
 * Details of `deadline_exceeded`. The error's `limit` names the phase's setting and its layer; a
 * call's own `timeout` argument is that setting at the `run` layer (ADR 0028).
 */
export interface DeadlineExceededDetails {
  /** The timeout that ran out, in milliseconds. */
  readonly timeout: number;
  readonly elapsed: number;
  /** What the call waited for, rendered in this SDK's language. */
  readonly waitedFor: string;
  /** The enclosing call whose timeout ended this one, when it was not the call's own. */
  readonly endedBy?: { readonly operation: string; readonly callId: string };
}

/** A removed name was called: its replacement is in `fix`. */
export declare class RemovedError extends ProgramError {
  readonly code: "removed";
}

