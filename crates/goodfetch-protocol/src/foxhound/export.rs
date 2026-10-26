//! A Dataset's Export (ADRs 0026, 0103, 0116 and 0119): `run.export`
//! writes one file of the Records a Dataset committed when the call is
//! made, in number order, into the Run's folder, and answers an `Export`
//! only once that file is whole, so an `Export` a program holds is always
//! whole. A failed write raises `file_write_failed`, side `engine`, and
//! leaves no unfinished file; at the `critical` storage level it fails
//! before writing. `export.save_as` puts the file at the program's path, as
//! `download.save_as` puts a Download's, and `export.stream` sends its
//! bytes. Every text Export is UTF-8: JSON and JSONL carry no BOM, write
//! characters as themselves and write each Record's value as it was
//! committed. Foxhound's own; the two Error codes of a failed write are
//! Collie's (ADR 0095).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::catalog::{
    Entries, Operation, OperationClass, OperationKind, ProtocolVersion, Reach, Stability, schema_of,
};
use crate::run::RunId;

const ARRIVED_IN: ProtocolVersion = ProtocolVersion { major: 0, minor: 1 };

/// What separates an Export's Run ID from its file's SHA-256 in its ID; a
/// SHA-256 is hexadecimal alone, so the last one splits the ID.
const EXPORT_MARK: &str = ".export-";

/// An Export of a Run, named by the engine when its file is whole:
/// `<run_id>.export-<sha256>`, the SHA-256 of its bytes in lowercase
/// hexadecimal, so two Exports of the same bytes are one file.
#[derive(
    Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(transparent)]
pub struct ExportId(pub String);

impl ExportId {
    /// The ID of the Run `run_id`'s Export whose bytes' SHA-256 is
    /// `sha256`.
    pub fn of(run_id: &RunId, sha256: &str) -> Self {
        Self(format!("{run_id}{EXPORT_MARK}{sha256}"))
    }

    /// Its Run and its file's SHA-256, or `None` for a text no engine names
    /// an Export with.
    pub fn split(&self) -> Option<(RunId, &str)> {
        let (run, sha256) = self.0.rsplit_once(EXPORT_MARK)?;
        let digest = sha256.len() == 64
            && sha256
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte));
        if !digest {
            return None;
        }
        Some((RunId::parse(run).ok()?, sha256))
    }
}

/// An Export's format, named, never inferred from a file name (ADR 0103).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ExportFormat {
    /// One JSON array of the Records' values.
    Json,
    /// One Record's value per line, each line ended by a line feed.
    Jsonl,
}

/// How an Export's bytes are compressed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ExportCompression {
    /// Not compressed.
    #[default]
    None,
}

/// What `run.export` takes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RunExportParams {
    /// The crawl's Run.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub run_id: RunId,
    /// The Dataset's name.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub dataset: String,
    /// The format.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub format: ExportFormat,
    /// The compression, none when absent.
    #[serde(default)]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub compression: ExportCompression,
}

/// What `run.export` answers once the Export's file is whole (ADR 0103).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Export {
    /// The Export, which `export.save_as` and `export.stream` take.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub export_id: ExportId,
    /// Its format.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub format: ExportFormat,
    /// Its compression.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub compression: ExportCompression,
    /// How many Records it holds.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub records: u64,
    /// The number of the last Record it holds, 0 when it holds none.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub last_number: u64,
    /// Its size in bytes.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub size: u64,
    /// The SHA-256 of its bytes, in lowercase hexadecimal.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub sha256: String,
}

/// `run.export`.
pub const RUN_EXPORT: Operation = Operation {
    name: "run.export",
    description: "Writes an Export of one Dataset of a crawl's Run: the Records it committed when the call is made, in number order, in the format named (`json`, one array of the Records' values; `jsonl`, one value per line), UTF-8 with no BOM, characters written as themselves and each value as it was committed. Answers the Export, with its Record count, its last Record's number, its size and its SHA-256, only once its file is whole in the Run's folder, where `export.save_as` and `export.stream` read it. Records committed after the call starts are not in it. A write the engine's storage refuses fails with `file_write_failed`, side `engine`, leaving no unfinished file, as does a call while that storage is at the `critical` level, before any byte is written. A Dataset the crawl does not declare fails with `dataset_unknown`.",
    class: OperationClass::IdempotentWrite,
    kind: OperationKind::RoundTrip,
    reach: Reach::AllSurfaces,
    params: schema_of::<RunExportParams>,
    result: schema_of::<Export>,
    events: &[],
    cancellable: true,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// What `export.save_as` takes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExportSaveAsParams {
    /// The Export.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub export_id: ExportId,
    /// The program's path, absolute, which an Attached engine writes
    /// itself; any other engine leaves it to the SDK and sends the file's
    /// bytes instead.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub path: Option<String>,
}

/// What `export.save_as` answers once the file is whole.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ExportSaved {
    /// The file's size in bytes.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub size: u64,
    /// The SHA-256 of its bytes, in lowercase hexadecimal.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub sha256: String,
    /// Whether the engine wrote the program's path itself; when not, the
    /// `$/chunk` notifications before this answer carried every byte, and
    /// the SDK writes the path.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub written: bool,
}

/// `export.save_as`.
pub const EXPORT_SAVE_AS: Operation = Operation {
    name: "export.save_as",
    description: "Saves an Export's file, which the engine keeps in its Run's folder, whole or not at all. An Attached engine given the program's path writes it through a hidden temporary file beside it, synced and renamed over it, creating missing folders and replacing a file there; any other engine writes nothing on its own machine and sends the file in `$/chunk` notifications of at most 64 KiB, one per credit granted with `$/credit`, for the SDK to write. Answers the file's size and SHA-256. A path that cannot be written raises `path_not_writable`, and full or failing storage `file_write_failed`, leaving the path as it was.",
    class: OperationClass::IdempotentWrite,
    kind: OperationKind::RoundTrip,
    reach: Reach::SdkOnly,
    params: schema_of::<ExportSaveAsParams>,
    result: schema_of::<ExportSaved>,
    events: &[],
    cancellable: true,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// What `export.stream` takes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExportStreamParams {
    /// The Export.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub export_id: ExportId,
}

/// What `export.stream` answers once it sent every byte.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ExportStreamed {
    /// The file's size in bytes.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub size: u64,
    /// The SHA-256 of its bytes, in lowercase hexadecimal.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub sha256: String,
}

/// `export.stream`.
pub const EXPORT_STREAM: Operation = Operation {
    name: "export.stream",
    description: "Sends an Export's bytes, which the engine keeps in its Run's folder, in order, in `$/chunk` notifications of at most 64 KiB, one per credit granted with `$/credit`, and answers its size and SHA-256 once every byte is sent. Nothing is written on either machine: a program that writes the bytes itself meets its own language's file errors.",
    class: OperationClass::Read,
    kind: OperationKind::Sequence,
    reach: Reach::SdkOnly,
    params: schema_of::<ExportStreamParams>,
    result: schema_of::<ExportStreamed>,
    events: &[],
    cancellable: true,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// An Export of the `json` format, its Compatibility corpus's document
/// (ADR 0066): an array of the Records' values, each as committed.
pub type JsonExport = Vec<Value>;

/// Every entry this module defines.
pub const ENTRIES: Entries = Entries {
    defined_in: module_path!(),
    operations: &[RUN_EXPORT, EXPORT_SAVE_AS, EXPORT_STREAM],
    events: &[],
    handler_requests: &[],
    error_codes: &[],
    plugin_kinds: &[],
    metrics: &[],
    settings: &[],
    hook_stages: &[],
    shared_type_values: &[],
};

#[cfg(test)]
mod tests;
