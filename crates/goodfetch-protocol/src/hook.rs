//! The Error code a Hook that raises fails with (ADRs 0033, 0051, 0079 and
//! 0104): a crawl's Hook in code, or a Collie Run's Script Hook, which
//! names the `script_*` code it wraps. The codes only a crawl's Hooks raise
//! are Foxhound's own, in [`crate::foxhound::hook`].

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::catalog::{Entries, ErrorCodeDefinition, ErrorGroup, ProtocolVersion};

const ARRIVED_IN: ProtocolVersion = ProtocolVersion { major: 0, minor: 1 };

/// A Hook that raised (ADRs 0104, 0079 and 0109).
pub const HOOK_FAILED: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "hook_failed",
    description: "A Hook raised, so its work failed: an attempt commits nothing, and one that failed at the Record stage commits none of what it staged. The message always names the exception. The details name the stage, the Hook, the request's Label (`null` at a lifecycle stage or outside a crawl), the exception's type, message and stack, and the Goodfetch code raised inside (`null` for the program's own exception). A wrapped code is retried when its own retryable flag says so; in a crawl, the program's own exception only when `crawl.retry.handler_failed` is on, since a bug fails the same way every time, and either retry draws on `crawl.retry.max`.",
    group: ErrorGroup::Program,
    retryable: true,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// What `hook_failed` details: the Hook, and the exception as
/// `handler_failed`'s details name a handler's (ADR 0079's notes of
/// 2026-10-24 and 2026-10-27).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct HookFailedDetails {
    /// The stage the Hook ran at.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub stage: String,
    /// The Hook that raised.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub hook: String,
    /// The request's Label, or none at a lifecycle stage, which has no
    /// request, and outside a crawl.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub label: Option<String>,
    /// The exception's type name: `BoxError` or `panic` from Rust (ADR
    /// 0109).
    #[schemars(extend("arrived_in" = "0.1"))]
    pub exception_type: String,
    /// The exception's message.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub exception_message: String,
    /// The frames of the program's code, as one text; empty from Rust.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub exception_stack: String,
    /// The Goodfetch code raised inside the Hook, such as
    /// `header_forbidden`, or none for the program's own exception.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub wrapped_code: Option<String>,
}

/// The Error code of a Hook that raised.
pub const ENTRIES: Entries = Entries {
    defined_in: module_path!(),
    operations: &[],
    events: &[],
    handler_requests: &[],
    error_codes: &[HOOK_FAILED],
    plugin_kinds: &[],
    metrics: &[],
    settings: &[],
    hook_stages: &[],
    shared_type_values: &[],
};

#[cfg(test)]
mod tests;
