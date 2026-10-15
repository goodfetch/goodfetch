//! Foxhound's own Error codes (ADR 0099): free constants of the client crate's open
//! [`ErrorCode`](goodfetch_collie_client::ErrorCode), equal with `==` and in a `match` to a code read
//! from the wire. Collie's codes are its associated constants (`ErrorCode::TARGET_NOT_UNIQUE`).

use goodfetch_collie_client::ErrorCode;

/// `handler_failed`: a crawl handler returned an error that is not a Goodfetch [`Error`].
pub const HANDLER_FAILED: ErrorCode = ErrorCode::new("handler_failed");
/// `hook_failed` (ADRs 0033, 0104).
pub const HOOK_FAILED: ErrorCode = ErrorCode::new("hook_failed");
/// `header_forbidden`: a before-request Hook set a header the engine owns, raised inside
/// `hook_failed` (ADR 0104).
pub const HEADER_FORBIDDEN: ErrorCode = ErrorCode::new("header_forbidden");
/// `hook_ineffective`: a Hook that can never run (ADR 0104).
pub const HOOK_INEFFECTIVE: ErrorCode = ErrorCode::new("hook_ineffective");
/// `hook_duplicate`: a second Hook of one name on one crawl (ADR 0104).
pub const HOOK_DUPLICATE: ErrorCode = ErrorCode::new("hook_duplicate");
/// `fetch_handler_failed`: the program's Fetch backend failed with its own error (ADR 0104).
pub const FETCH_HANDLER_FAILED: ErrorCode = ErrorCode::new("fetch_handler_failed");
/// `sink_handler_failed`: the program's Sink failed with its own error, retried on ADR 0008's
/// schedule (ADR 0104).
pub const SINK_HANDLER_FAILED: ErrorCode = ErrorCode::new("sink_handler_failed");
/// `payload_invalid`: a Payload failed its Label's schema, was not JSON, or was too large (#469).
pub const PAYLOAD_INVALID: ErrorCode = ErrorCode::new("payload_invalid");
/// `payload_schema_changed`: a resume found Payloads that fail their Label's changed schema (#469).
pub const PAYLOAD_SCHEMA_CHANGED: ErrorCode = ErrorCode::new("payload_schema_changed");
/// `label_duplicate`: a second Label of one name, or a second handler for one Label (ADR 0093).
pub const LABEL_DUPLICATE: ErrorCode = ErrorCode::new("label_duplicate");
/// `label_unhandled`: the Run's start found a Label with no handler (ADR 0093).
pub const LABEL_UNHANDLED: ErrorCode = ErrorCode::new("label_unhandled");
/// `label_unknown`: a Label the crawl does not declare, at an enqueue or a resume (ADR 0093).
pub const LABEL_UNKNOWN: ErrorCode = ErrorCode::new("label_unknown");
/// `attempt_ended`: a call on the context of an attempt that has ended; returned by
/// `ctx.skip()` and `ctx.retry()`, and consumed by the helper (ADR 0093).
pub const ATTEMPT_ENDED: ErrorCode = ErrorCode::new("attempt_ended");
/// `browser_required`: a call only a browser can serve while `fetch.tier` forces a tier without
/// one (ADR 0093).
pub const BROWSER_REQUIRED: ErrorCode = ErrorCode::new("browser_required");
/// `responses_not_kept`: `ctx.kept_response` or `ctx.kept_responses` on a request whose Label
/// keeps nothing; the fix names `fetch.kept_responses.urls` (ADR 0110).
pub const RESPONSES_NOT_KEPT: ErrorCode = ErrorCode::new("responses_not_kept");
/// `response_not_document`: a call on the document of a crawl page whose response the Label
/// accepted but is no HTML document (a PDF, JSON, an image). It never escalates; the fix says to
/// read `ctx.response()` (ADR 0119). Not retryable.
pub const RESPONSE_NOT_DOCUMENT: ErrorCode = ErrorCode::new("response_not_document");
/// `retry_requested`: an attempt the handler ended with `ctx.retry()` (ADR 0093).
pub const RETRY_REQUESTED: ErrorCode = ErrorCode::new("retry_requested");
/// `response_not_ok`: a final 4xx that is no Block cause and not in `crawl.handle_statuses`;
/// not retryable (ADR 0093).
pub const RESPONSE_NOT_OK: ErrorCode = ErrorCode::new("response_not_ok");
/// `response_unavailable`, a page error: a final 5xx or 408 after the request's retries;
/// retryable (ADR 0093).
pub const RESPONSE_UNAVAILABLE: ErrorCode = ErrorCode::new("response_unavailable");
/// `handler_lost`: the program's process or connection was lost mid-attempt (a crash, a signal
/// it does not handle, or a linked engine that died with it, counted on resume from the Run's
/// committed state); retryable, and counted against `crawl.retry.max`. The program's own stop
/// counts nothing (ADR 0093).
pub const HANDLER_LOST: ErrorCode = ErrorCode::new("handler_lost");
/// `site_preferences_refused`: a handler's own navigation that Site preferences refuse (ADR 0093).
pub const SITE_PREFERENCES_REFUSED: ErrorCode = ErrorCode::new("site_preferences_refused");
/// `hook_unknown`: the testing kit was asked for a Hook the crawl does not hold (ADR 0105).
pub const HOOK_UNKNOWN: ErrorCode = ErrorCode::new("hook_unknown");
/// `record_invalid`: a pushed Record failed its Dataset's schema, was not JSON, or lacked its key's
/// fields; also a reader whose native parse rejects a Record (ADR 0103).
pub const RECORD_INVALID: ErrorCode = ErrorCode::new("record_invalid");
/// `dataset_duplicate`: a second Dataset of one name in one crawl (ADR 0103).
pub const DATASET_DUPLICATE: ErrorCode = ErrorCode::new("dataset_duplicate");
/// `dataset_unknown`: a push to a Dataset another crawl declared, or a resume whose crawl no longer
/// declares a Dataset that holds Records (ADR 0103).
pub const DATASET_UNKNOWN: ErrorCode = ErrorCode::new("dataset_unknown");
/// `dataset_key_invalid`: a key naming a field the schema does not require (ADR 0103).
pub const DATASET_KEY_INVALID: ErrorCode = ErrorCode::new("dataset_key_invalid");

// Each Foxhound code's details type (ADR 0099).
impl goodfetch_collie_client::ErrorDetails for crate::outcomes::HandlerFailed {}
impl goodfetch_collie_client::ErrorDetails for crate::hooks::HookFailed {}
impl goodfetch_collie_client::ErrorDetails for crate::hooks::HeaderForbidden {}
impl goodfetch_collie_client::ErrorDetails for crate::hooks::HookIneffective {}
impl goodfetch_collie_client::ErrorDetails for crate::hooks::HookDuplicate {}
impl goodfetch_collie_client::ErrorDetails for crate::data::PayloadInvalid {}
impl goodfetch_collie_client::ErrorDetails for crate::data::PayloadSchemaChanged {}
impl goodfetch_collie_client::ErrorDetails for crate::output::RecordInvalid {}
impl goodfetch_collie_client::ErrorDetails for crate::output::DatasetDuplicate {}
impl goodfetch_collie_client::ErrorDetails for crate::output::DatasetUnknown {}
impl goodfetch_collie_client::ErrorDetails for crate::output::DatasetKeyInvalid {}
