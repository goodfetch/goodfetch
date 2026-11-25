//! Full-text search of a crawl's Records (ADR 0031; spec #593, stories 208
//! and 209): `dataset.search`, the Records of one Dataset of a run that
//! hold every word of a text, paged by number; `record.search`, the same
//! search across the runs a person selects, at most 100, to find which run
//! returned a Record without opening each; and `dataset.search.index`,
//! whether a crawl indexes its Records for them as it commits them.
//! Foxhound's own; catalog reads every surface offers.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::catalog::{
    Authority, Entries, Operation, OperationClass, OperationKind, ProtocolVersion, Reach, Setting,
    Stability, ValueKind, schema_of,
};
use crate::foxhound::dataset::DatasetRecord;
use crate::run::RunId;

const ARRIVED_IN: ProtocolVersion = ProtocolVersion { major: 0, minor: 1 };

/// The most Records one search answers.
pub const SEARCH_MOST: u32 = 1000;

/// The Records one search answers when its `limit` is absent.
pub const SEARCH_DEFAULT: u32 = 100;

/// What `dataset.search` takes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DatasetSearchParams {
    /// The crawl's Run.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub run_id: RunId,
    /// The Dataset's name.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub dataset: String,
    /// The words a Record holds, each anywhere in its values, in any case
    /// and with or without diacritics.
    #[schemars(length(min = 1))]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub text: String,
    /// The number of the last Record of the previous page, its `next`: the
    /// page starts after it. Absent, at the first.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub after: Option<u64>,
    /// The most Records the page carries, from 1 to 1000; absent, 100.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 1, max = 1000))]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub limit: Option<u32>,
}

/// One page of `dataset.search`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DatasetSearchPage {
    /// The Records that hold every word, in number order.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub records: Vec<DatasetRecord>,
    /// The next page's `after`; absent once no Record follows.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub next: Option<u64>,
}

/// What `record.search` takes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RecordSearchParams {
    /// The words a Record holds, each anywhere in its values, in any case
    /// and with or without diacritics.
    #[schemars(length(min = 1))]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub text: String,
    /// The runs it searches, from 1 to 100.
    #[schemars(length(min = 1, max = 100))]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub runs: Vec<RunId>,
    /// The Datasets it searches in each run; absent, every one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub datasets: Option<Vec<String>>,
    /// The most Records it answers, from 1 to 1000; absent, 100.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 1, max = 1000))]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub limit: Option<u32>,
}

/// One Record `record.search` found.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct FoundRecord {
    /// The run that returned it.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub run_id: RunId,
    /// Its Dataset.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub dataset: String,
    /// Its number in its Dataset.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub number: u64,
    /// The Record, as JSON.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub value: Value,
}

/// What `record.search` answers.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RecordSearchResult {
    /// The Records that hold every word, the best matches first, then by
    /// run, Dataset and number.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub records: Vec<FoundRecord>,
}

/// `dataset.search`.
pub const DATASET_SEARCH: Operation = Operation {
    name: "dataset.search",
    description: "Answers one page of the committed Records of one Dataset of a run that hold every word of `text`, each anywhere in the Record's values, in any case and with or without diacritics, in number order, after `after` and at most `limit` (1 to 1000, 100 by default), with the next page's `after` while more follow. It reads the index a crawl writes as it commits each Record while `dataset.search.index` is on, so a crawl with it off finds none. An empty `text` fails with `invalid_params`, and a Dataset the crawl does not declare with `dataset_unknown`.",
    class: OperationClass::Read,
    kind: OperationKind::RoundTrip,
    reach: Reach::AllSurfaces,
    params: schema_of::<DatasetSearchParams>,
    result: schema_of::<DatasetSearchPage>,
    events: &[],
    cancellable: true,
    input: None,
    bound: None,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `record.search`.
pub const RECORD_SEARCH: Operation = Operation {
    name: "record.search",
    description: "Searches the committed Records of the runs `runs` selects, at most 100, for those that hold every word of `text`, each anywhere in the Record's values, in any case and with or without diacritics, in every Dataset or those `datasets` names, and answers at most `limit` (1 to 1000, 100 by default), the best matches first, each with its run, Dataset and number, so the runs that returned one can be read or queried with `dataset.query`. It reads only the selected runs' indexes, which a crawl writes while `dataset.search.index` is on, and runs under `timeout.dataset_query` and `dataset.query.memory_limit`. An empty `text` fails with `invalid_params`, and a run that does not exist with `run_not_found`.",
    class: OperationClass::Read,
    kind: OperationKind::RoundTrip,
    reach: Reach::AllSurfaces,
    params: schema_of::<RecordSearchParams>,
    result: schema_of::<RecordSearchResult>,
    events: &[],
    cancellable: true,
    input: None,
    bound: None,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

fn yes() -> Value {
    json!(true)
}

/// `dataset.search.index` (ADR 0138).
pub const DATASET_SEARCH_INDEX: Setting = Setting {
    name: "dataset.search.index",
    description: "Whether the crawl indexes each Record it commits for `dataset.search` and `record.search`, in the commit's transaction. On by default. Off, no Record is indexed, so neither search finds one of the crawl's Records, while each is still committed and read as before.",
    kind: ValueKind::Boolean,
    default: yes,
    clearable: false,
    authority: Authority::Every,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// The module's entries.
pub const ENTRIES: Entries = Entries {
    defined_in: module_path!(),
    operations: &[DATASET_SEARCH, RECORD_SEARCH],
    events: &[],
    handler_requests: &[],
    error_codes: &[],
    plugin_kinds: &[],
    metrics: &[],
    settings: &[DATASET_SEARCH_INDEX],
    hook_stages: &[],
    shared_type_values: &[],
};
