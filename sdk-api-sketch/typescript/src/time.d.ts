// Time, waits, deadlines, cancellation and Limits (ADR 0080, decided in #467).
// Every duration in the TypeScript SDK is a `number` of milliseconds, named without a unit suffix.
// A `timeout` nests: a call inside another gets the smaller of its own timeout and what remains of
// the outer one, and never extends it; time spent in a Human pause does not count.
// The page API around the waits (locators, actions, `goto`) is a placeholder owned by #472.

import type { ProgramError } from "./results.js";

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

/** Matches a URL: a glob, a regular expression, or a host-side predicate (where it runs is #468's). */
export type UrlMatcher = string | RegExp | ((url: URL) => boolean);

/** A response the page received. Its full shape is #472's. */
export interface Response {
  readonly url: string;
  readonly status: number;
}

/** A request the page sent. Its full shape is #472's. */
export interface Request {
  readonly url: string;
  readonly method: string;
}

/** Load states. `networkidle`: no request in flight for 500 ms, WebSockets and event streams excluded. */
export type LoadState = "domcontentloaded" | "load" | "networkidle";

/** A count, exact or bounded. */
export type Count = number | { readonly min?: number; readonly max?: number };

/** A placeholder locator: its constructors and actions are #472's; its waits are #467's. */
export interface Locator {
  /** Waits through the guard (ADR 0019) until the action is safe, then acts. */
  click(options?: CallOptions): Promise<void>;
  fill(value: string, options?: CallOptions): Promise<void>;
  /** Playwright's `waitFor`: one element in the given state. */
  waitFor(options?: CallOptions & { readonly state?: "attached" | "detached" | "visible" | "hidden" }): Promise<void>;
  /** Waits until the number of matches is in range: a different behaviour, so a different name (S3). */
  waitForCount(count: Count, options?: CallOptions): Promise<number>;
}

/**
 * The page's navigation and app-state waits. Each waiter is armed when the method is called, not
 * when it is awaited, so `const r = page.waitForResponse(u); await click(); await r` and
 * `Promise.all` never miss an event (the engine keeps each page's calls in order).
 */
export interface PageWaits {
  /** Placeholder for #472's navigation. Its timeout is the navigation phase's. */
  goto(url: string, options?: CallOptions & { readonly waitUntil?: LoadState }): Promise<Response | null>;
  /** Placeholder locators (#472). */
  locator(selector: string): Locator;
  getByRole(role: string, options?: { readonly name?: string | RegExp }): Locator;

  waitForURL(url: UrlMatcher, options?: CallOptions & { readonly waitUntil?: LoadState }): Promise<void>;
  waitForResponse(url: UrlMatcher, options?: CallOptions): Promise<Response>;
  waitForRequest(url: UrlMatcher, options?: CallOptions): Promise<Request>;
  /** Waits until a matching request has finished, its body read. */
  waitForRequestFinished(url: UrlMatcher, options?: CallOptions): Promise<Request>;
  waitForLoadState(state?: LoadState, options?: CallOptions): Promise<void>;
  /** Waits until a page function returns a truthy value; its serialization contract is #472's. */
  waitForFunction<R>(pageFunction: string, arg?: unknown, options?: CallOptions): Promise<R>;

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
  /** A price, not a kind of Usage: priced through ADR 0012's price table. */
  readonly cost?: Money;
}

/** Options of starting a Run. The start verbs are #478's; these two options are #467's. */
export interface StartOptions {
  /** How long the whole Run may last. Omitted: no timeout. On expiry the Run stops with a Checkpoint, resumable. */
  readonly timeout?: Timeout;
  readonly limits?: Limits;
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

