// Crawl definition and handler context (ADR 0093, decided in #477).
// A crawl is declared by a `Crawl` value, made with no connection and no I/O: each Label is declared
// first, returning its typed handle, and then gets exactly one handler. A Run carries the crawl out;
// `Foxhound.startCrawl` starts one, and a start under the same ID resumes it (ADR 0102, run.d.ts).
// A handler receives one context on every fetch tier: Collie's page over the request's document,
// with a call that only a browser can serve escalating the request to the browser tier.
// Outcomes and skip reasons are in results.d.ts, the settings in settings.d.ts. Datasets, pushing and
// reading Records are ADR 0103's (output.d.ts).

import type {
  CallOptions,
  EventIterator,
  JSONObject,
  Limits,
  Locator,
  Page,
  PageError,
  ProgramError,
  Recording,
  RefusedError,
  Response,
  Run,
  RunView,
  Schema,
  SchemaInput,
  SchemaOutput,
  Timeout,
  UnavailableError,
  URLMatcher,
  ExplainedSetting,
} from "@goodfetch/collie-client";
import type { Dataset, Label, PayloadInput, PayloadOutput } from "./data.js";
import type {
  AfterResponseHook,
  AnyLabel,
  BeforeRequestHook,
  HookOptions,
  PageStageHook,
  RecordCommitHook,
  RecordHookOptions,
  RecordOf,
  UntypedRecord,
} from "./hooks.js";
import type { CrawlRequestOutcome } from "./outcomes.js";
import type {
  DatasetOptions,
  DatasetRecord,
  Export,
  ExportOptions,
  KeyField,
  RecordsOptions,
  RejectedRecord,
} from "./output.js";
import type { CrawlRunProgress, CrawlRunReport } from "./run.js";
import type { CrawlRunSettings, LabelSettings, SettingKey } from "./settings.js";

/**
 * A pattern a link or a sitemap entry is matched against in the engine: a glob, or a regular expression
 * in the Rust `regex` syntax (ADR 0081). No predicate: every link would cross the connection. Not the
 * web platform's `URLPattern` (ADR 0117).
 */
export type LinkPattern = string | RegExp;

/**
 * The fetch tiers of ADR 0020. `auto`, the default, learns the tier per Label on a domain by sample
 * comparison; any other value forces it (ADR 0023), and Provenance records that it was forced.
 */
export type FetchTier = "http" | "discovered_api" | "browser_fetch" | "light_browser" | "browser";
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
   * Declares a Dataset and returns its typed handle (ADR 0103): there is no default Dataset, so every
   * push names one. A second Dataset of the same name throws `DatasetDuplicateError` at once, and a
   * `key` on a field the schema does not require throws `DatasetKeyInvalidError`.
   */
  dataset(name: string, options?: { readonly key?: readonly KeyField<JSONObject>[] }): Dataset<JSONObject>;
  dataset<S extends Schema>(name: string, options: DatasetOptions<S>): Dataset<SchemaInput<S>, SchemaOutput<S>>;
  /**
   * Attaches the Label's one handler. A second handler throws `LabelDuplicateError` at once; a Label
   * left without one makes the Run's start throw `LabelUnhandledError`.
   */
  handle<L extends Label<unknown, unknown>>(label: L, handler: Handler<L>): void;

  // Hooks in code (ADR 0104): one method per stage, run in the order declared, after the Hooks
  // that settings name (`hooks.*`), with no integers. Scoped to `labels`, a Hook sees their
  // Payloads typed; left unscoped, it runs for every Label. Never persisted, like handlers.

  /** Before each fetch of a request, retries, escalations and sample-comparison runs included: it may rewrite the URL, set headers or skip. */
  beforeRequest<const Ls extends readonly AnyLabel[] = readonly AnyLabel[]>(hook: BeforeRequestHook<Ls[number]>, options?: HookOptions<Ls>): void;
  /** After the final response, before the status rule: it may rewrite the body, mark a block or ask for a retry. */
  afterResponse<const Ls extends readonly AnyLabel[] = readonly AnyLabel[]>(hook: AfterResponseHook<Ls[number]>, options?: HookOptions<Ls>): void;
  /** Before the browser navigates: set cookies, block resources, add init scripts. Only on the `browser` and `light_browser` tiers. */
  beforeNavigation<const Ls extends readonly AnyLabel[] = readonly AnyLabel[]>(hook: PageStageHook<Ls[number]>, options?: HookOptions<Ls>): void;
  /** After the browser navigated, before the engine's own page work (consent, challenges, the expected page). Only on the `browser` and `light_browser` tiers. */
  afterNavigation<const Ls extends readonly AnyLabel[] = readonly AnyLabel[]>(hook: PageStageHook<Ls[number]>, options?: HookOptions<Ls>): void;
  /** Just before the handler, on every tier, with the handler's page: the router middleware of Crawlee's `router.use`. */
  pageReady<const Ls extends readonly AnyLabel[] = readonly AnyLabel[]>(hook: PageStageHook<Ls[number]>, options?: HookOptions<Ls>): void;
  /** Before each Record's commit: it may change, drop or reject it. Scoped to `datasets`, it sees their Record type. */
  beforeRecordCommit<
    const Ls extends readonly AnyLabel[] = readonly AnyLabel[],
    const Ds extends readonly Dataset<unknown, unknown>[] = readonly Dataset<UntypedRecord, unknown>[],
  >(hook: RecordCommitHook<Ls[number], RecordOf<Ds[number]>>, options?: RecordHookOptions<Ls, Ds>): void;
}

/** A link `enqueueLinks` matched, as its Payload function sees it. */
export interface LinkInfo {
  readonly href: string;
  readonly text: string;
  readonly attributes: Readonly<Record<string, string>>;
}

/** `enqueueLinks`'s arguments after the Label: one Payload for every link, or a function of each link returning its Payload, or `null` to skip it. */
export type LinkPayloadArgs<L extends Label<unknown, unknown>, O> = [PayloadInput<L>] extends [undefined]
  ? [options?: O]
  : [payload: PayloadInput<L> | ((link: LinkInfo) => PayloadInput<L> | null), options?: O];

/** The arguments after the Label: the Payload only when the Label has one, then the options. */
export type PayloadArgs<L extends Label<unknown, unknown>, O> = [PayloadInput<L>] extends [undefined]
  ? [options?: O]
  : [payload: PayloadInput<L>, options?: O];

/** A seed: a crawl request the program gives at the start, checked as an enqueue is. On a resume, the seeds must match the Run's. */
export interface Seed {
  readonly url: string;
  readonly label: string;
  /** Phantom: built only by `seed()`, so its Payload matched its Label. */
  readonly "~seed": true;
}
/** Builds a seed, typed as `enqueue` is. */
export declare function seed<L extends Label<unknown, unknown>>(url: string | URL, label: L, ...args: PayloadArgs<L, RequestShape>): Seed;
/** One rule of a sitemap seed: URLs matching the pattern go to the Label, one without a Payload. The first rule that matches wins. */
export type SitemapRule = readonly [pattern: LinkPattern, label: Label<undefined>];
/**
 * Seeds from a sitemap (ADR 0093; ADR 0037's CO-10): `url` is a sitemap, or a site's robots.txt
 * whose `Sitemap:` lines are read. Nested sitemap indexes are followed; an entry matching no rule
 * is not added and is reported as `no_rule`. It widens the scope by its own host, like any seed,
 * and its entries are checked against the scope as enqueues are, at depth 0.
 */
export declare function sitemap(url: string | URL, rules: readonly SitemapRule[]): Seed;

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

/** A crawl request's own method, headers and body (ADR 0093): a POST or GraphQL API is crawled page by page. */
export interface RequestShape {
  /**
   * `GET` by default. A crawl request is a read whatever its method, sent again on a retry, an
   * escalation or a sample comparison; an Effect goes through `page.request` inside `ctx.run.effect`
   * (ADR 0106).
   */
  readonly method?: string;
  readonly headers?: Readonly<Record<string, string>>;
  readonly body?: string | Uint8Array;
}

/** Options of `enqueue`. */
export interface EnqueueOptions extends CallOptions, RequestShape {
  /** Higher first, within its domain (ADRs 0004, 0005). Default 0. */
  readonly priority?: number;
  /** Ahead of the domain's pending requests at its priority (ADR 0005). */
  readonly forefront?: boolean;
  /** The request's identity in the Frontier instead of its method, URL and body hash: a declared canonicalization, which always wins (ADR 0005). */
  readonly key?: string;
}

/** Options of `enqueueLinks`: no `key` or `forefront`, which would give every link one identity or jump them all ahead. */
export interface EnqueueLinksOptions extends CallOptions {
  /** Higher first, within its domain (ADRs 0004, 0005). Default 0. */
  readonly priority?: number;
  /** Only links matching one of these, within the crawl's scope, which they narrow and never widen. */
  readonly include?: readonly LinkPattern[];
  /** No link matching any of these. */
  readonly exclude?: readonly LinkPattern[];
  /** At most this many links added: the first in document order, once `include` and `exclude` have filtered them. */
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
  | "no_rule"
  | "skipped_by_payload"
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
 * What a handler receives, the same on every fetch tier. Every tier but `browser` serves what it
 * can exactly: `http` and `browser_fetch` the fetched document, `discovered_api` the document and
 * network waits from the JSON calls it replays, `light_browser` scripts without layout. On the
 * document tiers `goto`, `reload` and `page.session().newPage()` are served over the same tier
 * through the navigation gate (ADR 0087). A call a tier cannot serve (on the document tiers another
 * gesture, a popup, `evaluate`, an unanswered network wait, a screenshot, an acting AI verb; on
 * `light_browser`, what needs layout or pixels),
 * a wait for something a static document lacks, or a page failing its Session's
 * `session.page_check.expectedPage` (ADR 0092) ends the attempt and runs the request again in the
 * browser, unless `fetch.tier` forces the tier, when the call throws `BrowserRequiredError` and the
 * page fails with `page_check_failed`. `page.request` never escalates. A question (`count()`,
 * `isVisible()`) answers at once and never escalates.
 *
 * A context belongs to one attempt. Once the attempt has ended (the handler's return, `skip`, `retry`,
 * an escalation, the handler's timeout, a lost lease, the Run's close), every call on it throws `AttemptEndedError`. An attempt
 * whose handler returned commits what it staged; one that ended early discards it.
 */
export interface CrawlContext<L extends Label<unknown, unknown> = Label<undefined>> {
  readonly request: CrawlRequest<L>;
  /**
   * Collie's page (ADR 0085), with its AI verbs, fenced by the work item's lease and journaled.
   * `page.session()` reaches cookies. The request's navigation has already happened when the handler
   * runs, so a route set here applies from the next navigation: to block whole resource types use
   * `browser.block_resources`, and to act before the navigation a before-navigation Hook (ADR 0117).
   */
  readonly page: Page;
  /** The main document's final response, after redirects; its body is always kept. */
  readonly response: Response;
  /** The tier that fetched this attempt. */
  tier(): ReportedFetchTier;
  /**
   * The crawl's Run, bound to this attempt (ADR 0093): `addSeed` and its Key-value store's writes
   * through it are staged with the attempt, and its reads see them first. The program's own `run`
   * value writes at once.
   */
  readonly run: CrawlRun;
  /**
   * Stages a Record with this attempt (ADR 0103): it commits with the attempt, numbered in its
   * Dataset, or is discarded if the attempt fails or ends early (`skip`, `retry`, an escalation,
   * the handler's timeout, a lost lease, the Run's close), after which a push throws
   * `AttemptEndedError`, as `enqueue` does. Resolves once staged. A Record failing the Dataset's schema, not JSON, or lacking its key's
   * fields throws `RecordInvalidError` and is not staged; a Dataset of another crawl throws
   * `DatasetUnknownError`.
   */
  push<I>(dataset: Dataset<I, unknown>, record: NoInfer<I>, options?: CallOptions): Promise<void>;
  /** Adds one URL to the Frontier, checked against the crawl's scope; a Payload failing its schema throws `PayloadInvalidError`. */
  enqueue<L extends Label<unknown, unknown>>(url: string | URL, label: L, ...args: PayloadArgs<L, EnqueueOptions>): Promise<Enqueued>;
  /** Adds every link the locator matches, each `href` resolved against the document's base URL. */
  enqueueLinks<L extends Label<unknown, unknown>>(
    links: Locator,
    label: L,
    ...args: LinkPayloadArgs<L, EnqueueLinksOptions>
  ): Promise<EnqueuedLinks>;
  /**
   * The first response kept for this attempt whose URL matches (ADR 0110): kept because its Label's
   * `fetch.kept_responses.urls` names it, on the request's page and its frames, from the start of its
   * navigation, so one the page loaded before the handler ran is found. Waits for one under
   * `timeout.wait`, as a waiter armed when the navigation started, and resolves once its body has
   * arrived, so the body never fails unless past `fetch.kept_responses.max_size`
   * (`BodyUnavailableError`). Past the timeout, `DeadlineExceededError` naming the matcher; on a
   * Label that keeps nothing, `ResponsesNotKeptError` at once. On `http` and `browser_fetch` it
   * escalates (`BrowserRequiredError` under a forced tier); `discovered_api` answers from its
   * replayed calls.
   */
  keptResponse(url: URLMatcher, options?: CallOptions): Promise<Response>;
  /** The responses kept for this attempt so far, in arrival order, without waiting; `url` filters them. Same rules as `keptResponse`. */
  keptResponses(url?: URLMatcher, options?: Omit<CallOptions, "timeout">): Promise<readonly Response[]>;
  /** Ends the request `skipped`, with the reason `handler` and this message. Throws to unwind the handler. */
  skip(message: string): never;
  /** Ends this attempt with `retry_requested`, retried under `crawl.retry.max`. Throws to unwind the handler. */
  retry(message: string): never;
}

/**
 * Options of `startCrawl` (ADR 0102). `runId`, `settings`, `timeout`, `limits` and `plan` are a Run's,
 * with `StartOptions`' rules on a resume (time.d.ts).
 */
export interface CrawlStartOptions {
  /**
   * Names the Run for good: a start with an existing Run ID resumes the Run if it was interrupted, or
   * returns it as it ended (ADR 0102); a new ID starts a new Run.
   *
   * On a resume the seeds and the Plan file are inputs that must match (`RunConflictError`); left out,
   * the engine generates one. `run.id()` reads it back.
   */
  readonly runId?: string;
  /** At least one. They define the crawl's scope, and on a resume they must match the Run's (`RunConflictError`). */
  readonly seeds: readonly Seed[];
  readonly settings?: CrawlRunSettings;
  readonly timeout?: Timeout;
  readonly limits?: Limits;
  /** The Run's Plan file (ADR 0091), an input of the Run like its seeds. */
  readonly plan?: string;
  /** The Run's Response recording (ADR 0105), an input like its seeds: replayed, or recorded into. */
  readonly recording?: Recording;
  /** Aborting the start sends `$/cancel` (ADR 0080): the start rejects with `signal.reason`, and a Run it already began stays, to be closed or resumed by its ID. */
  readonly signal?: AbortSignal;
}

/** Which outcomes `outcomes()` reads, filtered in the engine (ADR 0081). */
export interface OutcomesFilter {
  readonly kinds?: readonly ("succeeded" | "failed" | "skipped" | "blocked")[];
  readonly labels?: readonly Label<unknown, unknown>[];
  /** `now`: from the next commit, not the Run's first outcome: failures as they happen, in place of a failed-request callback (ADR 0093's R4). */
  readonly after?: "now";
}

/** Options of `retryFailed`. */
export interface RetryFailedOptions extends CallOptions {
  /** Re-admit the requests that ended `blocked` too: a block may have lifted (ADR 0021). Built-in: false. */
  readonly blocked?: boolean;
}

/** Options of `failedRequests`, the same choice as `retryFailed`'s (ADR 0117). */
export interface FailedRequestsOptions {
  /** Read the requests whose latest outcome is `blocked` too. Built-in: false. */
  readonly blocked?: boolean;
}

/** A crawl's Run (ADR 0102); its Records are ADR 0103's. Its handlers run from its start until it ends. */
export interface CrawlRun extends Run {
  /** All the Run's settings when `keys` is left out, Foxhound's keys included. */
  explainSettings(keys?: readonly SettingKey[], options?: CallOptions): Promise<readonly ExplainedSetting[]>;
  /** Lets the crawl finish: once its Frontier is empty and nothing is in flight, or, under `crawl.keep_alive`, once drained after this call. */
  finish(options?: CallOptions): Promise<CrawlRunReport>;
  stop(options?: CallOptions): Promise<CrawlRunReport>;
  progress(): EventIterator<CrawlRunProgress>;
  /**
   * Every ending in commit order, a request ending again after `retryFailed`: a log of endings, not
   * current state; `failedRequests()` reads the current failures.
   *
   * The crawl requests' outcomes, read from the Run's committed state in commit order from its first,
   * then following new commits until the Run ends. A reader of committed state, it never slows the
   * Run. A request `retryFailed` re-admits ends again, and its new outcome supersedes the old: key
   * outcomes by `request.id`.
   */
  outcomes(filter?: OutcomesFilter): EventIterator<CrawlRequestOutcome>;
  /**
   * The crawl requests whose latest outcome is `failed` (with `blocked: true`, also those whose latest
   * is `blocked`), each as that outcome, read from committed state at the call and ending after what is
   * committed: exactly what `retryFailed` with the same option would re-admit (ADR 0117). Never slows the Run.
   */
  failedRequests(options?: FailedRequestsOptions): EventIterator<CrawlRequestOutcome>;
  /**
   * Re-admits, in this same Run, every request that ended `failed` (and `blocked`, if asked), each
   * keeping its ID, Label, Payload and depth, its attempts counting on with a fresh `crawl.retry.max`.
   * A skip is a decision and is never re-admitted. On an ended Run it returns the Run to `running`,
   * its Verdict and Completeness given anew when it ends (ADR 0029). Answers how many it re-admitted.
   */
  retryFailed(options?: RetryFailedOptions): Promise<number>;
  /**
   * The Dataset's committed Records, from its first (or after `after`), following the Run live until
   * it ends and every Record is read (ADR 0103). A cursor over run state: reading slowly, or not at
   * all, never slows the crawl. One a native parse rejects ends the loop with `RecordInvalidError`
   * carrying its number. Through a handler's `ctx.run`, it reads what has committed and ends, since
   * the Run cannot end while the handler holds its request.
   */
  records<O>(dataset: Dataset<unknown, O>, options?: RecordsOptions): EventIterator<DatasetRecord<O>>;
  /** The Dataset's rejection table, each entry numbered in commit order, following the Run as `records` does. */
  rejectedRecords(dataset: Dataset<unknown, unknown>, options?: RecordsOptions): EventIterator<RejectedRecord>;
  /**
   * Writes an Export of the Records committed so far (ADR 0026), to save or stream wherever the engine
   * runs. Resolves only once its file is whole; a write the engine cannot make throws
   * `FileWriteFailedError` with `side: "engine"` and leaves no unfinished file (ADR 0116).
   */
  export(dataset: Dataset<unknown, unknown>, options: ExportOptions): Promise<Export>;
  /** Each seed's answer, given once at the start: added with its ID, or not added with its reason. On a resume, the answers given when the Run began. */
  seeds(): readonly Enqueued[];
  /**
   * Adds a seed to the running crawl (ADR 0037's CO-03), committed at once, or staged with the attempt
   * when called through a handler's `ctx.run`. A seed widens the scope by its host, where a handler's
   * `enqueue` is checked against it; one the domain policy refuses is not added (`domain_policy`).
   */
  addSeed<L extends Label<unknown, unknown>>(url: string | URL, label: L, ...args: PayloadArgs<L, EnqueueOptions>): Promise<Enqueued>;
  /** Adds a built seed, such as `sitemap(url, rules)`; a sitemap's entries matching no rule are reported as `no_rule` in the Run's report (ADR 0102). */
  addSeed(seed: Seed): Promise<Enqueued>;
}

/** A crawl's Run read by a program that does not drive it, opened with `openCrawlRun` (ADR 0102). Its Records are read with the program's own Dataset handles; on a Run with no live writer, `records()` and `rejectedRecords()` end after the last committed Record, as `outcomes()` does. */
export interface CrawlRunView extends RunView, Pick<CrawlRun, "outcomes" | "failedRequests" | "records" | "rejectedRecords" | "export"> {
  progress(): EventIterator<CrawlRunProgress>;
  report(options?: CallOptions): Promise<CrawlRunReport>;
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

/** Why an attempt ended: early, or `returned` for a call made after its handler returned. */
export type AttemptEndReason = "returned" | "skipped" | "retry" | "escalated" | "timeout" | "lease_lost" | "run_closed" | (string & {});
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
/** `keptResponse` or `keptResponses` on a request whose Label keeps nothing; the fix names `fetch.kept_responses.urls` (ADR 0110). Not retryable. */
export declare class ResponsesNotKeptError extends ProgramError {
  readonly code: "responses_not_kept";
  readonly details: { readonly label: string };
}
/**
 * A call on the document of a crawl page whose response the Label accepted but is no HTML document (a
 * PDF, JSON, an image): a locator, `content()`, `evaluate`, a screenshot or an AI verb. It never
 * escalates, since the browser holds no document either; the fix says to read `ctx.response`
 * (ADR 0119). Not retryable.
 */
export declare class ResponseNotDocumentError extends ProgramError {
  readonly code: "response_not_document";
  readonly details: { readonly contentType: string; readonly label: string };
}
/** An attempt the handler ended with `ctx.retry()`; the request's error once retries run out. */
export declare class RetryRequestedError extends PageError {
  readonly code: "retry_requested";
  readonly details: { readonly message: string };
}
/** A final 4xx that is no Block cause and not in `crawl.handle_statuses`. Not retryable. */
export declare class ResponseNotOKError extends PageError {
  readonly code: "response_not_ok";
  readonly details: { readonly status: number };
}
/** A final 5xx or 408 after the request's retries, not in `crawl.handle_statuses`: the site is failing for now. Retryable. */
export declare class ResponseUnavailableError extends PageError {
  readonly code: "response_unavailable";
  readonly details: { readonly status: number };
}
/**
 * The program stopped answering for this attempt with no word: its process ended (a crash, or a
 * signal it does not handle: close the Run on SIGINT or SIGTERM), its connection dropped, or its
 * Attached engine died with it and the Run was resumed. Retryable, and counted against
 * `crawl.retry.max`. The program's own stop (`close()`, a dropped Rust `serve` future) counts nothing.
 */
export declare class HandlerLostError extends UnavailableError {
  readonly code: "handler_lost";
}
/**
 * A handler's own navigation or fetch that Site preferences refuse: before it is sent, when its
 * response carries an AI-use signal, or from a waiter armed for a navigation no call waits on. A
 * crawl request's reasons, `robots_txt`, `crawl_delay`, or `use_signals` while
 * `site_preferences.respect_use_signals` is on (ADR 0086). If it escapes the handler, the request ends
 * `skipped` with the same reason. A Collie program's own navigation is never refused for a use signal.
 */
export declare class SitePreferencesRefusedError extends RefusedError {
  readonly code: "site_preferences_refused";
  readonly details: { readonly url: string; readonly reason: "robots_txt" | "crawl_delay" | "use_signals" | (string & {}) };
}
