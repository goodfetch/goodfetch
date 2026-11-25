//! Erasing a Dataset (ADR 0031; spec #1937, story 214): `dataset.erase`,
//! a person's answer that applies at once with no user code, on a run no
//! engine runs, and that no agent is offered, since a model should make no
//! destructive write (ADR 0077's G5). Foxhound's own.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::catalog::{
    Entries, Operation, OperationClass, OperationKind, ProtocolVersion, Reach, Stability, schema_of,
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
    description: "Erases every Record of one Dataset of a run that no engine runs, a person's answer that applies at once: its committed and rejected Records and their full-text index rows are deleted from the run file, their pages overwritten, and the run's kept Exports, written before, are deleted too, while the crawl still declares the Dataset, which reads empty from then on. It cannot be undone. It is offered to a person only, never to an agent or a program (ADR 0031): any other surface is refused with `person_required`. A run this engine runs fails with `run_live`, one another engine writes with `run_in_use`, a Dataset the crawl does not declare with `dataset_unknown`, and a run that does not exist with `run_not_found`.",
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

/// The module's entries.
pub const ENTRIES: Entries = Entries {
    defined_in: module_path!(),
    operations: &[DATASET_ERASE],
    events: &[],
    handler_requests: &[],
    error_codes: &[],
    plugin_kinds: &[],
    metrics: &[],
    settings: &[],
    hook_stages: &[],
    shared_type_values: &[],
};
