//! The refusal of a run folder on a filesystem whose locking SQLite cannot
//! rely on (ADR 0049): a network filesystem, or a FUSE mount of object
//! storage. The engine keeps no run state there, so the Run does not start.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::catalog::{ErrorCodeDefinition, ErrorGroup, ProtocolVersion};

use super::RunId;

const ARRIVED_IN: ProtocolVersion = ProtocolVersion { major: 0, minor: 1 };

/// A Run whose folder is on a filesystem the engine keeps no run state on
/// (ADR 0049).
pub const RUN_STATE_FILESYSTEM_UNSUPPORTED: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "run_state_filesystem_unsupported",
    description: "The Run's folder is on a network filesystem (NFS, which EFS and Filestore are, or SMB, which Azure Files is) or on a FUSE mount of object storage, whose locking SQLite cannot rely on, so the engine keeps no run state there. The details name the Run, its folder, the `filesystem` and the `fix`: a `run_state.location` on a local disk or a block volume.",
    group: ErrorGroup::Unavailable,
    retryable: false,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// The kind of filesystem a run folder was refused on (ADR 0049).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RunStateFilesystem {
    /// NFS, which EFS, Filestore and Azure Files' NFS shares are.
    Nfs,
    /// SMB or CIFS, which Azure Files' SMB shares are.
    Smb,
    /// A FUSE mount of an object store, such as `s3fs`, Mountpoint for
    /// Amazon S3, `gcsfuse` or `BlobFuse`; the message names its tool.
    FuseObjectStorage,
}

/// What `run_state_filesystem_unsupported` details.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RunStateFilesystemUnsupportedDetails {
    /// The Run ID.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub run_id: RunId,
    /// The Run's folder.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub path: String,
    /// The filesystem the folder is on.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub filesystem: RunStateFilesystem,
    /// How to keep the Run's state where the engine can.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub fix: String,
}
