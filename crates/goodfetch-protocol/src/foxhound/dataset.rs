//! A crawl's Datasets and the Records its handlers push into them (ADRs
//! 0082 and 0103): each Dataset a crawl declares at its start, with its
//! JSON Schema and its key; `crawl_request.push`, a handler context's
//! `push`, which checks one Record against its Dataset and stages it with
//! the attempt; and the Error codes of a declaration or a Record that does
//! not hold. Foxhound's own.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::catalog::{
    Entries, ErrorCodeDefinition, ErrorGroup, Operation, OperationClass, OperationKind,
    ProtocolVersion, Reach, Stability, schema_of,
};
use crate::run::RunId;

const ARRIVED_IN: ProtocolVersion = ProtocolVersion { major: 0, minor: 1 };

/// One Dataset a crawl declares (ADR 0103): its name, the JSON Schema its
/// Records are checked against, and the top-level fields that identify one
/// Record.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DatasetDeclaration {
    /// Its name, unique in the crawl.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub name: String,
    /// The JSON Schema its Records are checked against, the side the
    /// program's schema parses from; none takes any JSON object (ADR 0082).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub schema: Option<Value>,
    /// The top-level fields that identify one Record, each one the schema
    /// requires; none when no field does.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub key: Vec<String>,
}

/// What `crawl_request.push` takes: one Record an attempt pushes into one
/// of its crawl's Datasets.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CrawlRequestPushParams {
    /// The crawl's Run.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub run_id: RunId,
    /// The ID of the crawl request whose attempt pushes.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub crawl_request_id: String,
    /// The attempt's lease epoch, as `crawl_request.handle` gave it.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub lease_epoch: u64,
    /// The Dataset's name.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub dataset: String,
    /// The Record.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub data: Value,
}

/// What `crawl_request.push` answers once the Record is staged.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CrawlRequestPushed {}

/// `crawl_request.push`.
pub const CRAWL_REQUEST_PUSH: Operation = Operation {
    name: "crawl_request.push",
    description: "Pushes one Record into a Dataset the crawl declares from inside a handler's attempt, a handler context's `push`, and answers once it is staged: it commits once with the attempt's other outputs when the handler returns, and is discarded with them when the attempt ends any other way. The Record is checked first: a Dataset the crawl does not declare fails with `dataset_unknown`; a Record that is no JSON object, fails its Dataset's JSON Schema or lacks one of its key's fields fails with `record_invalid`, naming each issue's path, and is not staged: its rejection is staged instead, entering the Dataset's rejection table (`run.rejected_records`) if the attempt commits, while the attempt keeps what it staged before. An attempt that already ended fails with `attempt_ended`.",
    class: OperationClass::IdempotentWrite,
    kind: OperationKind::RoundTrip,
    reach: Reach::SdkOnly,
    params: schema_of::<CrawlRequestPushParams>,
    result: schema_of::<CrawlRequestPushed>,
    events: &[],
    cancellable: false,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// A crawl that declares two Datasets of one name (ADR 0103).
pub const DATASET_DUPLICATE: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "dataset_duplicate",
    description: "The crawl declares a second Dataset of a name it already declares, so it was not started. The details name the Dataset. Give each Dataset its own name.",
    group: ErrorGroup::Program,
    retryable: false,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// A Record pushed into a Dataset the crawl does not declare (ADR 0103).
pub const DATASET_UNKNOWN: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "dataset_unknown",
    description: "A Record was pushed into a Dataset the crawl does not declare, such as another crawl's, so it was not staged. The details name the Dataset and those the crawl declares.",
    group: ErrorGroup::Program,
    retryable: false,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// A Dataset's key on a field its schema does not require (ADR 0103).
pub const DATASET_KEY_INVALID: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "dataset_key_invalid",
    description: "A Dataset's key names a field that is no top-level field its JSON Schema requires, so a Record could lack it and the crawl was not started. The details name the Dataset and the field. Key a Dataset only by fields its schema requires.",
    group: ErrorGroup::Program,
    retryable: false,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// A Record that does not hold for its Dataset (ADRs 0082 and 0103).
pub const RECORD_INVALID: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "record_invalid",
    description: "A Record is no JSON object, fails its Dataset's JSON Schema, or lacks a field of its Dataset's key, so it was not staged. The details name the Dataset and each issue by its path, a JSON Pointer into the Record. Caught in the handler, the attempt keeps its other outputs; escaping it, the request ends `failed` and is not retried, since the same Record fails the same way.",
    group: ErrorGroup::Program,
    retryable: false,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// What `dataset_duplicate` details.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DatasetDuplicateDetails {
    /// The Dataset's name.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub dataset: String,
}

/// What `dataset_unknown` details.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DatasetUnknownDetails {
    /// The Dataset's name.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub dataset: String,
    /// The names of the Datasets the crawl declares, in order.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub declared: Vec<String>,
}

/// What `dataset_key_invalid` details.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DatasetKeyInvalidDetails {
    /// The Dataset's name.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub dataset: String,
    /// The key's field that its schema does not require.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub field: String,
}

/// One way a Record does not hold for its Dataset.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RecordIssue {
    /// Where in the Record, a JSON Pointer: empty for the Record itself.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub path: String,
    /// What is wrong there.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub message: String,
}

/// What `record_invalid` details.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RecordInvalidDetails {
    /// The Dataset's name.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub dataset: String,
    /// Each way the Record does not hold, at least one.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub issues: Vec<RecordIssue>,
}

/// The module's entries.
pub const ENTRIES: Entries = Entries {
    defined_in: module_path!(),
    operations: &[CRAWL_REQUEST_PUSH],
    events: &[],
    handler_requests: &[],
    error_codes: &[
        DATASET_DUPLICATE,
        DATASET_UNKNOWN,
        DATASET_KEY_INVALID,
        RECORD_INVALID,
    ],
    plugin_kinds: &[],
    metrics: &[],
    settings: &[],
    hook_stages: &[],
    shared_type_values: &[],
};
