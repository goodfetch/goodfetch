//! The Error codes a crawl's Hooks in code fail with (ADRs 0104 and 0105),
//! declared once here for the tickets that raise them. A Hook is named by
//! the name it carries, `addToken` or `before_request#2`, and its stage by
//! the catalog's stage name, such as `before_request`. Foxhound's own; a
//! Hook that raises fails with Collie's `hook_failed` ([`crate::hook`]),
//! since a Collie Run's Script Hook does too (ADR 0051).

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

/// The Error codes of a crawl's Hooks in code that only a crawl raises.
pub const ENTRIES: Entries = Entries {
    defined_in: module_path!(),
    operations: &[],
    events: &[],
    handler_requests: &[],
    error_codes: &[HOOK_DUPLICATE, HOOK_INEFFECTIVE, HOOK_UNKNOWN],
    plugin_kinds: &[],
    metrics: &[],
    settings: &[],
    hook_stages: &[],
    shared_type_values: &[],
};

#[cfg(test)]
mod tests;
