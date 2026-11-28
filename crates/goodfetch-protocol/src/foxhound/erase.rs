//! Erasing a Dataset (`dataset.erase`, ADR 0031; spec #1937, story 214) and
//! erasing matching personal data across a Run's stored data (`run.erase`,
//! ADR 0016; spec #565, story 4-120): person-only answers that apply at once
//! with no user code, on a run no engine runs, and that no agent or program
//! is offered, since a model should make no destructive write (ADR 0077's
//! G5). Foxhound's own.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::catalog::{
    Entries, Operation, OperationClass, OperationKind, ProtocolVersion, Reach, SharedType,
    SharedTypeValue, Stability, schema_of,
};
use crate::run::RunId;

const ARRIVED_IN: ProtocolVersion = ProtocolVersion { major: 0, minor: 1 };

/// What `dataset.erase` takes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DatasetEraseParams {
    /// The crawl's Run, which no engine runs.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub run_id: RunId,
    /// The Dataset's name.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub dataset: String,
}

/// What `dataset.erase` answers once the Dataset is erased.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DatasetErased {
    /// The crawl's Run.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub run_id: RunId,
    /// The Dataset's name.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub dataset: String,
    /// How many committed Records it erased.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub records: u64,
    /// How many rejected Records it erased.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub rejected_records: u64,
}

/// `dataset.erase`.
pub const DATASET_ERASE: Operation = Operation {
    name: "dataset.erase",
    description: "Erases every Record of one Dataset of a run that no engine runs, a person's answer that applies at once: its committed and rejected Records and their full-text index rows are deleted from the run file, their pages overwritten, and the run's kept Exports, written before, are deleted too, while the crawl still declares the Dataset, which reads empty from then on. Each erase writes an Audit log entry. It cannot be undone. It is offered to a person only, never to an agent or a program (ADR 0031): any other surface is refused with `person_required`. A run this engine runs fails with `run_live`, one another engine writes with `run_in_use`, a Dataset the crawl does not declare with `dataset_unknown`, and a run that does not exist with `run_not_found`.",
    class: OperationClass::IdempotentWrite,
    kind: OperationKind::RoundTrip,
    reach: Reach::PersonOnly,
    params: schema_of::<DatasetEraseParams>,
    result: schema_of::<DatasetErased>,
    events: &[],
    cancellable: false,
    input: None,
    bound: None,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// What `run.erase` takes: a Run no engine runs, and a value or key (or both)
/// whose matching data is erased across the Run's stored Records, Baseline
/// rows, Checkpoint fields and Response cache entries (ADR 0016).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RunEraseParams {
    /// The crawl's Run, which no engine runs.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub run_id: RunId,
    /// A value to erase wherever it appears across the Run's stored data.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub value: Option<String>,
    /// A field name or dotted path (or key value) to erase across the Run's
    /// stored data.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub key: Option<String>,
}

/// What `run.erase` answers once the matching data is erased across the Run.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RunErased {
    /// The crawl's Run.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub run_id: RunId,
    /// How many committed Records it erased.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub records: u64,
    /// How many rejected Records it erased.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub rejected_records: u64,
    /// How many Baseline rows it erased.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub baseline_rows: u64,
    /// How many Checkpoint entries or fields it erased.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub checkpoint_entries: u64,
    /// How many Response cache entries it erased.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub response_cache_entries: u64,
}

/// `run.erase` (ADR 0016; spec #565, story 4-120).
pub const RUN_ERASE: Operation = Operation {
    name: "run.erase",
    description: "Erases every matching Record, Baseline row, Checkpoint field and Response cache entry of one run that no engine runs, matching by `value`, `key` or both (at least one must be non-empty): its matching committed and rejected Records and full-text index rows are deleted with secure deletion, matching Checkpoint fields and kept Exports are scrubbed or deleted, matching Response cache entries and unheld bodies are purged, and an Audit log entry is written without repeating the erased personal value. It cannot be undone. It is offered to a person only, in an interactive CLI prompt, the Kennel or a person-attended Client, never under `--json`, a Recipe, an agent or a program (ADR 0016): any other surface is refused with `person_required`. A run this engine runs fails with `run_live`, one another engine writes with `run_in_use`, and a run that does not exist with `run_not_found`.",
    class: OperationClass::IdempotentWrite,
    kind: OperationKind::RoundTrip,
    reach: Reach::PersonOnly,
    params: schema_of::<RunEraseParams>,
    result: schema_of::<RunErased>,
    events: &[],
    cancellable: false,
    input: None,
    bound: None,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// A person erased data from a Run (`dataset.erase` or `run.erase`, ADR 0016).
pub const RUN_DATA_ERASED: SharedTypeValue = SharedTypeValue {
    shared_type: SharedType::AuditKind,
    name: "run_data_erased",
    description: "A person erased data from a Run's stored Records, Baseline rows, Checkpoint or Response cache entries (`dataset.erase` or `run.erase`, ADR 0016), recording the operation and counts without repeating any erased personal value.",
    arrived_in: ARRIVED_IN,
};

/// The module's entries.
pub const ENTRIES: Entries = Entries {
    defined_in: module_path!(),
    operations: &[DATASET_ERASE, RUN_ERASE],
    events: &[],
    handler_requests: &[],
    error_codes: &[],
    plugin_kinds: &[],
    metrics: &[],
    settings: &[],
    hook_stages: &[],
    shared_type_values: &[RUN_DATA_ERASED],
};
