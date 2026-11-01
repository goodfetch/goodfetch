//! A crawl's Datasets and the Records its handlers push into them (ADRs
//! 0082 and 0103): each Dataset a crawl declares at its start, with its
//! JSON Schema and its key; `crawl_request.push`, a handler context's
//! `push`, which checks one Record against its Dataset and stages it with
//! the attempt; `run.records`, the cursor that reads a Dataset's committed
//! Records from the first, or after a number, following the Run live under
//! its Client's credits, so a reader never slows the crawl; and the Error
//! codes of a declaration or a Record that does not hold. Foxhound's own.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::catalog::{
    Entries, ErrorCodeDefinition, ErrorGroup, Operation, OperationClass, OperationKind,
    ProtocolVersion, Reach, Stability, schema_of,
};
use crate::foxhound::provenance::RecordProvenance;
use crate::jsonrpc::RequestId;
use crate::run::{PointInTime, RunId};

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

/// The notification that carries one Record `run.records` sends.
pub const RECORD: &str = "$/record";

/// What `run.records` takes: a crawl's Run, one of its Datasets, and where
/// to start.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RunRecordsParams {
    /// The crawl's Run.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub run_id: RunId,
    /// The Dataset's name.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub dataset: String,
    /// The number of the last Record the reader handled: the call starts
    /// after it. Absent, it starts at the Dataset's first Record.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub after: Option<u64>,
    /// The number of the last Record to read, such as the last one
    /// committed when the reader began (`dataset.page`'s `through`): the
    /// call answers once it sent every Record numbered through it, waiting
    /// on a live Run for those not committed yet. Absent, it follows a live
    /// Run until the Run ends.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub through: Option<u64>,
    /// `true` reads only the Records whose page was read past a Wall,
    /// `false` only the others; absent, every Record (ADR 0052).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub wall: Option<bool>,
}

/// One committed Record of a Dataset, as a reader reads it (ADR 0103).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DatasetRecord {
    /// Its number in its Dataset, from 1, in commit order, with no gaps.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub number: u64,
    /// The Record, as JSON; the program's schema parses it.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub value: Value,
    /// The ID of the crawl request whose attempt committed it.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub request_id: String,
    /// That crawl request's URL.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub url: String,
    /// When it was committed.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub committed_at: PointInTime,
    /// How it was obtained (ADR 0123's Provenance of a Record).
    #[schemars(extend("arrived_in" = "0.1"))]
    pub provenance: RecordProvenance,
}

/// What `$/record` carries: the next Record a `run.records` call reads.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RecordRead {
    /// The id of the `run.records` call.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub id: RequestId,
    /// The Record.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub record: DatasetRecord,
}

/// What `run.records` answers once it sent the last Record: how many it
/// sent, and the number of the last, which a later call's `after` takes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RecordsRead {
    /// How many Records it sent.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub count: u64,
    /// The number of the last Record it sent, else the `after` it took, 0
    /// without one.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub last: u64,
}

/// `run.records`.
pub const RUN_RECORDS: Operation = Operation {
    name: "run.records",
    description: "Reads the committed Records of one Dataset of a crawl's Run in number order, from the first, or after the number `after`, which resumes a reader from the last Record it handled. Each Record goes in a `$/record` notification with its number, its value, its request's ID and URL and when it was committed, one per credit granted with `$/credit`. While the Run is live it follows the Run, sending each Record once it commits; once the Run has ended, or has no live writer, and every Record is sent, it answers how many it sent and the last one's number. With `through`, such as the number `dataset.page` answers as the last committed, it reads no Record numbered past it and answers once it sent every Record through it, a live Run's included. Records are read from the Run's committed state on a connection of their own, so a reader that grants no credit never slows the crawl. With `wall` set to `true` it reads only the Records whose page was read past a Wall, which their Provenance marks, and with `false` only the others (ADR 0052). A Dataset the crawl does not declare fails with `dataset_unknown`.",
    class: OperationClass::Read,
    kind: OperationKind::Sequence,
    reach: Reach::AllSurfaces,
    params: schema_of::<RunRecordsParams>,
    result: schema_of::<RecordsRead>,
    events: &[],
    cancellable: true,
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
    description: "A Record was pushed into a Dataset the crawl does not declare, such as another crawl's, so it was not staged; `run.records` read such a Dataset, so nothing was read; or a start under the ID of a Run whose Dataset holds Records no longer declares it, a renamed one included, so the Run did no work and no Record is orphaned. The details name the Dataset and those the crawl declares. Declare the Dataset again, or start a new Run.",
    group: ErrorGroup::Program,
    retryable: false,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// A Dataset's key on a field its schema does not require (ADR 0103).
pub const DATASET_KEY_INVALID: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "dataset_key_invalid",
    description: "A Dataset's key names a field that is no top-level field its JSON Schema requires, so a Record could lack it and the crawl was not started (`reason` `not_required`, naming the field); or a start under the ID of a Run changes the key of a Dataset that holds Records, so the Run did no work and a Sink's upsert never mixes two identities (`reason` `changed`, naming both keys). The details name the Dataset. Key a Dataset only by fields its schema requires, and keep its key for the Run's life or start a new Run.",
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
    /// Why its key is refused.
    #[serde(flatten)]
    pub refused: KeyRefused,
}

/// Why a Dataset's key is refused, its `reason`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "reason", rename_all = "snake_case")]
pub enum KeyRefused {
    /// The key names a field that is no top-level field its schema
    /// requires, as the Dataset is declared.
    NotRequired {
        /// That field.
        #[schemars(extend("arrived_in" = "0.1"))]
        field: String,
    },
    /// At a resume, the key of a Dataset that holds Records differs from
    /// the one the Run began with.
    Changed {
        /// The key the crawl declares now.
        #[schemars(extend("arrived_in" = "0.1"))]
        key: Vec<String>,
        /// The key the Run began with.
        #[schemars(extend("arrived_in" = "0.1"))]
        began_with: Vec<String>,
    },
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
    operations: &[CRAWL_REQUEST_PUSH, RUN_RECORDS],
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
