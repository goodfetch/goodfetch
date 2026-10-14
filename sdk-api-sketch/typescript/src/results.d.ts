// Results, outcomes and errors (ADR 0079, decided in #466).
// Failures throw a GoodfetchError: one class per Error code under one of seven groups by remedy.
// Expected outcomes are values in the return type, and no option changes which (G1, S2).
// The codes below are a representative subset: each area's spec lists its codes, their groups
// and their details in goodfetch-protocol, and codegen writes one class per code.

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
  | "handler_failed"
  | "hook_failed"
  | "limit_reached"
  | "page_check_failed"
  | "target_not_unique"
  | "blocked"
  | "not_verified"
  | "deadline_exceeded"
  | "plugin_crashed"
  | "internal";

/** A code a newer engine sends and this SDK does not know is kept as received (G3). */
export type ErrorCode = KnownErrorCode | (string & {});

/** The settings layers of ADR 0028, lowest first. */
export type SettingsLayer = "default" | "operator_policy" | "configuration" | "recipe" | "run";

/** Where the limit that ended a call came from (G6). */
export interface LimitSource {
  /** The setting key, such as `timeouts.action`. */
  readonly setting: string;
  readonly value: unknown;
  /** The layer that set the value. */
  readonly layer: SettingsLayer;
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
/** The deadline passed. Raise it, or wait for the right thing; deadlines are #467's.
 * A code named like its group (`deadline_exceeded`, `internal`) shares the group's class. */
export declare class DeadlineExceededError extends GoodfetchError {
  readonly group: "deadline_exceeded";
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

/** Details of `handler_failed`: the user's own exception, as text, since the outcome outlives the process. */
export interface HandlerFailedDetails {
  readonly exceptionType: string;
  readonly exceptionMessage: string;
  /** The stack frames of the user's code. */
  readonly stack: readonly string[];
}
/** A handler threw something other than a GoodfetchError. `cause` holds the original in the process that threw it. */
export declare class HandlerFailedError extends ProgramError {
  readonly code: "handler_failed";
  readonly details: HandlerFailedDetails;
}

export declare class LimitReachedError extends RefusedError {
  readonly code: "limit_reached";
}

export declare class PageCheckFailedError extends PageError {
  readonly code: "page_check_failed";
}

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

/**
 * A crawl request's ending. The noun "request" is #477's to settle. A kind a newer engine sends
 * and this SDK does not know arrives as `unknown`, with the kind kept as received in `raw` (G3).
 */
export type RequestOutcome = Attempts &
  (
    | { readonly kind: "succeeded" }
    | { readonly kind: "failed"; readonly error: GoodfetchError }
    | { readonly kind: "skipped"; readonly reason: SkipReason }
    | { readonly kind: "blocked"; readonly blockCause: BlockCause }
    | { readonly kind: "unknown"; readonly raw: string }
  );

/** Why a request was skipped; the full list is #477's and #480's. */
export type SkipReason =
  | "out_of_scope"
  | "duplicate"
  | "site_preferences"
  | "skipped_by_hook"
  | "skipped_by_handler"
  | (string & {});

/** A Verdict, given by checks under ADR 0029's rules and never by the model's claim. Any other value is never `verified`. */
export type Verdict = "verified" | "rejected" | "needs-review" | (string & {});

/** How AI work ended. A failed ending throws instead. */
export type Ended =
  | { readonly kind: "finished" }
  | { readonly kind: "stopped"; readonly reason: StopReason };

/** Why AI work stopped early: a Limit by name, or an abort. */
export type StopReason =
  | { readonly kind: "limit"; readonly limit: string }
  | { readonly kind: "abort" }
  | { readonly kind: "unknown"; readonly raw: string };

/** What the model said about its own work: a signal, never proof (ADR 0013). */
export interface Reported {
  readonly claim: "done" | "gave_up" | (string & {});
  readonly message: string;
}

/**
 * The result of AI work that can claim success: an Agent's task, or a Plan replayed toward a goal.
 * #474 and #473 build their result types on it and decide `partial`'s exact shape.
 */
export interface AiWorkResult<T> {
  readonly ended: Ended;
  readonly verdict: Verdict;
  readonly reported: Reported;
  /** Present only once the work finished with a complete `T`; a value of `T` is always complete. */
  readonly output: T | undefined;
  /** What was gathered before an early end, never typed as complete. */
  readonly partial: Partial<T> | undefined;
  /** Returns the output when the Verdict is `verified`; otherwise throws `NotVerifiedError`. */
  requireVerified(): T;
}

export interface NotVerifiedDetails {
  readonly verdict: Verdict;
  readonly ended: Ended;
  readonly reported: Reported;
  /** Why it is not `verified`: failed, missing or switched-off checks, uncertain Effects. */
  readonly reasons: readonly string[];
}
export declare class NotVerifiedError extends PageError {
  readonly code: "not_verified";
  readonly details: NotVerifiedDetails;
}
