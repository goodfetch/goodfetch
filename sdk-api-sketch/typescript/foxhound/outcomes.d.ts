// A crawl request's outcomes and Foxhound's own error of a handler (ADRs 0079, 0093). The error
// groups and every class of Collie's codes are the Collie client package's (ADR 0095).

import type {
  BlockCause,
  GoodfetchError,
  HandlerFailedDetails,
  KnownErrorCode as CollieKnownErrorCode,
  ProgramError,
} from "@goodfetch/collie-client";
import type { RecordCounts } from "./output.js";

/**
 * A crawl handler threw something other than a GoodfetchError. `cause` holds the original in the
 * process that threw it. Not retryable, unless `crawl.retry.handler_failed` is on (ADR 0093).
 */
export declare class HandlerFailedError extends ProgramError {
  readonly code: "handler_failed";
  readonly details: HandlerFailedDetails;
}

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
 * - `duplicate`: a URL rule the Frontier learned after admitting it merged it with another (ADR 0005), or its
 *   redirect reached a URL whose key the Frontier already admitted, and its handler did not run (ADR 0120);
 * - `robots_txt`: a Disallow line of the origin's robots.txt, or a robots.txt answered with a 429 or a
 *   5xx, or unreachable, with no earlier copy, until the request's retries ran out (ADR 0086); also a
 *   handler's own navigation refused for it whose `SitePreferencesRefusedError` escaped the handler;
 * - `crawl_delay`: a Crawl-delay above `site_preferences.max_crawl_delay` (ADR 0086), also for an escaped refusal of a handler's navigation;
 * - `use_signals`: an AI-use signal that refuses the run's Purpose while `site_preferences.respect_use_signals` is on (ADR 0086), also for an escaped refusal of a handler's navigation;
 * - `hook`: a before-request Hook (ADR 0033);
 * - `handler`: the handler called `ctx.skip()`;
 * - `model_allowance`: a `ModelAllowanceReachedError` escaped the handler once the Run's Model allowance
 *   was reached (ADR 0107); the Run then ends `incomplete` though it finishes;
 * - `content_type`: a response whose type the Label's `crawl.content_types` does not list, skipped
 *   before its body is read, with a message naming the type (ADR 0119).
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
  | "model_allowance"
  | "content_type"
  | (string & {});


/**
 * The Error codes this sketch shows: Collie's, which every Foxhound engine also sends, and Foxhound's
 * own. Their classes extend the client package's group classes, so a Collie program that meets one
 * gets its group's class with the code kept (ADRs 0079, 0099).
 */
export type KnownErrorCode =
  | CollieKnownErrorCode
  | "handler_failed"
  | "hook_failed"
  | "hook_ineffective"
  | "hook_duplicate"
  | "hook_unknown"
  | "header_forbidden"
  | "fetch_handler_failed"
  | "sink_handler_failed"
  | "payload_invalid"
  | "payload_schema_changed"
  | "label_duplicate"
  | "label_unhandled"
  | "label_unknown"
  | "attempt_ended"
  | "browser_required"
  | "responses_not_kept"
  | "response_not_document"
  | "retry_requested"
  | "response_not_ok"
  | "response_unavailable"
  | "handler_lost"
  | "site_preferences_refused"
  | "dataset_duplicate"
  | "dataset_unknown"
  | "dataset_key_invalid"
  | "record_invalid";
/** A code a newer engine sends and this SDK does not know is kept as received (G3). */
export type ErrorCode = KnownErrorCode | (string & {});
