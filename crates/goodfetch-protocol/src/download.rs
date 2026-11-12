//! A Download's file, which the engine keeps with its Run until the program
//! saves it (ADRs 0001, 0085 and 0116): `download.save_as` puts the whole
//! file at the program's path or leaves the path as it was. An Attached
//! engine, which runs on the program's machine, writes the path itself
//! through a hidden temporary file beside it, synced, then renamed over it;
//! any other engine writes nothing on its own machine and sends the file in
//! `$/chunk` notifications, one per credit the Client grants, so the SDK
//! writes the path the same way. A Download has no `stream()` and no
//! `path()`: a path on the engine's machine means nothing to the program.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::catalog::{
    Entries, Operation, OperationClass, OperationKind, ProtocolVersion, Reach, Stability, schema_of,
};
use crate::run::RunId;

const ARRIVED_IN: ProtocolVersion = ProtocolVersion { major: 0, minor: 1 };

/// What separates a Download's Run ID from its position in its ID; a
/// position is digits alone, so the last one splits the ID.
const DOWNLOAD_MARK: &str = ".download-";

/// A Download of a Run, named by the engine when it keeps its file:
/// `<run_id>.download-<position>`.
#[derive(
    Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(transparent)]
pub struct DownloadId(pub String);

impl DownloadId {
    /// The ID of the Run `run_id`'s file kept at `position`.
    pub fn of(run_id: &RunId, position: u64) -> Self {
        Self(format!("{run_id}{DOWNLOAD_MARK}{position}"))
    }

    /// Its Run and its position, or `None` for a text no engine names a
    /// Download with.
    pub fn split(&self) -> Option<(RunId, u64)> {
        let (run, position) = self.0.rsplit_once(DOWNLOAD_MARK)?;
        if position.is_empty() || !position.bytes().all(|byte| byte.is_ascii_digit()) {
            return None;
        }
        Some((RunId::parse(run).ok()?, position.parse().ok()?))
    }
}

/// What `download.save_as` takes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DownloadSaveAsParams {
    /// The Download.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub download_id: DownloadId,
    /// The program's path, absolute, which an Attached engine writes
    /// itself; any other engine leaves it to the SDK and sends the file's
    /// bytes instead.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub path: Option<String>,
}

/// What `download.save_as` answers once the file is whole.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DownloadSaved {
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

/// `download.save_as`.
pub const DOWNLOAD_SAVE_AS: Operation = Operation {
    name: "download.save_as",
    description: "Saves a Download's file, which the engine keeps with its Run, whole or not at all. An Attached engine given the program's path writes it through a hidden temporary file beside it, synced and renamed over it, creating missing folders and replacing a file there; any other engine writes nothing on its own machine and sends the file in `$/chunk` notifications of at most 64 KiB, one per credit granted with `$/credit`, for the SDK to write. Answers the file's size and SHA-256. A path that cannot be written raises `path_not_writable`, and full or failing storage `file_write_failed`, leaving the path as it was.",
    class: OperationClass::IdempotentWrite,
    kind: OperationKind::RoundTrip,
    reach: Reach::SdkOnly,
    params: schema_of::<DownloadSaveAsParams>,
    result: schema_of::<DownloadSaved>,
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
    operations: &[DOWNLOAD_SAVE_AS],
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
