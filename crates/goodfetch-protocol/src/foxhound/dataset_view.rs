//! How a person surface reads a crawl's Datasets (spec #593, stories 204, 205
//! and 233; ADRs 0031 and 0103): `dataset.list`, each Dataset a crawl
//! declares with its JSON Schema, its key and how many Records were
//! committed, dropped and rejected; and `dataset.page`, one page of a
//! Dataset's committed Records, paged by key, with filtering (the `wall`
//! marker of ADR 0052 included), sorting and column selection done in the
//! engine, so a Dataset of millions of Records opens at once. A page reads
//! the Records committed when its first page was read, the snapshot its
//! `through` names, so Records committed meanwhile never make a page
//! repeat or skip one. Foxhound's own.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::catalog::{
    Entries, Operation, OperationClass, OperationKind, ProtocolVersion, Reach, Stability, schema_of,
};
use crate::foxhound::dataset::DatasetRecord;
use crate::run::RunId;

const ARRIVED_IN: ProtocolVersion = ProtocolVersion { major: 0, minor: 1 };

/// The most Records one page of `dataset.page` carries, and its `limit`'s
/// ceiling.
pub const DATASET_PAGE_MOST: u32 = 1000;

/// How many Records a page of `dataset.page` carries without a `limit`.
pub const DATASET_PAGE_DEFAULT: u32 = 100;

/// What `dataset.list` takes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DatasetListParams {
    /// The crawl's Run.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub run_id: RunId,
}

/// One Dataset a crawl declares, with what its crawl kept of it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DatasetSummary {
    /// Its name.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub name: String,
    /// The JSON Schema its Records are checked against; none takes any
    /// JSON object.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub schema: Option<Value>,
    /// The top-level fields that identify one Record; empty when none do.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub key: Vec<String>,
    /// How many Records the crawl committed into it.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub committed: u64,
    /// How many a Hook dropped before their commit.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub dropped: u64,
    /// How many were rejected into its rejection table.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub rejected: u64,
}

/// What `dataset.list` answers.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DatasetList {
    /// Each Dataset the crawl declares, in the order declared.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub datasets: Vec<DatasetSummary>,
}

/// `dataset.list`.
pub const DATASET_LIST: Operation = Operation {
    name: "dataset.list",
    description: "Lists the Datasets a crawl's Run declares, in the order declared, each with its JSON Schema, its key and how many Records the crawl committed into it, a Hook dropped and were rejected into its rejection table, read from the Run's committed state. A Run no crawl wrote declares none.",
    class: OperationClass::Read,
    kind: OperationKind::RoundTrip,
    reach: Reach::AllSurfaces,
    params: schema_of::<DatasetListParams>,
    result: schema_of::<DatasetList>,
    events: &[],
    cancellable: false,
    input: None,
    bound: None,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// Which Records a page takes by a Wall read past (ADR 0052).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum WallFilter {
    /// Only Records whose page was read past a Wall: their Provenance
    /// carries the `wall` marker.
    With,
    /// Only Records whose page met no Wall.
    Without,
}

/// What one top-level field of a Record must hold.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum FieldCondition {
    /// The field equals this JSON value.
    Equals {
        /// The value.
        #[schemars(extend("arrived_in" = "0.1"))]
        value: Value,
    },
    /// The field is text holding this text, compared code point by code
    /// point, case included.
    Contains {
        /// The text.
        #[schemars(extend("arrived_in" = "0.1"))]
        text: String,
    },
}

/// A condition on one top-level field of a Record.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct FieldFilter {
    /// The field's name.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub field: String,
    /// What it must hold.
    #[serde(flatten)]
    pub condition: FieldCondition,
}

/// Which Records a page takes: those that meet every condition.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RecordFilter {
    /// Conditions on the Records' fields, every one met.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub fields: Vec<FieldFilter>,
    /// The Wall marker a Record carries or not; absent takes both.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub wall: Option<WallFilter>,
}

/// A sort's direction.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SortDirection {
    /// Smallest first.
    Ascending,
    /// Largest first.
    Descending,
}

/// The order of a page's Records: by one top-level field, Records that
/// lack it last, then by number.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RecordSort {
    /// The field's name.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub field: String,
    /// Its direction.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub direction: SortDirection,
}

/// What `dataset.page` takes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DatasetPageParams {
    /// The crawl's Run.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub run_id: RunId,
    /// The Dataset's name.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub dataset: String,
    /// The number of the last Record of the previous page, its `next`: the
    /// page starts after that Record in the page's order. Absent, at the
    /// first.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub after: Option<u64>,
    /// The previous page's `through`, so every page reads the Records
    /// committed when the first was read. Absent, the last number
    /// committed now.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub through: Option<u64>,
    /// Which Records to take; absent, every one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub filter: Option<RecordFilter>,
    /// Their order; absent, by number.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub sort: Option<RecordSort>,
    /// The top-level fields each Record's value keeps; absent, every one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub columns: Option<Vec<String>>,
    /// The most Records the page carries, from 1 to 1000; absent, 100.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub limit: Option<u32>,
}

/// One page of `dataset.page`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DatasetPage {
    /// The Records, in the page's order.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub records: Vec<DatasetRecord>,
    /// The number of the last Record committed when the first page was
    /// read: the next page's `through`.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub through: u64,
    /// The next page's `after`; absent once no Record follows.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub next: Option<u64>,
}

/// `dataset.page`.
pub const DATASET_PAGE: Operation = Operation {
    name: "dataset.page",
    description: "Reads one page of a Dataset's committed Records, paged by key: the Records that meet `filter` (conditions on top-level fields, and the `wall` marker a Record's Provenance carries or not), in the order of `sort` (one top-level field, Records that lack it last, then by number), each value keeping only the fields `columns` names, at most `limit`. Filtering, sorting and column selection run in the engine. A page starts after the Record `after` names, the previous page's `next`, and reads only the Records committed through `through`, the number the first page answered, so Records committed meanwhile never make a page repeat or skip one. A Dataset the crawl does not declare fails with `dataset_unknown`; a field name holding `\"`, or a `limit` outside 1 to 1000, with `invalid_params`.",
    class: OperationClass::Read,
    kind: OperationKind::RoundTrip,
    reach: Reach::AllSurfaces,
    params: schema_of::<DatasetPageParams>,
    result: schema_of::<DatasetPage>,
    events: &[],
    cancellable: false,
    input: None,
    bound: None,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// Every entry this module defines.
pub const ENTRIES: Entries = Entries {
    defined_in: module_path!(),
    operations: &[DATASET_LIST, DATASET_PAGE],
    events: &[],
    handler_requests: &[],
    error_codes: &[],
    plugin_kinds: &[],
    metrics: &[],
    settings: &[],
    hook_stages: &[],
    shared_type_values: &[],
};
