// Crawl definition and handler context (ADR 0093, decided in #477).
// A crawl is declared by a `Crawl` value, made with no connection and no I/O: each Label is declared
// first, returning its typed handle, and then gets exactly one handler. A Run carries the crawl out;
// its start and resume verbs are #478's, so `Foxhound.start` in lifecycle.d.ts is provisional.
// A handler receives one context on every fetch tier: Collie's page over the request's document,
// with a call that only a browser can serve escalating the request to the browser tier.
// Outcomes and skip reasons are in results.d.ts, the settings in settings.d.ts.

import type { Label, PayloadInput, PayloadOutput, Schema, SchemaInput, SchemaOutput } from "./data.js";
import type { Page, Run } from "./lifecycle.js";
import type { Locator, Response } from "./page.js";
import type { PageError, ProgramError, RefusedError, UnavailableError } from "./results.js";
import type { CrawlRunSettings, LabelSettings } from "./settings.js";
import type { CallOptions, Limits, Timeout } from "./time.js";

/** A URL pattern matched in the engine: a glob, or a regular expression in the Rust `regex` syntax (ADR 0081). No predicate: every link would cross the connection. */
export type UrlPattern = string | RegExp;

/**
 * The fetch tiers of ADR 0020. `auto`, the default, learns the tier per Label on a domain by sample
 * comparison; any other value forces it (ADR 0023), and Provenance records that it was forced.
 */
export type FetchTier = "http" | "discovered_api" | "browser_fetch" | "browser";
/** A tier a newer engine reports and this SDK does not know is kept as received (G3). */
export type ReportedFetchTier = FetchTier | (string & {});

/** Options of declaring a Label. */
export interface LabelOptions<S extends Schema | undefined> {
  /** The Payload's schema (ADR 0082). Left out: the Label takes no Payload. */
  readonly payload?: S;
  /** Settings for this Label's requests: a scope between the Run and each request. */
  readonly settings?: LabelSettings;
  /** The scripted login step of the Sessions this Label's requests use (ADRs 0006, 0092), called with the Session's page on the login page. Setting it gives the Label a Session pool of its own. */
  readonly login?: (page: Page) => Promise<void>;
}

/** Options of a crawl's declaration. */
export interface CrawlOptions {
  /** The scripted login step of the Run's Session pool (ADRs 0006, 0092); a Label's own `login` wins for its Sessions. */
  readonly login?: (page: Page) => Promise<void>;
}

/** A Label's handler. It may return a promise; its value is ignored. */
export type Handler<L extends Label<unknown, unknown>> = (ctx: CrawlContext<L>) => void | Promise<void>;

/**
 * A crawl's declaration: its Labels and their handlers. It needs no connection and does no I/O, so a
 * module may declare it when it loads. A Run is started from it, and resumed with it, since handlers
 * are never persisted.
 */
export declare class Crawl {
  constructor(options?: CrawlOptions);
  /**
   * Declares a Label and returns its typed handle. A second Label of the same name throws
   * `LabelDuplicateError` at once.
   */
  label(name: string, options?: LabelOptions<undefined>): Label<undefined>;
  label<S extends Schema>(name: string, options: LabelOptions<S> & { readonly payload: S }): Label<SchemaInput<S>, SchemaOutput<S>>;
  /**
   * Attaches the Label's one handler. A second handler throws `LabelDuplicateError` at once; a Label
   * left without one makes the Run's start throw `LabelUnhandledError`.
   */
  handle<L extends Label<unknown, unknown>>(label: L, handler: Handler<L>): void;
}

/** The arguments after the Label: the Payload only when the Label has one, then the options. */
export type PayloadArgs<L extends Label<unknown, unknown>, O> = [PayloadInput<L>] extends [undefined]
  ? [options?: O]
  : [payload: PayloadInput<L>, options?: O];

/** A seed: a crawl request the program gives at the start, checked as an enqueue is. Provisional: the start verb is #478's. */
export interface Seed {
  readonly url: string;
  readonly label: string;
  /** Phantom: built only by `seed()`, so its Payload matched its Label. */
  readonly "~seed": true;
}
/** Builds a seed, typed as `enqueue` is. */
export declare function seed<L extends Label<unknown, unknown>>(url: string | URL, label: L, ...args: PayloadArgs<L, EnqueueOptions>): Seed;

/** One crawl request of Label `L`, as its handler sees it. The engine's fields are kept apart from the Payload (ADR 0082). */
export interface CrawlRequest<L extends Label<unknown, unknown> = Label<undefined>> {
  readonly id: string;
  /** The URL the Frontier admitted, after its URL rules (ADR 0005). The final URL is `page.url()`. */
  readonly url: string;
  /** The request's own Label, the handle to enqueue more of the same kind (pagination). */
  readonly label: L;
  /** Parsed by the Label's schema: its output type. */
  readonly payload: PayloadOutput<L>;
  /** 0 for a seed, one more for each enqueue from a handler. */
  readonly depth: number;
  /** 1 for the first attempt. Escalations and sample-comparison runs are not retries and do not count. */
  readonly attempt: number;
}

/** Options of `enqueue`. */
export interface EnqueueOptions extends CallOptions {
  /** Higher first, within its domain (ADRs 0004, 0005). Default 0. */
  readonly priority?: number;
  /** Ahead of the domain's pending requests at its priority (ADR 0005). */
  readonly forefront?: boolean;
  /** The request's identity in the Frontier instead of its URL: a declared canonicalization, which always wins (ADR 0005). */
  readonly key?: string;
}

/** Options of `enqueueLinks`: no `key` or `forefront`, which would give every link one identity or jump them all ahead. */
export interface EnqueueLinksOptions extends CallOptions {
  /** Higher first, within its domain (ADRs 0004, 0005). Default 0. */
  readonly priority?: number;
  /** Only links matching one of these, within the crawl's scope, which they narrow and never widen. */
  readonly include?: readonly UrlPattern[];
  /** No link matching any of these. */
  readonly exclude?: readonly UrlPattern[];
  /** At most this many links added. */
  readonly limit?: number;
}

/** Why a URL was not added. A newer engine's reason is kept as received (G3). */
export type NotAddedReason =
  | "duplicate"
  | "out_of_scope"
  | "domain_policy"
  | "max_depth"
  | "max_requests"
  | "unsupported_scheme"
  | "no_link"
  | (string & {});

/**
 * What an enqueue did: an answer, never an error (ADR 0079). From a handler, an addition is staged
 * with the attempt and committed with it (ADR 0003).
 */
export type Enqueued =
  | { readonly added: true; readonly id: string; readonly url: string }
  | { readonly added: false; readonly url: string; readonly reason: NotAddedReason };

/** What `enqueueLinks` did with each element its locator matched. An element with no link is reported with `url: null` and `no_link`. */
export interface EnqueuedLinks {
  readonly added: readonly { readonly id: string; readonly url: string }[];
  readonly notAdded: readonly { readonly url: string | null; readonly reason: NotAddedReason }[];
}

/**
 * What a handler receives, the same on every fetch tier. On the HTTP and discovered-API tiers the
 * engine serves `page` from the fetched document; a call only a browser can serve (a gesture,
 * `evaluate`, a network wait, a screenshot, an acting AI verb), or a wait for something the static
 * document lacks, ends the attempt and runs the request again in the browser, unless `fetch.tier`
 * forces the tier, when it throws `BrowserRequiredError`. The non-browser tiers are `http` and
 * `browser_fetch`, served from the document, and `discovered_api`, which also serves network waits
 * from the JSON calls it replays; `page.request` never escalates. So does a page failing its Session's
 * `session.page_check.expectedPage` (ADR 0092): it escalates, or fails with `page_check_failed` on
 * the browser tier or a forced one. A question (`count()`, `isVisible()`)
 * answers at once and never escalates.
 *
 * A context belongs to one attempt. Once the attempt has ended (`skip`, `retry`, an escalation, the
 * handler's timeout, a lost lease), every call on it throws `AttemptEndedError`, and what the attempt
 * staged is discarded.
 */
export interface CrawlContext<L extends Label<unknown, unknown> = Label<undefined>> {
  readonly request: CrawlRequest<L>;
  /** Collie's page (ADR 0085), with its AI verbs, fenced by the work item's lease and journaled. `page.session()` reaches cookies. */
  readonly page: Page;
  /** The main document's final response, after redirects; its body is always kept. */
  readonly response: Response;
  /** The tier that fetched this attempt. */
  readonly tier: ReportedFetchTier;
  /** The crawl's Run: its Datasets and Key-value store are #479's. */
  readonly run: CrawlRun;
  /** Adds one URL to the Frontier, checked against the crawl's scope; a Payload failing its schema throws `PayloadInvalidError`. */
  enqueue<L extends Label<unknown, unknown>>(url: string | URL, label: L, ...args: PayloadArgs<L, EnqueueOptions>): Promise<Enqueued>;
  /** Adds every link the locator matches, each `href` resolved against the document's base URL. */
  enqueueLinks<L extends Label<unknown, unknown>>(
    links: Locator,
    label: L,
    ...args: PayloadArgs<L, EnqueueLinksOptions>
  ): Promise<EnqueuedLinks>;
  /** Ends the request `skipped`, with the reason `handler` and this message. Throws to unwind the handler. */
  skip(message: string): never;
  /** Ends this attempt with `retry_requested`, retried under `crawl.retry.max`. Throws to unwind the handler. */
  retry(message: string): never;
}

/** Options of starting a crawl. Provisional: the start verb is #478's; `settings`, `timeout` and `limits` are a Run's (ADRs 0080, 0083). */
export interface CrawlStartOptions {
  /** At least one. They define the crawl's scope. */
  readonly seeds: readonly Seed[];
  readonly settings?: CrawlRunSettings;
  readonly timeout?: Timeout;
  readonly limits?: Limits;
}

/** A crawl's Run. Its outcomes, progress and resume are #478's. */
export interface CrawlRun extends Run {
  /**
   * Adds a seed to the running crawl (ADR 0037's CO-03), committed at once, or staged with the attempt
   * when called through a handler's `ctx.run`. A seed widens the scope by its host, where a handler's
   * `enqueue` is checked against it; one the domain policy refuses is not added (`domain_policy`).
   */
  addSeed<L extends Label<unknown, unknown>>(url: string | URL, label: L, ...args: PayloadArgs<L, EnqueueOptions>): Promise<Enqueued>;
}

// The crawl's Error codes (ADR 0093).

/** A second Label of one name, or a second handler for one Label. Thrown at once, with no I/O. */
export declare class LabelDuplicateError extends ProgramError {
  readonly code: "label_duplicate";
  readonly details: { readonly label: string };
}
/** The Run's start found a Label with no handler. */
export declare class LabelUnhandledError extends ProgramError {
  readonly code: "label_unhandled";
  readonly details: { readonly labels: readonly string[] };
}
/** An enqueue named a Label this crawl does not declare, or a resume found pending requests of one it no longer declares; nothing ran. */
export declare class LabelUnknownError extends ProgramError {
  readonly code: "label_unknown";
  readonly details: { readonly label: string; readonly pending: number };
}

/** Why an attempt ended before its handler returned. */
export type AttemptEndReason = "skipped" | "retry" | "escalated" | "timeout" | "lease_lost" | "run_closed" | (string & {});
/**
 * A call on the context of an attempt that has already ended. Let it propagate: the SDK ends the
 * handler with it, and catching it changes nothing, since the decision stands.
 */
export declare class AttemptEndedError extends ProgramError {
  readonly code: "attempt_ended";
  readonly details: { readonly reason: AttemptEndReason };
}
/** A call only a browser can serve, while `fetch.tier` forces a tier without one; `limit` names that setting and its Label. */
export declare class BrowserRequiredError extends ProgramError {
  readonly code: "browser_required";
  readonly details: { readonly tier: ReportedFetchTier; readonly label: string };
}
/** An attempt the handler ended with `ctx.retry()`; the request's error once retries run out. */
export declare class RetryRequestedError extends PageError {
  readonly code: "retry_requested";
  readonly details: { readonly message: string };
}
/** A final 4xx that is no Block cause and not in `crawl.handle_statuses`. Not retryable. */
export declare class ResponseNotOkError extends PageError {
  readonly code: "response_not_ok";
  readonly details: { readonly status: number };
}
/** A final 5xx or 408 after the request's retries, not in `crawl.handle_statuses`: the site is failing for now. Retryable. */
export declare class ResponseUnavailableError extends UnavailableError {
  readonly code: "response_unavailable";
  readonly details: { readonly status: number };
}
/**
 * The program stopped answering for this attempt: its process ended, its connection dropped, or
 * Rust's `serve` future was dropped. Retryable, and counted against `crawl.retry.max`; an attempt the
 * program's own `close()` stops counts nothing.
 */
export declare class HandlerLostError extends UnavailableError {
  readonly code: "handler_lost";
}
/**
 * A handler's own navigation or fetch that Site preferences refuse (ADR 0086), refused before it is
 * sent. If it escapes the handler, the request ends `skipped` with the same reason.
 */
export declare class SitePreferencesRefusedError extends RefusedError {
  readonly code: "site_preferences_refused";
  readonly details: { readonly url: string; readonly reason: "robots_txt" | "crawl_delay" | (string & {}) };
}
