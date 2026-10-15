// Hooks and SDK handlers in code (ADR 0104, decided in #481).
// A crawl declares its Hooks in code on its `Crawl`, one method per stage (crawl.d.ts), at the six
// stages whose answer changes the work. The observe-only stages (the run's lifecycle and a request's
// end) reach code through the Run's own values (its progress, outcomes and report, ADR 0102), and
// Hooks only as process Plugins and Script Hooks.
// A Collie program has no Hooks in code: its own lines, routes, init scripts and locator handlers
// answer each stage (ADR 0085).
// A Hook returns its change as a value and ends the work with a call that never returns, as a
// handler's `skip` and `retry` do (ADR 0093).
// A program's own implementation of a Plugin kind, an SDK handler, is registered on `connect()`
// in one option per kind keyed by name, as ADR 0101 registers `modelProviders`, and settings
// choose it by that name as they choose an installed Plugin.

import type { CrawlRequest, CrawlRun, ReportedFetchTier } from "./crawl.js";
import type { Dataset, JsonObject, Label } from "./data.js";
import type { Page } from "./lifecycle.js";
import type { Response } from "./page.js";
import type { BlockCause, ErrorCode, GoodfetchError, HandlerFailedDetails, ProgramError } from "./results.js";
import type { SecretPlaceholder, SecretReference } from "./sessions.js";
import type { CallOptions } from "./time.js";

/** The stages a Hook in code runs at, by their catalog names. */
export type HookStage =
  | "before_request"
  | "after_response"
  | "before_navigation"
  | "after_navigation"
  | "page_ready"
  | "before_record_commit";
/** A stage as reported: also the observe-only ones only Plugins and Script Hooks reach, and a newer engine's, kept as received (G3). */
export type ReportedHookStage = HookStage | "request_ended" | (string & {});

/** Any Label, for a Hook that runs for every Label: its Payload is then `unknown`. */
export type AnyLabel = Label<unknown, unknown>;

/** Options of declaring a Hook. */
export interface HookOptions<Ls extends readonly AnyLabel[]> {
  /**
   * Only the requests of these Labels; left out, every Label's. A Label of another crawl throws
   * `LabelUnknownError` at once, and an empty list `HookIneffectiveError`.
   */
  readonly labels?: Ls;
  /**
   * Names the Hook in `hook_failed`, the Run trace and a test's `runHook`: unique on its crawl, else
   * `HookDuplicateError` at once. Default: the function's own name, else the stage and position
   * (`before_request#2`).
   */
  readonly name?: string;
}

/** Options of declaring a Hook before a Record's commit, which may also be scoped by Dataset. */
export interface RecordHookOptions<Ls extends readonly AnyLabel[], Ds extends readonly Dataset<unknown, unknown>[]> extends HookOptions<Ls> {
  /** Only the Records of these Datasets, typed by their schema (ADR 0082); left out, every Dataset's. A Dataset of another crawl throws `DatasetUnknownError` at once, and an empty list `HookIneffectiveError`. */
  readonly datasets?: Ds;
}

/** What every Hook's context carries. A context belongs to one call: once the Hook has returned or ended the work, calls on it throw `AttemptEndedError`. */
export interface HookContext<L extends AnyLabel> {
  readonly request: CrawlRequest<L>;
  /** The crawl's Run, for its Key-value store (#479), which Script Hooks have too (ADR 0051). */
  readonly run: CrawlRun;
}

/**
 * Before each fetch of a request: every attempt, escalation and sample-comparison run comes here
 * again, so this is also the stage before a retry.
 */
export interface BeforeRequestContext<L extends AnyLabel> extends HookContext<L> {
  /** The tier about to fetch. */
  readonly tier: ReportedFetchTier;
  /** The method the request was enqueued with, `GET` by default (ADR 0093). A Hook reads it and never changes it. */
  readonly method: string;
  /** The body the request was enqueued with, or `null`. A Hook reads it (to sign the request, say) and never changes it. */
  readonly body: Uint8Array | null;
  /**
   * The request's headers as built so far, the enqueue's own and the Hooks before this one included. Cookies, Secret
   * values and the Session's credentials are added after the last Hook, so none of them is here.
   */
  readonly headers: Readonly<Record<string, string>>;
  /** The error of the attempt before this one; `null` on the first. */
  readonly previousError: GoodfetchError | null;
  /** Ends the request `skipped`, with the reason `hook` and this message; its outcome still lists every failed attempt's error. */
  skip(message: string): never;
}

/** What a before-request Hook changes: the URL and headers, never the method or the body the request was enqueued with (ADR 0093). */
export interface RequestChange {
  /**
   * A new URL, checked against the crawl's scope and the run's domain policy, else the request ends
   * `skipped` (`out_of_scope`, `domain_policy`); an enqueue's other checks do not apply. A Hook in
   * code may reach any host that admits. robots.txt and Site preferences read it as rewritten.
   */
  readonly url?: string | URL;
  /**
   * Headers to set, or `null` to remove one. `Host`, `Cookie`, `Authorization`, `Proxy-*` and
   * method overrides fail the Hook with `header_forbidden`. A value may be a Secret placeholder,
   * bound by `secrets.*` and checked against the request's URL (ADR 0092).
   */
  readonly headers?: Readonly<Record<string, string | SecretPlaceholder | null>>;
}

/** After the final response, before ADR 0093's status rule: where a soft block served with a 200 is marked. */
export interface AfterResponseContext<L extends AnyLabel> extends HookContext<L> {
  readonly tier: ReportedFetchTier;
  /** The final response, after redirects. */
  readonly response: Response;
  /** Ends the attempt `blocked` with this cause: ADR 0021 escalates by new Session, and the request ends `blocked` once escalation runs out. */
  block(cause: BlockCause, message?: string): never;
  /** Ends this attempt with `retry_requested`, retried under `crawl.retry.max`, as a handler's `retry`. */
  retry(message: string): never;
}

/** What an after-response Hook changes. */
export interface ResponseChange {
  /** A rewritten body, which the handler's page is served from. */
  readonly body?: string | Uint8Array;
}

/**
 * Before and after a navigation (only on the `browser` and `light_browser` tiers), and when the page is
 * ready, just before the handler, on every tier. Page calls follow the handler's tier rules: one
 * only a browser can serve escalates, or throws `BrowserRequiredError` under a forced tier.
 */
export interface PageStageContext<L extends AnyLabel> extends HookContext<L> {
  readonly tier: ReportedFetchTier;
  readonly page: Page;
}

/** Before a Record's commit, with the attempt's other outputs (ADR 0003). The engine checks the Record it commits against the Dataset's schema. */
export interface RecordCommitContext<L extends AnyLabel, R> extends HookContext<L> {
  readonly dataset: Dataset<R, unknown>;
  readonly record: R;
  /** Leaves the Record out, counted in the run's report with this reason. */
  drop(reason: string): never;
  /** Puts the Record in the Dataset's rejection table with this reason, read with `run.rejectedRecords` (ADR 0103). */
  reject(reason: string): never;
}

/** The Record type of a Dataset, as pushed: what a Hook before its commit sees and returns. */
export type RecordOf<D> = D extends Dataset<infer I, unknown> ? I : never;

export type BeforeRequestHook<L extends AnyLabel> = (ctx: BeforeRequestContext<L>) => RequestChange | void | Promise<RequestChange | void>;
export type AfterResponseHook<L extends AnyLabel> = (ctx: AfterResponseContext<L>) => ResponseChange | void | Promise<ResponseChange | void>;
export type PageStageHook<L extends AnyLabel> = (ctx: PageStageContext<L>) => void | Promise<void>;
/** Returns the Record to commit, changed or not; returning nothing commits it unchanged. */
export type RecordCommitHook<L extends AnyLabel, R> = (ctx: RecordCommitContext<L, R>) => R | void | Promise<R | void>;

// The Error codes of Hooks (ADR 0104).

/** Details of `hook_failed`: the Hook, then the exception as `handler_failed` carries a handler's. */
export interface HookFailedDetails extends HandlerFailedDetails {
  readonly stage: ReportedHookStage;
  /** The Hook's name. */
  readonly hook: string;
  /** `null` at a lifecycle stage, which has no request. */
  readonly label: string | null;
  /** The Goodfetch code raised inside the Hook, whose retryable flag this error carries; `null` for the program's own exception. */
  readonly wrappedCode: ErrorCode | null;
}
/**
 * A Hook threw. For the program's own exception, not retried unless `crawl.retry.handler_failed`
 * is on; for a Goodfetch code raised inside, retried when that code is. At the Record stage, the
 * attempt's commit fails and everything it staged is discarded.
 */
export declare class HookFailedError extends ProgramError {
  readonly code: "hook_failed";
  readonly details: HookFailedDetails;
}
/** A Hook that can never run: an empty `labels` or `datasets` list, at once, or, when the Run starts, a navigation Hook all of whose Labels force a tier that does not navigate (`http`, `discovered_api`, `browser_fetch`). */
export declare class HookIneffectiveError extends ProgramError {
  readonly code: "hook_ineffective";
  readonly details: { readonly hook: string; readonly stage: ReportedHookStage };
}
/** A second Hook of one name on one crawl, thrown at once. */
export declare class HookDuplicateError extends ProgramError {
  readonly code: "hook_duplicate";
  readonly details: { readonly hook: string };
}
/** A before-request Hook set a header the engine owns: `Host`, `Cookie`, `Authorization`, `Proxy-*` or a method override. Raised inside `hook_failed`. */
export declare class HeaderForbiddenError extends ProgramError {
  readonly code: "header_forbidden";
  readonly details: { readonly header: string };
}

// SDK handlers: a program's own implementation of a Plugin kind (ADR 0033).
//
// Registered on `connect()`, one option per kind, each a map keyed by name (`modelProviders` is ADR
// 0101's; `resolveSecret` ADR 0092's; Hooks are declared on the `Crawl`), each serving every Run of
// its connection. Settings choose one by name as they choose an installed Plugin. The only one of a
// kind that one setting chooses (Fetch backend, Proxy provider) becomes that setting's value at the
// connection scope, in the run layer with the origin `registered`, the lowest: every other
// run-layer value replaces it, and configuration does not; a lock on the key wins with no refusal.
// Two of such a kind fail `connect()` with `setting_invalid` when nothing else sets the key, as
// does a handler named as a built-in or an installed Plugin. A setting naming a handler that is
// neither installed nor registered fails with `setting_invalid` when its scope opens, and a resume
// whose program no longer registers one its Run used is refused before any work. A Product's SDK
// has only its own kinds' options: Collie's has no `fetchBackends` or `sinks` (ADR 0088). Each
// kind's handler is generated from its plugin-protocol method group (area 7's spec): the shapes
// below are provisional. A handler runs with the program's full rights, never confined. A
// GoodfetchError it throws keeps its code; any other exception is its kind's
// `<kind>_handler_failed`, whose message names it, as ADR 0101's `model_handler_failed`.

/** The request a Fetch backend is handed: a target the engine already checked under the page class and the run's domain policy (ADR 0033). */
export interface BackendRequest {
  readonly url: string;
  /** The crawl request's own method (ADR 0093). */
  readonly method: string;
  readonly headers: Readonly<Record<string, string>>;
  /** The crawl request's own body, or `null`. */
  readonly body: Uint8Array | null;
  /** The Session's proxy server, with no credentials, or `null`: a proxied crawl does not leave from the user's own address. */
  readonly proxy: string | null;
}
/** A Fetch backend's answer. A redirect's target comes back as a new request, checked again. */
export interface BackendResponse {
  readonly status: number;
  readonly headers: Readonly<Record<string, string>>;
  readonly body: Uint8Array;
}
/**
 * The HTTP tiers' transport. Like a third-party backend, it never carries a request that uses a
 * Secret or a logged-in identity, such as a logged-in Session's, one whose Hook set a Secret
 * placeholder, one whose Session has `session.http_credentials` or one whose proxy has a password:
 * the engine sends those itself, so no cookie or Secret value passes through the program.
 */
export type FetchBackendHandler = (request: BackendRequest, options: CallOptions) => Promise<BackendResponse>;

/** What a Proxy provider is asked for: one proxy per Session (ADRs 0021, 0092). */
export interface ProxyRequest {
  readonly sessionId: string;
  /** The registrable domain of the Session's first request. */
  readonly domain: string;
  /** Set when the Session replaces one retired for this Block cause. */
  readonly replacing: BlockCause | null;
}
/** A proxy, with Playwright's fields as `proxy.*` has them (ADR 0092). */
export interface ProxyAnswer {
  readonly server: string;
  readonly username?: string;
  /** A Secret reference, never a value: `caller:` resolves through `resolveSecret`. */
  readonly password?: SecretReference;
}
export type ProxyProviderHandler = (request: ProxyRequest) => Promise<ProxyAnswer>;

/** One delivery to a Sink handler (ADR 0008): the same delivery ID on each retry, so a write keyed by it lands once. */
export interface SinkDelivery {
  readonly deliveryId: string;
  readonly runEpoch: number;
  readonly dataset: string;
  /** Committed Records, in commit order. */
  readonly records: readonly JsonObject[];
}
/**
 * A Sink written in the program (ADR 0026's SDK-handler form), serving the `sinks.<name>` entry of
 * its name, which gives its Datasets, write mode and key; a `type` there beside a handler of that
 * name is refused with `setting_invalid` when the Run starts. A failed delivery is retried on ADR
 * 0008's schedule, since its delivery ID makes a retry safe; its Guarantee class is probed as any
 * Sink's (ADR 0026).
 */
export type SinkHandler = (delivery: SinkDelivery) => Promise<void>;

/** Details of a `<kind>_handler_failed`: the handler, then the exception as `handler_failed` carries a handler's. */
export interface SdkHandlerFailedDetails extends HandlerFailedDetails {
  /** The name the handler was registered under. */
  readonly handler: string;
}
/** A Fetch backend handler threw something other than a GoodfetchError. In a crawl, retried only when `crawl.retry.handler_failed` is on for the request's Label. */
export declare class FetchHandlerFailedError extends ProgramError {
  readonly code: "fetch_handler_failed";
  readonly details: SdkHandlerFailedDetails;
}
/** A Proxy provider handler threw something other than a GoodfetchError: the Session does not open. In a crawl, the requests that needed it are retried only when `crawl.retry.handler_failed` is on for their Label; in a Collie Run, the call fails. */
export declare class ProxyHandlerFailedError extends ProgramError {
  readonly code: "proxy_handler_failed";
  readonly details: SdkHandlerFailedDetails;
}
/** A Sink handler threw something other than a GoodfetchError: the delivery is retried on ADR 0008's schedule. */
export declare class SinkHandlerFailedError extends ProgramError {
  readonly code: "sink_handler_failed";
  readonly details: SdkHandlerFailedDetails;
}

/** The Record a Hook not scoped by Dataset sees and returns: any JSON object, as an undeclared Dataset takes (ADR 0082). */
export type UntypedRecord = JsonObject;
