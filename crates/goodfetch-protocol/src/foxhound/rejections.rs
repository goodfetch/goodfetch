//! A crawl's rejection tables (ADR 0103): each Dataset's Records that were
//! rejected rather than committed, so a rejection is never lost. A Record
//! the engine's check refuses at `crawl_request.push` is raised to the
//! handler and its rejection staged with the attempt; it enters the table
//! once, when that attempt commits, numbered from 1 per Dataset in commit
//! order, with no gaps. `run.rejected_records` reads one Dataset's table
//! from the first entry or after a number, and follows the Run live as
//! `run.outcomes` does. Foxhound's own.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::catalog::{
    Entries, Operation, OperationClass, OperationKind, ProtocolVersion, Reach, Stability, schema_of,
};
use crate::foxhound::dataset::RecordIssue;
use crate::run::RunId;

const ARRIVED_IN: ProtocolVersion = ProtocolVersion { major: 0, minor: 1 };

/// The most entries one page of `run.rejected_records` carries.
pub const REJECTED_RECORDS_PAGE: usize = 256;

/// One entry of a Dataset's rejection table.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RejectedRecord {
    /// Its place in the Dataset's rejection table, from 1, in commit
    /// order with no gaps, which `run.rejected_records`'s `after` takes.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub number: u64,
    /// The Record as JSON, as it was pushed, never parsed by the schema it
    /// failed.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub data: Value,
    /// Why it was rejected.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub reason: String,
    /// Each issue the check found, by its path, a JSON Pointer into the
    /// Record; empty when a Hook rejected it with none.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub issues: Vec<RecordIssue>,
    /// The Hook that rejected it; absent when the engine's own check did.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub hook: Option<String>,
    /// The ID of the crawl request whose attempt pushed it.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub request_id: String,
    /// That request's URL.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub url: String,
}

/// What `run.rejected_records` takes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RunRejectedRecordsParams {
    /// The crawl's Run.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub run_id: RunId,
    /// The Dataset whose rejection table to read.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub dataset: String,
    /// The number of the last entry a reader handled; absent, from the
    /// first.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub after: Option<u64>,
}

/// One page of `run.rejected_records`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RejectedRecords {
    /// The entries after `after`, in number order.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub records: Vec<RejectedRecord>,
    /// The number the next page starts after.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub next: u64,
    /// Whether the table is read to its end: the Run has ended, or has no
    /// live writer, and no entry follows `next`.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub ended: bool,
}

/// `run.rejected_records`.
pub const RUN_REJECTED_RECORDS: Operation = Operation {
    name: "run.rejected_records",
    description: "Reads one Dataset's rejection table from a crawl's committed state: each Record rejected rather than committed, numbered from 1 in commit order, with the Record as JSON, the reason, each issue's path, the Hook when one rejected it, and the request's ID and URL. It starts at the first entry, or after `after`, the number a reader last handled. While the Run is live and nothing follows, it waits for the next commit; a page says `ended` once the Run has ended, or has no live writer, and no entry follows. A reader never slows the Run. A Dataset the crawl does not declare fails with `dataset_unknown`.",
    class: OperationClass::Read,
    kind: OperationKind::RoundTrip,
    reach: Reach::AllSurfaces,
    params: schema_of::<RunRejectedRecordsParams>,
    result: schema_of::<RejectedRecords>,
    events: &[],
    cancellable: true,
    input: None,
    bound: None,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// Every entry this module defines.
pub const ENTRIES: Entries = Entries {
    defined_in: module_path!(),
    operations: &[RUN_REJECTED_RECORDS],
    events: &[],
    handler_requests: &[],
    error_codes: &[],
    plugin_kinds: &[],
    metrics: &[],
    settings: &[],
    hook_stages: &[],
    shared_type_values: &[],
};
