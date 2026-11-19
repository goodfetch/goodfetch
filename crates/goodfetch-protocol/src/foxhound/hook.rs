//! A crawl's Hooks in code (ADR 0104): how `crawl.start` declares them,
//! the `crawl_hook.handle` request that asks the program to run one, and
//! the Error codes they fail with (ADRs 0104 and 0105). A Hook is named by
//! the name it carries, `addToken` or `before_request#2`, and its stage by
//! the catalog's stage name, such as `before_request`. Foxhound's own; a
//! Hook that raises fails with Collie's `hook_failed` ([`crate::hook`]),
//! since a Collie Run's Script Hook does too (ADR 0051).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::catalog::{
    Entries, ErrorCodeDefinition, ErrorGroup, HandlerRequest, ProtocolVersion, Stability, schema_of,
};
use crate::connection::WaitedFor;
use crate::foxhound::crawl::{CrawlRequest, PushedRecord};
use crate::run::RunId;

const ARRIVED_IN: ProtocolVersion = ProtocolVersion { major: 0, minor: 1 };

/// A stage whose answer changes the work, where a crawl's Hooks in code
/// run (ADR 0104), by the catalog's name. The observe-only stages reach
/// Plugins and Script Hooks only.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
#[schemars(extend("open" = true))]
pub enum CrawlHookStage {
    /// Before each fetch of a request, its retries included.
    BeforeRequest,
    /// After the request's final response, redirects followed.
    AfterResponse,
    /// Before a navigation to the request's URL, on the tiers that
    /// navigate.
    BeforeNavigation,
    /// After that navigation.
    AfterNavigation,
    /// Just before the handler, with its page, on every tier.
    PageReady,
    /// Before each Record the attempt staged commits.
    BeforeRecordCommit,
}

impl CrawlHookStage {
    /// Its name in the catalog, on the wire and in `hook_failed`.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::BeforeRequest => "before_request",
            Self::AfterResponse => "after_response",
            Self::BeforeNavigation => "before_navigation",
            Self::AfterNavigation => "after_navigation",
            Self::PageReady => "page_ready",
            Self::BeforeRecordCommit => "before_record_commit",
        }
    }

    /// What a call to one of its Hooks past `timeout.hook` waited for, as
    /// `deadline_exceeded` names it: the answer of a Hook at this stage.
    pub const fn waited_for(self) -> WaitedFor {
        WaitedFor::named(match self {
            Self::BeforeRequest => "answer of a `before_request` Hook",
            Self::AfterResponse => "answer of an `after_response` Hook",
            Self::BeforeNavigation => "answer of a `before_navigation` Hook",
            Self::AfterNavigation => "answer of an `after_navigation` Hook",
            Self::PageReady => "answer of a `page_ready` Hook",
            Self::BeforeRecordCommit => "answer of a `before_record_commit` Hook",
        })
    }
}

/// One Hook in code a crawl declares, in `crawl.start`'s `hooks`, which
/// list them in the order declared: per stage, they run in that order,
/// after the Hooks named in settings (ADR 0104).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CrawlHookDeclaration {
    /// Its name, which `crawl_hook.handle` and `hook_failed` carry.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub name: String,
    /// The stage it runs at.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub stage: CrawlHookStage,
    /// The Labels whose requests it runs for; absent for every Label.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub labels: Option<Vec<String>>,
    /// At `before_record_commit`, the Datasets whose Records it runs for;
    /// absent for every Dataset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub datasets: Option<Vec<String>>,
}

/// What `crawl_hook.handle` gives the program: one call of one Hook, for
/// one attempt of a work item, under its lease.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CrawlHookHandleParams {
    /// The crawl's Run.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub run_id: RunId,
    /// The Hook's name, as the crawl declared it.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub hook: String,
    /// The stage it runs at.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub stage: CrawlHookStage,
    /// The work item.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub crawl_request: CrawlRequest,
    /// Which attempt this is, from 1.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub attempt: u32,
    /// The lease's epoch, which only goes up (ADR 0003).
    #[schemars(extend("arrived_in" = "0.1"))]
    pub lease_epoch: u64,
    /// At `before_record_commit`, the Record about to commit and its
    /// Dataset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub record: Option<PushedRecord>,
}

/// What a Hook answers once it returned: nothing yet, the work going on
/// unchanged.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CrawlHookHandled {}

/// `crawl_hook.handle`.
pub const CRAWL_HOOK_HANDLE: HandlerRequest = HandlerRequest {
    name: "crawl_hook.handle",
    description: "Asks the program to run one of the crawl's Hooks in code at its stage, for one attempt of a work item under its lease; a stage none of the crawl's Hooks is scoped to sends none. Per stage, the Hooks run in the order the crawl declared them, each after the one before answered. `timeout.hook`, which a Label may set for its requests, bounds each call: past it, the engine sends `$/cancel` for the request and the attempt fails with `deadline_exceeded`, whose `limit` names `timeout.hook` and whose `waited_for` names the stage. A failure fails the attempt with `hook_failed`, naming the stage, the Hook and the request's Label, the program's exception carried as its type, message and stack, and a Goodfetch Error code raised inside as `wrapped_code`; the program's own exception is retried only under `crawl.retry.handler_failed`, a wrapped code as its retryable flag says, and both draw on `crawl.retry.max` with the handler's failures. At `before_record_commit`, a failure discards everything the attempt staged, so it commits nothing. A failure with the Error code `call_cancelled` by the program with nothing closed gives the request back, as for `crawl_request.handle`.",
    params: schema_of::<CrawlHookHandleParams>,
    result: schema_of::<CrawlHookHandled>,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

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

/// The request that runs a crawl's Hook in code, and the Error codes of
/// those Hooks that only a crawl raises.
pub const ENTRIES: Entries = Entries {
    defined_in: module_path!(),
    operations: &[],
    events: &[],
    handler_requests: &[CRAWL_HOOK_HANDLE],
    error_codes: &[HOOK_DUPLICATE, HOOK_INEFFECTIVE, HOOK_UNKNOWN],
    plugin_kinds: &[],
    metrics: &[],
    settings: &[],
    hook_stages: &[],
    shared_type_values: &[],
};

#[cfg(test)]
mod tests;
