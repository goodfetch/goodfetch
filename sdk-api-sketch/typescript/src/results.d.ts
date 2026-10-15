// Results, outcomes and errors (ADR 0079, decided in #466).
// Failures throw a GoodfetchError: one class per Error code under one of seven groups by remedy.
// Expected outcomes are values in the return type, and no option changes which (G1, S2).
// The codes below are a representative subset: each area's spec lists its codes, their groups
// and their details in goodfetch-protocol, and codegen writes one class per code.

import type { AgentTaskProgress } from "./agent.js";
import type { ModelUsage } from "./models.js";
import type { RecordCounts } from "./output.js";
import type { DeadlineExceededDetails } from "./time.js";

/** A group of Error codes sharing one remedy, carried on the wire as the JSON-RPC numeric class. */
export type ErrorGroup =
  | "program"
  | "refused"
  | "page"
  | "model"
  | "deadline_exceeded"
  | "unavailable"
  | "internal";

/** The Error codes this sketch shows. */
export type KnownErrorCode =
  | "closed"
  | "removed"
  | "call_cancelled"
  | "handler_failed"
  | "listener_failed"
  | "hook_failed"
  | "header_forbidden"
  | "hook_ineffective"
  | "hook_duplicate"
  | "fetch_handler_failed"
  | "proxy_handler_failed"
  | "sink_handler_failed"
  | "limit_reached"
  | "page_check_failed"
  | "target_not_unique"
  | "blocked"
  | "not_verified"
  | "deadline_exceeded"
  | "plugin_crashed"
  | "payload_invalid"
  | "payload_schema_changed"
  | "schema_validator_failed"
  | "setting_unknown"
  | "setting_invalid"
  | "setting_not_applicable"
  | "setting_locked"
  | "capability_unavailable"
  | "selector_invalid"
  | "target_unsupported"
  | "page_function_failed"
  | "page_value_invalid"
  | "page_handler_failed"
  | "route_already_handled"
  | "navigation_interrupted"
  | "response_not_json"
  | "navigation_failed"
  | "page_crashed"
  | "body_unavailable"
  | "download_failed"
  | "cdp_command_refused"
  | "secret_unbound"
  | "secret_unavailable"
  | "secret_domain_not_allowed"
  | "secret_input_refused"
  | "logged_out"
  | "human_pause_declined"
  | "session_not_savable"
  | "label_duplicate"
  | "label_unhandled"
  | "label_unknown"
  | "attempt_ended"
  | "browser_required"
  | "retry_requested"
  | "response_not_ok"
  | "response_unavailable"
  | "handler_lost"
  | "site_preferences_refused"
  | "run_id_invalid"
  | "run_conflict"
  | "run_ended"
  | "run_in_use"
  | "limit_raise_refused"
  | "run_state_unavailable"
  | "run_not_found"
  | "run_interrupted"
  | "model_not_found"
  | "model_capability_missing"
  | "model_handler_failed"
  | "model_unauthorized"
  | "model_funds_exhausted"
  | "model_price_unknown"
  | "model_unavailable"
  | "model_declined"
  | "model_context_exceeded"
  | "model_output_invalid"
  | "model_request_rejected"
  | "agent_tool_failed"
  | "agent_tool_invalid"
  | "success_check_threw"
  | "response_not_recorded"
  | "recording_invalid"
  | "scripted_model_exhausted"
  | "scripted_model_unused"
  | "hook_unknown"
  | "internal";

/** A code a newer engine sends and this SDK does not know is kept as received (G3). */
export type ErrorCode = KnownErrorCode | (string & {});

/** The settings layers of ADR 0028, lowest first. A program that is not a Recipe sets the `run` layer (ADR 0083). */
export type SettingsLayer = "default" | "operator_policy" | "configuration" | "recipe" | "run";

/** What set a value at the `run` layer; a call's argument beats `settings`. The environment sets no setting (ADR 0083). */
/** `registered`, the lowest: the only SDK handler of a kind that one setting chooses (ADR 0104). */
export type RunLayerOrigin = "argument" | "settings" | "flag" | "form" | "registered";

/** Where the limit that ended a call came from (G6). */
export interface LimitSource {
  /** The setting's canonical key, such as `timeout.action`. */
  readonly setting: string;
  readonly value: unknown;
  /** The layer that set the value. */
  readonly layer: SettingsLayer;
  /** Set when `layer` is `run`: a call's own argument, a `settings` option, a CLI flag or a form (ADRs 0080, 0083). */
  readonly origin?: RunLayerOrigin;
}

/** What a call acted on, rendered by the engine in this SDK's language and kept structured too. */
export interface ErrorTarget {
  /** As the program wrote it, such as `page.getByRole('button', { name: 'Next' })`. */
  readonly rendered: string;
  /** The target as data, for programs that inspect it. */
  readonly structured: unknown;
}

/** The base of every Goodfetch failure. Check `code` or `name`: `instanceof` fails across bundle copies. */
export declare abstract class GoodfetchError extends Error {
  readonly name: string;
  readonly code: ErrorCode;
  readonly group: ErrorGroup;
  readonly retryable: boolean;
  /** The SDK call, such as `locator.click`. */
  readonly operation: string;
  /** Finds the call in the Run trace. */
  readonly callId: string;
  readonly runId?: string;
  readonly target?: ErrorTarget;
  readonly url?: string;
  /** What was there instead. */
  readonly observed?: string;
  readonly limit?: LimitSource;
  /** What would change the outcome, naming setting keys or operations. */
  readonly fix?: string;
  /** The steps the engine took for this call. Never holds a Secret's value (ADR 0015). */
  readonly callLog: readonly string[];
  /** Per-code typed fields; `undefined` for codes without any. */
  readonly details?: unknown;
  /** What the call's model calls consumed before it failed, when it made any: a failed call's spend is never hidden (ADR 0101). */
  readonly usage?: ModelUsage;
  /** On an error `page.agent` throws after its task took steps: the steps and the partial answer (ADR 0100). */
  readonly agentTask?: AgentTaskProgress;
}

// The seven groups by remedy. A code this SDK does not know is thrown as its group's class.

/** The program is wrong: its call, its settings or its own handler code. Fix the program (G3). */
export declare class ProgramError extends GoodfetchError {
  readonly group: "program";
}
/** A policy, a Limit, a lock or a person refused. Change a setting, raise the Limit or ask the operator. */
export declare class RefusedError extends GoodfetchError {
  readonly group: "refused";
}
/** The page or site never reached the state the call needed. Look at the Run trace. */
export declare class PageError extends GoodfetchError {
  readonly group: "page";
}
/** The Model provider's answer was unusable. Change the model or the strategy. */
export declare class ModelError extends GoodfetchError {
  readonly group: "model";
}
/** The deadline passed. Raise the timeout its `limit` names, or wait for the right thing (#467).
 * A code named like its group (`deadline_exceeded`, `internal`) shares the group's class. */
export declare class DeadlineExceededError extends GoodfetchError {
  readonly group: "deadline_exceeded";
  /** Every timeout is the code `deadline_exceeded`, whatever the phase (ADR 0080), except a target still ambiguous at its deadline: `target_not_unique` (ADR 0085). */
  readonly details: DeadlineExceededDetails;
}
/** Something Goodfetch depends on is unreachable or gone. Usually retryable: retry or fix the setup. */
export declare class UnavailableError extends GoodfetchError {
  readonly group: "unavailable";
}
/** A bug in Goodfetch. Report it. */
export declare class InternalError extends GoodfetchError {
  readonly group: "internal";
}

// One generated class per code: the code in PascalCase plus `Error`.

export declare class ClosedError extends ProgramError {
  readonly code: "closed";
}

/**
 * Someone else stopped the call in flight: a person, another Client, or the program closing a
 * parent value. Not retryable; `observed` says who, and `fix` names resuming the Run, or the close
 * that stopped the call. The caller's own abort rejects with `signal.reason` instead and never
 * raises this (ADR 0080).
 */
export declare class CallCancelledError extends UnavailableError {
  readonly code: "call_cancelled";
}

/** Details of `handler_failed`: the user's own exception, as text, since the outcome outlives the process. */
export interface HandlerFailedDetails {
  readonly exceptionType: string;
  readonly exceptionMessage: string;
  /** The stack frames of the user's code. */
  readonly stack: readonly string[];
}
/**
 * A crawl handler threw something other than a GoodfetchError. `cause` holds the original in the
 * process that threw it. Not retryable, unless `crawl.retry.handler_failed` is on (ADR 0093).
 */
export declare class HandlerFailedError extends ProgramError {
  readonly code: "handler_failed";
  readonly details: HandlerFailedDetails;
}

/**
 * A listener threw (ADR 0081). It was removed, and this is thrown once, by the next call on the value
 * it was registered on or by the `close()` that closes that value, its own or a parent's; `cause`
 * holds the listener's exception. A call that throws it is not sent, and the value stays open; a
 * `close()` that throws it closes the value first. If the value is released before either, the SDK
 * logs it at error level instead.
 */
export declare class ListenerFailedError extends ProgramError {
  readonly code: "listener_failed";
  readonly details: { readonly event: string };
}

export declare class LimitReachedError extends RefusedError {
  readonly code: "limit_reached";
}

export declare class PageCheckFailedError extends PageError {
  readonly code: "page_check_failed";
}

/**
 * Details of `target_not_unique`. Uniqueness is one of the conditions the guard waits for: the call
 * throws this when its deadline passes with the target still ambiguous (ADR 0085), or at once for a
 * question that does not wait, such as `isVisible()`.
 */
export interface TargetNotUniqueDetails {
  /** Each match, rendered in this SDK's language. */
  readonly matches: readonly string[];
}
export declare class TargetNotUniqueError extends PageError {
  readonly code: "target_not_unique";
  readonly details: TargetNotUniqueDetails;
}

/** The Block causes of ADRs 0021, 0035, 0045 and 0048; a newer engine's cause is kept as received. */
export type BlockCause =
  | "rate_limited"
  | "challenge"
  | "ip_blocked"
  | "payment_required"
  | "consent_required"
  | (string & {});

export declare class BlockedError extends PageError {
  readonly code: "blocked";
  readonly details: { readonly blockCause: BlockCause };
}

// Outcomes are values.

/** Retries are not an outcome: every outcome counts its attempts and keeps the failed ones. */
export interface Attempts {
  readonly attempts: number;
  readonly attemptErrors: readonly GoodfetchError[];
}

/** Which crawl request an outcome is about. Outcomes span Labels, so the Payload is not typed here. */
export interface CrawlRequestRef {
  readonly id: string;
  readonly url: string;
  /** The Label's name. */
  readonly label: string;
  readonly depth: number;
}

/**
 * A crawl request's ending (ADR 0079; the noun is ADR 0093's). A kind a newer engine sends and this
 * SDK does not know arrives as `unknown`, with the kind kept as received in `raw` (G3). A program
 * reads them with `run.outcomes()`, and counts them in the Run's progress and report (ADR 0102).
 */
export type CrawlRequestOutcome = Attempts & { readonly request: CrawlRequestRef; readonly endedAt: Date } & (
    | {
        readonly kind: "succeeded";
        /** Per Dataset name, what its commit did to the Records it pushed (ADR 0103); empty when it pushed none. */
        readonly records: Readonly<Record<string, RecordCounts>>;
      }
    | { readonly kind: "failed"; readonly error: GoodfetchError }
    | { readonly kind: "skipped"; readonly reason: SkipReason; readonly message: string | null }
    | { readonly kind: "blocked"; readonly blockCause: BlockCause }
    | { readonly kind: "unknown"; readonly raw: string }
  );

/**
 * Why a crawl request was skipped (ADR 0093's full list). `message` carries a Hook's or a handler's
 * own text.
 * - `out_of_scope`: a redirect left the crawl's scope;
 * - `domain_policy`: a redirect or a Hook's rewrite reached a host the run's domain policy refuses (ADRs 0014, 0017);
 * - `duplicate`: a URL rule the Frontier learned after admitting it merged it with another (ADR 0005);
 * - `robots_txt`: a Disallow line of the origin's robots.txt, or a robots.txt answered with a 429 or a
 *   5xx, or unreachable, with no earlier copy, until the request's retries ran out (ADR 0086); also a
 *   handler's own navigation refused for it whose `SitePreferencesRefusedError` escaped the handler;
 * - `crawl_delay`: a Crawl-delay above `site_preferences.max_crawl_delay` (ADR 0086), also for an escaped refusal of a handler's navigation;
 * - `use_signals`: an AI-use signal that refuses the run's Purpose while `site_preferences.respect_use_signals` is on (ADR 0086), also for an escaped refusal of a handler's navigation;
 * - `hook`: a before-request Hook (ADR 0033);
 * - `handler`: the handler called `ctx.skip()`.
 */
export type SkipReason =
  | "out_of_scope"
  | "domain_policy"
  | "duplicate"
  | "robots_txt"
  | "crawl_delay"
  | "use_signals"
  | "hook"
  | "handler"
  | (string & {});

/** A Verdict, given by checks under ADR 0029's rules and never by the model's claim. Any other value is never `verified`. */
export type Verdict = "verified" | "rejected" | "needs-review" | (string & {});

/** How AI work ended. A failed ending throws instead. */
export type Ended =
  | { readonly kind: "finished" }
  | { readonly kind: "stopped"; readonly reason: StopReason };

/**
 * Why AI work stopped early (ADRs 0079, 0080, 0100): a Limit by name, after which the Run stops too;
 * a timeout, the Agent's own `timeout.agent` or the Run's; the Agent's step cap (`agent.max_steps`) or
 * stall rule (`agent.max_stalled_steps`), which stop only the task; or an abort by someone other than
 * the caller. `source` names the setting, its value and its layer (G6).
 */
export type StopReason =
  | { readonly kind: "limit"; readonly limit: string }
  | { readonly kind: "timeout"; readonly source: LimitSource }
  | { readonly kind: "max_steps"; readonly source: LimitSource }
  | { readonly kind: "no_progress"; readonly source: LimitSource }
  /** Who stopped it: a person in the Console, another Client, or the program closing the page or the Run. */
  | { readonly kind: "abort"; readonly by: "person" | "client" | "program" | (string & {}) }
  | { readonly kind: "unknown"; readonly raw: string };

/** What the model said about its own work: a signal, never proof (ADR 0013). */
export interface Reported {
  readonly claim: "done" | "gave_up" | (string & {});
  readonly message: string;
}

/**
 * What AI work gathered before an early end, never typed as complete (ADR 0100): each top-level field
 * of an object, or each element of a list, valid against its own part of the schema and Grounded.
 */
export type PartialOf<T> = T extends readonly (infer Element)[] ? readonly Element[] : T extends object ? Partial<T> : never;

/**
 * The result of AI work that can claim success: an Agent's task, replayed toward a goal or not.
 * `AgentResult` (ADR 0100) builds on it.
 */
export interface AiWorkResult<T> {
  readonly ended: Ended;
  readonly verdict: Verdict;
  /** Absent when no model claimed anything: the work stopped before a claim, or was replayed whole. */
  readonly reported: Reported | undefined;
  /** Present only once the work finished with a complete `T`; a value of `T` is always complete. */
  readonly output: T | undefined;
  /** What was gathered before an early end, set only when `output` is not. */
  readonly partial: PartialOf<T> | undefined;
  /** Returns the output when the Verdict is `verified`; otherwise throws `NotVerifiedError`. */
  requireVerified(): T;
}

export interface NotVerifiedDetails {
  readonly verdict: Verdict;
  readonly ended: Ended;
  readonly reported: Reported | undefined;
  /** Why it is not `verified`: failed, missing or switched-off checks, uncertain Effects. */
  readonly reasons: readonly string[];
}
export declare class NotVerifiedError extends PageError {
  readonly code: "not_verified";
  readonly details: NotVerifiedDetails;
}
