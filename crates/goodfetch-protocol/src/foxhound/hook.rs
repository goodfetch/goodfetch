//! The Error codes a crawl's Hooks in code fail with (ADRs 0104 and 0105),
//! declared once here for the tickets that raise them. A Hook is named by
//! the name it carries, `addToken` or `before_request#2`, and its stage by
//! the catalog's stage name, such as `before_request`. Foxhound's own.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::catalog::{Entries, ErrorCodeDefinition, ErrorGroup, ProtocolVersion};

const ARRIVED_IN: ProtocolVersion = ProtocolVersion { major: 0, minor: 1 };

/// A second Hook of one name on one crawl (ADR 0104).
pub const HOOK_DUPLICATE: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "hook_duplicate",
    description: "The crawl declares a second Hook of a name one of its Hooks already carries, so the declaration failed at once, with no I/O: a name identifies one Hook, in `hook_failed`, in the Run trace and for a test's `runHook`. The details name the Hook and its stage. Give each Hook its own `name`.",
    group: ErrorGroup::Program,
    retryable: false,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// What `hook_duplicate` details.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct HookDuplicateDetails {
    /// The name both Hooks carry.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub hook: String,
    /// The stage of the second Hook, the one refused.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub stage: String,
}

/// A Hook that can never run (ADR 0104).
pub const HOOK_INEFFECTIVE: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "hook_ineffective",
    description: "A Hook can never run, so nothing was done: its `labels` or `datasets` list is empty, which fails at once with no I/O, or, when the Run starts, it is a navigation Hook all of whose Labels force a tier that does not navigate (`http`, `discovered_api` or `browser_fetch`) through `fetch.tier`, so the Run was not started. The details name the Hook and its stage. Scope it to a Label or Dataset it can see, or let one of its Labels navigate.",
    group: ErrorGroup::Program,
    retryable: false,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// What `hook_ineffective` details.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct HookIneffectiveDetails {
    /// The Hook refused.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub hook: String,
    /// Its stage.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub stage: String,
}

/// A Hook that raised (ADRs 0104, 0079 and 0109).
pub const HOOK_FAILED: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "hook_failed",
    description: "A Hook raised, so its work failed: an attempt commits nothing, and one that failed at the Record stage commits none of what it staged. The message always names the exception. The details name the stage, the Hook, the request's Label (`null` at a lifecycle stage), the exception's type, message and stack, and the Goodfetch code raised inside (`null` for the program's own exception). A wrapped code is retried when its own retryable flag says so; the program's own exception only when `crawl.retry.handler_failed` is on, since a bug fails the same way every time. Either retry draws on `crawl.retry.max`.",
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
    /// request.
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

/// A test's `runHook` naming a Hook the crawl does not hold (ADR 0105).
pub const HOOK_UNKNOWN: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "hook_unknown",
    description: "A test's `runHook` names a Hook the crawl does not hold, so no Hook ran. The details name the Hook asked for and the closest name the crawl holds, if it holds any. Use a name the crawl's Hooks carry, which `name` sets.",
    group: ErrorGroup::Program,
    retryable: false,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// What `hook_unknown` details.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct HookUnknownDetails {
    /// The name asked for.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub hook: String,
    /// The closest name the crawl's Hooks carry, or none when it has no
    /// Hook.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub closest: Option<String>,
}

/// The Error codes of a crawl's Hooks in code.
pub const ENTRIES: Entries = Entries {
    defined_in: module_path!(),
    operations: &[],
    events: &[],
    handler_requests: &[],
    error_codes: &[HOOK_DUPLICATE, HOOK_INEFFECTIVE, HOOK_FAILED, HOOK_UNKNOWN],
    plugin_kinds: &[],
    metrics: &[],
    settings: &[],
    hook_stages: &[],
    shared_type_values: &[],
};

#[cfg(test)]
mod tests;
