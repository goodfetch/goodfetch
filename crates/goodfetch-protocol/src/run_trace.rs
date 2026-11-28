//! A Run's trace copy (ADRs 0102 and 0116): `run.trace`, which answers the
//! Run's trace copy once its file is complete (or none when the Run keeps no
//! trace); `trace.save_as`, which writes it whole or not at all on the
//! program's machine; and `trace.stream`, which sends its bytes in order.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::catalog::{
    Entries, Operation, OperationClass, OperationKind, ProtocolVersion, Reach, Stability, schema_of,
};
use crate::run::{RunId, RunParams};

const ARRIVED_IN: ProtocolVersion = ProtocolVersion { major: 0, minor: 1 };

/// A Run's trace copy, kept on the engine's machine (ADRs 0102 and 0116).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RunTrace {
    /// The Run's ID.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub run_id: RunId,
    /// Where it is: the trace file's path on the engine's machine.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub location: String,
    /// Its size in bytes.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub size: u64,
    /// The SHA-256 of its bytes, in lowercase hexadecimal.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub digest: String,
}

/// What `run.trace` answers: the Run's trace copy, or none when the Run keeps
/// no trace.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RunTraceResult {
    /// The Run's trace copy, or null when the Run keeps no trace.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub trace: Option<RunTrace>,
}

/// `run.trace`.
pub const RUN_TRACE: Operation = Operation {
    name: "run.trace",
    description: "Reads a Run's trace copy as recorded so far, or null when the Run keeps no trace (ADRs 0102 and 0116). Returns only once the copy is whole, with its size and SHA-256 digest; a write the engine cannot make fails with `file_write_failed`, side `engine`, and leaves no unfinished file. Takes no writer lock, so a view reads a Run no program holds without resuming it. A Run ID the Run index does not list fails with `run_not_found`.",
    class: OperationClass::Read,
    kind: OperationKind::RoundTrip,
    reach: Reach::SdkOnly,
    params: schema_of::<RunParams>,
    result: schema_of::<RunTraceResult>,
    events: &[],
    cancellable: true,
    input: None,
    bound: None,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// What `trace.save_as` takes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TraceSaveAsParams {
    /// The Run's ID.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub run_id: RunId,
    /// Where the trace copy is on the engine's machine, as `run.trace` answered.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub location: String,
    /// The program's path, absolute, which an Attached engine writes
    /// itself; any other engine leaves it to the SDK and sends the file's
    /// bytes instead.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub path: Option<String>,
}

/// What `trace.save_as` answers once the file is whole.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TraceSaved {
    /// The file's size in bytes.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub size: u64,
    /// The SHA-256 of its bytes, in lowercase hexadecimal.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub digest: String,
    /// Whether the engine wrote the program's path itself; when not, the
    /// `$/chunk` notifications before this answer carried every byte, and
    /// the SDK writes the path.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub written: bool,
}

/// `trace.save_as`.
pub const TRACE_SAVE_AS: Operation = Operation {
    name: "trace.save_as",
    description: "Saves a Run trace's file, which the engine keeps on its machine, whole or not at all (ADRs 0102 and 0116). An Attached engine given the program's path writes it through a hidden temporary file beside it, synced and renamed over it, creating missing folders and replacing a file there; any other engine writes nothing on its own machine and sends the file in `$/chunk` notifications of at most 64 KiB, one per credit granted with `$/credit`, for the SDK to write. Answers the file's size and its SHA-256 digest. A path that cannot be written raises `path_not_writable`, and full or failing storage `file_write_failed`, leaving the path as it was.",
    class: OperationClass::IdempotentWrite,
    kind: OperationKind::RoundTrip,
    reach: Reach::SdkOnly,
    params: schema_of::<TraceSaveAsParams>,
    result: schema_of::<TraceSaved>,
    events: &[],
    cancellable: true,
    input: None,
    bound: None,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// What `trace.stream` takes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TraceStreamParams {
    /// The Run's ID.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub run_id: RunId,
    /// Where the trace copy is on the engine's machine, as `run.trace` answered.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub location: String,
}

/// What `trace.stream` answers once it sent every byte.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TraceStreamed {
    /// The file's size in bytes.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub size: u64,
    /// The SHA-256 of its bytes, in lowercase hexadecimal.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub digest: String,
}

/// `trace.stream`.
pub const TRACE_STREAM: Operation = Operation {
    name: "trace.stream",
    description: "Sends a Run trace's bytes in order, in `$/chunk` notifications of at most 64 KiB, one per credit granted with `$/credit`, and answers its size and its SHA-256 digest once every byte is sent (ADRs 0102 and 0116). Nothing is written on either machine: a program that writes the bytes itself meets its own language's file errors.",
    class: OperationClass::Read,
    kind: OperationKind::Sequence,
    reach: Reach::SdkOnly,
    params: schema_of::<TraceStreamParams>,
    result: schema_of::<TraceStreamed>,
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
    operations: &[RUN_TRACE, TRACE_SAVE_AS, TRACE_STREAM],
    events: &[],
    handler_requests: &[],
    error_codes: &[],
    plugin_kinds: &[],
    metrics: &[],
    settings: &[],
    hook_stages: &[],
    shared_type_values: &[],
};
