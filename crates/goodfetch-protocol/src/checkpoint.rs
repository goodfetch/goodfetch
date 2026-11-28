//! A Run's Checkpoints and its resume, which every Product carries (ADRs
//! 0002, 0003, 0047, 0056, 0066, 0102 and 0116): `run.checkpoint`, which
//! writes a Checkpoint and answers it once its file is complete;
//! `run.pause` and `run.migrate`, which checkpoint a Run and stop it,
//! resumable; `run.resume`, which takes the next run epoch of an
//! interrupted Run, from its run file or a Checkpoint; the lifecycle events
//! a Run's stream carries (RT-14); what binds a Run to its machine; the
//! periodic interval; and the Error codes they answer with.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::catalog::{
    Authority, Entries, ErrorCodeDefinition, ErrorGroup, Event, EventClass, Operation,
    OperationClass, OperationKind, ProtocolVersion, Reach, Setting, Stability, ValueKind,
    schema_of,
};
use crate::connection::StoppedBy;
use crate::run::{Checkpoint, RunId, RunParams, RunReport, RunStarted, StopReason};
use crate::settings::ProjectSettings;

const ARRIVED_IN: ProtocolVersion = ProtocolVersion { major: 0, minor: 1 };

/// What `run.checkpoint` takes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RunCheckpointParams {
    /// The Run's ID.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub run_id: RunId,
    /// Where to write the Checkpoint, a path on the engine's machine or an
    /// object-storage URL (`s3://`, `gs://`, `az://`); by default where the
    /// engine keeps the Run's Checkpoints, its own folder. The catalog's
    /// caller alone gives it, such as Goodfetch Cloud: an SDK's
    /// `checkpoint()` takes no path, which would break on a Daemon or
    /// Goodfetch Cloud (ADR 0102).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1", "catalog_only" = true))]
    pub location: Option<String>,
}

/// What `run.migrate` takes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RunMigrateParams {
    /// The Run's ID.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub run_id: RunId,
    /// Where to write the Checkpoint the Run is resumed from, a path on the
    /// engine's machine or an object-storage URL (`s3://`, `gs://`, `az://`);
    /// by default where the engine keeps the Run's Checkpoints, its own folder.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub location: Option<String>,
}

/// What `run.resume` takes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RunResumeParams {
    /// The Run's ID.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub run_id: RunId,
    /// A Checkpoint to resume from, a path on the engine's machine or an
    /// object-storage URL (`s3://`, `gs://`, `az://`), which becomes the
    /// Run's state in this engine's run folder; by default the Run resumes
    /// from its run file on this machine.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub checkpoint: Option<String>,
    /// Whether a `checkpoint` behind the Run's run file on this machine, or
    /// given where that file cannot be read, replaces it all the same,
    /// rolling the Run back to the Checkpoint, as the Audit log records; by
    /// default such a Checkpoint is refused, with `checkpoint_behind` or the
    /// error the file's read gave. Without `checkpoint` it changes nothing.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub rollback: bool,
    /// The Run's settings from the resume on, nested by key: the run layer
    /// (ADRs 0083 and 0102).
    #[serde(default, skip_serializing_if = "serde_json::Map::is_empty")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub settings: serde_json::Map<String, serde_json::Value>,
    /// The project configuration the Client found, layer 3 (ADR 0083).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub project: Option<ProjectSettings>,
    /// Optional Group member pin in a Daemon group (ADR 0055).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub group_member: Option<String>,
}

/// What a Run is bound to (ADR 0056), which a resume elsewhere checks.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RunBinding {
    /// The machine its Session source is on: it resumes only there.
    Machine {
        /// The machine's identity, its Goodfetch home's.
        #[schemars(extend("arrived_in" = "0.1"))]
        machine: String,
    },
    /// A person's Browser host, which serves its Session source to any
    /// engine: it resumes anywhere.
    BrowserHost {
        /// The Browser host's identity.
        #[schemars(extend("arrived_in" = "0.1"))]
        browser_host: String,
    },
}

/// `run.checkpoint`.
pub const RUN_CHECKPOINT: Operation = Operation {
    name: "run.checkpoint",
    description: "Writes a Checkpoint of a live Run now, through SQLite's online backup, where the engine keeps the Run's Checkpoints, or where `location` says, a path on the engine's machine that only the catalog's caller gives, an SDK's `checkpoint()` taking none, and answers it once its file is complete, with its size, digest, run epoch and journal position. A Checkpoint the engine cannot write fails with `file_write_failed` and leaves no file.",
    class: OperationClass::Effect,
    kind: OperationKind::RoundTrip,
    reach: Reach::AllSurfaces,
    params: schema_of::<RunCheckpointParams>,
    result: schema_of::<Checkpoint>,
    events: &[&RUN_CHECKPOINTING],
    cancellable: false,
    input: None,
    bound: None,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `run.pause` (RT-11).
pub const RUN_PAUSE: Operation = Operation {
    name: "run.pause",
    description: "Pauses a live Run: checkpoints it, then stops it at its next safe point, `stopped` with the stop reason `pause` and resumable by its Run ID with `run.resume`, after a restart too. Answers its Run report, which names the Checkpoint.",
    class: OperationClass::IdempotentWrite,
    kind: OperationKind::RoundTrip,
    reach: Reach::AllSurfaces,
    params: schema_of::<RunParams>,
    result: schema_of::<RunReport>,
    events: &[&RUN_CHECKPOINTING, &RUN_STOPPING],
    cancellable: false,
    input: None,
    bound: None,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `run.migrate`.
pub const RUN_MIGRATE: Operation = Operation {
    name: "run.migrate",
    description: "Migrates a live Run: checkpoints it where `location` says or where the engine keeps its Checkpoints, then stops it at its next safe point, `stopped` with the stop reason `migrate`, to be resumed from that Checkpoint on another machine. Answers its Run report, which names the Checkpoint.",
    class: OperationClass::IdempotentWrite,
    kind: OperationKind::RoundTrip,
    reach: Reach::AllSurfaces,
    params: schema_of::<RunMigrateParams>,
    result: schema_of::<RunReport>,
    events: &[&RUN_MIGRATING, &RUN_CHECKPOINTING, &RUN_STOPPING],
    cancellable: false,
    input: None,
    bound: None,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `run.resume`.
pub const RUN_RESUME: Operation = Operation {
    name: "run.resume",
    description: "Resumes an interrupted Run from its committed state, its run file on this machine or the Checkpoint `checkpoint` names: it takes the Run's writer lock and the next run epoch, so every write of an earlier writer is refused, and redoes the work since that state; its recorded Clean-up rules digest is kept. A Run this engine runs already fails with `run_in_use`, as one another engine writes does; a finished one is answered as it ended, `ended`. A Run bound to another machine fails with `session_bound_to_machine`, a Checkpoint of a format version this engine cannot read with `format_version_unsupported`, and a Checkpoint behind the Run's run file on this machine with `checkpoint_behind`, or with the error that file's read gives when it cannot be read, unless `rollback` replaces that file, as the Audit log records.",
    class: OperationClass::IdempotentWrite,
    kind: OperationKind::Opener,
    reach: Reach::AllSurfaces,
    params: schema_of::<RunResumeParams>,
    result: schema_of::<RunStarted>,
    events: &[],
    cancellable: false,
    input: None,
    bound: None,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// What `run.checkpointing` carries.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RunCheckpointing {
    /// The Run's ID.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub run_id: RunId,
    /// The run epoch the Checkpoint is taken under.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub run_epoch: u64,
}

/// What `run.migrating` carries.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RunMigrating {
    /// The Run's ID.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub run_id: RunId,
}

/// What `run.aborting` carries.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RunAborting {
    /// The Run's ID.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub run_id: RunId,
    /// Who aborts it.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub by: StoppedBy,
}

/// What `run.stopping` carries.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RunStopping {
    /// The Run's ID.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub run_id: RunId,
    /// Why it stops.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub cause: StopReason,
}

/// A Run's Checkpoint is being written (RT-14).
pub const RUN_CHECKPOINTING: Event = Event {
    name: "run.checkpointing",
    description: "A Checkpoint of a Run is being written: on `run.checkpoint`, a pause, a migration, a clean stop or the periodic `checkpoint.interval`. Lossless.",
    class: EventClass::Lossless,
    payload: schema_of::<RunCheckpointing>,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// A Run is being migrated (RT-14).
pub const RUN_MIGRATING: Event = Event {
    name: "run.migrating",
    description: "A Run is being migrated to another machine: it checkpoints, then stops. Lossless.",
    class: EventClass::Lossless,
    payload: schema_of::<RunMigrating>,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// A Run is being aborted (RT-14).
pub const RUN_ABORTING: Event = Event {
    name: "run.aborting",
    description: "A Run is being aborted: it stops at its next safe point. Lossless.",
    class: EventClass::Lossless,
    payload: schema_of::<RunAborting>,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// A Run is stopping (RT-14).
pub const RUN_STOPPING: Event = Event {
    name: "run.stopping",
    description: "A Run is stopping at its next safe point, with its cause: an abort, a pause or a migration. Lossless.",
    class: EventClass::Lossless,
    payload: schema_of::<RunStopping>,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// A file the engine could not write where it keeps it (ADR 0116).
pub const FILE_WRITE_FAILED: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "file_write_failed",
    description: "The bytes of a file could not be written: no space or quota left, or an I/O error, or any failure where the engine keeps its files. No partial file is left. Free space where the files go, or ask the Daemon's operator, then make the same call again.",
    group: ErrorGroup::Unavailable,
    retryable: false,
    exit_code: Some(14),
    arrived_in: ARRIVED_IN,
};

/// A path the program chose that cannot be written, for a reason the path
/// causes (ADR 0116): the other half of `file_write_failed`, split by
/// remedy.
pub const PATH_NOT_WRITABLE: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "path_not_writable",
    description: "The path the program chose cannot be written for a reason the path causes: permission denied, a read-only filesystem, a folder at the path, a file where a parent folder should be, a name the filesystem refuses, a file too large for that filesystem, or a path not found once its parents are made. No partial file is left. Change the path.",
    group: ErrorGroup::Program,
    retryable: false,
    exit_code: Some(13),
    arrived_in: ARRIVED_IN,
};

/// Whose storage a write failed on (ADR 0116).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum WriteSide {
    /// Where the engine keeps its files.
    Engine,
    /// The program's machine.
    Program,
}

/// What `file_write_failed` details.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct FileWriteFailedDetails {
    /// Why, as Rust's `io::ErrorKind` name in `snake_case`, such as
    /// `storage_full` or `read_only_filesystem`, or `other`.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub reason: String,
    /// The OS's own name for the error, such as `ENOSPC`, or none.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub os_code: Option<String>,
    /// On the engine's side, the folder where it keeps the Run; on the
    /// program's, its path as given.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub path: Option<String>,
    /// Whose storage failed.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub side: WriteSide,
}

/// What `path_not_writable` details (ADR 0116).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PathNotWritableDetails {
    /// Why, as Rust's `io::ErrorKind` name in `snake_case`, such as
    /// `permission_denied` or `is_a_directory`.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub reason: String,
    /// The OS's own name for the error, such as `EACCES`, or none.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub os_code: Option<String>,
    /// The program's path, as given.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub path: String,
    /// What to change in the path argument.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub fix: String,
}

/// A file of a format version this engine cannot read (ADRs 0030 and 0066).
pub const FORMAT_VERSION_UNSUPPORTED: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "format_version_unsupported",
    description: "A file, such as a run state or a Checkpoint, is of a format version this engine cannot read: a newer minor or major, written by a newer engine, which `fix` says to upgrade to, or an older major, which `upgrade-files` upgrades. The file is left as it is.",
    group: ErrorGroup::Unavailable,
    retryable: false,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// What `format_version_unsupported` details.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct FormatVersionUnsupportedDetails {
    /// The format's name.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub format: String,
    /// The file's version.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub found: String,
    /// The version this engine reads up to.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub supported: String,
    /// What to do: upgrade the engine, or run `upgrade-files`.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub fix: String,
}

/// A Run bound to another machine (ADR 0056).
pub const SESSION_BOUND_TO_MACHINE: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "session_bound_to_machine",
    description: "The Run is bound to the machine its Session source is on, and this is another machine: its login would reach sites from a new IP under the same cookies. Resume it on its own machine, or serve its Session source through a Browser host.",
    group: ErrorGroup::Refused,
    retryable: false,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// What `session_bound_to_machine` details.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SessionBoundToMachineDetails {
    /// The Run's ID.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub run_id: RunId,
}

/// A Checkpoint behind the run file it would replace (ADR 0002's note of
/// #2322).
pub const CHECKPOINT_BEHIND: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "checkpoint_behind",
    description: "The Checkpoint a resume names is behind the Run's run file on this machine: a later run epoch, or the same one at a later journal position, wrote that file, so replacing it would throw away the work done since the Checkpoint and could perform its Effects again. Nothing is changed. Resume without `checkpoint` to go on from the run file, or pass `rollback` to replace it with the Checkpoint, as the Audit log records.",
    group: ErrorGroup::Refused,
    retryable: false,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// Where a Run's state stands: the run epoch of its last writer, then the
/// journal position within it, compared in that order.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
pub struct RunStatePosition {
    /// The run epoch of the writer whose commits it holds.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub run_epoch: u64,
    /// How many commits of the run file it holds.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub journal_position: u64,
}

/// What `checkpoint_behind` details.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CheckpointBehindDetails {
    /// The Run's ID.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub run_id: RunId,
    /// Where the Checkpoint stands.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub checkpoint: RunStatePosition,
    /// Where the run file on this machine stands, past the Checkpoint.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub run_file: RunStatePosition,
    /// What to do: resume without `checkpoint`, or roll back.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub fix: String,
}

fn none() -> serde_json::Value {
    serde_json::Value::Null
}

/// `checkpoint.interval` (ADRs 0002 and 0102).
pub const CHECKPOINT_INTERVAL: Setting = Setting {
    name: "checkpoint.interval",
    description: "How often a live Run writes a Checkpoint where the engine keeps its Checkpoints, on the engine's clock; none by default locally.",
    kind: ValueKind::Duration,
    default: none,
    clearable: true,
    authority: Authority::Every,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `checkpoint.object_storage.s3.endpoint` (issue 3472).
pub const CHECKPOINT_OBJECT_STORAGE_S3_ENDPOINT: Setting = Setting {
    name: "checkpoint.object_storage.s3.endpoint",
    description: "Endpoint URL for `s3://` Checkpoints, for S3-compatible stores such as R2 or `RustFS`; defaults to AWS S3 for the configured region.",
    kind: ValueKind::Text,
    default: none,
    clearable: true,
    authority: Authority::Every,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `checkpoint.object_storage.s3.region` (issue 3472).
pub const CHECKPOINT_OBJECT_STORAGE_S3_REGION: Setting = Setting {
    name: "checkpoint.object_storage.s3.region",
    description: "Signing region for `s3://` Checkpoints; defaults to `us-east-1`.",
    kind: ValueKind::Text,
    default: none,
    clearable: true,
    authority: Authority::Every,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `checkpoint.object_storage.s3.access_key_id` (issue 3472).
pub const CHECKPOINT_OBJECT_STORAGE_S3_ACCESS_KEY_ID: Setting = Setting {
    name: "checkpoint.object_storage.s3.access_key_id",
    description: "Secret reference for the AWS access key ID used for `s3://` Checkpoints.",
    kind: ValueKind::SecretReference,
    default: none,
    clearable: true,
    authority: Authority::Every,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `checkpoint.object_storage.s3.secret_access_key` (issue 3472).
pub const CHECKPOINT_OBJECT_STORAGE_S3_SECRET_ACCESS_KEY: Setting = Setting {
    name: "checkpoint.object_storage.s3.secret_access_key",
    description: "Secret reference for the AWS secret access key used for `s3://` Checkpoints.",
    kind: ValueKind::SecretReference,
    default: none,
    clearable: true,
    authority: Authority::Every,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `checkpoint.object_storage.s3.session_token` (issue 3472).
pub const CHECKPOINT_OBJECT_STORAGE_S3_SESSION_TOKEN: Setting = Setting {
    name: "checkpoint.object_storage.s3.session_token",
    description: "Optional Secret reference for the AWS session token used for temporary credentials on `s3://` Checkpoints.",
    kind: ValueKind::SecretReference,
    default: none,
    clearable: true,
    authority: Authority::Every,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `checkpoint.object_storage.gs.endpoint` (issue 3472).
pub const CHECKPOINT_OBJECT_STORAGE_GS_ENDPOINT: Setting = Setting {
    name: "checkpoint.object_storage.gs.endpoint",
    description: "Endpoint URL for `gs://` Checkpoints; defaults to `https://storage.googleapis.com`.",
    kind: ValueKind::Text,
    default: none,
    clearable: true,
    authority: Authority::Every,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `checkpoint.object_storage.gs.access_token` (issue 3472).
pub const CHECKPOINT_OBJECT_STORAGE_GS_ACCESS_TOKEN: Setting = Setting {
    name: "checkpoint.object_storage.gs.access_token",
    description: "Secret reference for the `OAuth2` bearer access token used for `gs://` Checkpoints.",
    kind: ValueKind::SecretReference,
    default: none,
    clearable: true,
    authority: Authority::Every,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `checkpoint.object_storage.az.endpoint` (issue 3472).
pub const CHECKPOINT_OBJECT_STORAGE_AZ_ENDPOINT: Setting = Setting {
    name: "checkpoint.object_storage.az.endpoint",
    description: "Endpoint URL for `az://` Checkpoints, such as an Azurite emulator URL; defaults to `https://<account>.blob.core.windows.net`.",
    kind: ValueKind::Text,
    default: none,
    clearable: true,
    authority: Authority::Every,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `checkpoint.object_storage.az.account` (issue 3472).
pub const CHECKPOINT_OBJECT_STORAGE_AZ_ACCOUNT: Setting = Setting {
    name: "checkpoint.object_storage.az.account",
    description: "Azure Storage account name used for `az://` Checkpoints.",
    kind: ValueKind::Text,
    default: none,
    clearable: true,
    authority: Authority::Every,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `checkpoint.object_storage.az.account_key` (issue 3472).
pub const CHECKPOINT_OBJECT_STORAGE_AZ_ACCOUNT_KEY: Setting = Setting {
    name: "checkpoint.object_storage.az.account_key",
    description: "Secret reference for the Base64-encoded Azure Storage `SharedKey` used for `az://` Checkpoints.",
    kind: ValueKind::SecretReference,
    default: none,
    clearable: true,
    authority: Authority::Every,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `checkpoint.object_storage.az.sas_token` (issue 3472).
pub const CHECKPOINT_OBJECT_STORAGE_AZ_SAS_TOKEN: Setting = Setting {
    name: "checkpoint.object_storage.az.sas_token",
    description: "Secret reference for the Azure Storage SAS query token used for `az://` Checkpoints when `account_key` is not set.",
    kind: ValueKind::SecretReference,
    default: none,
    clearable: true,
    authority: Authority::Every,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// What `checkpoint.save_as` takes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CheckpointSaveAsParams {
    /// The Run's ID.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub run_id: RunId,
    /// Where the Checkpoint is on the engine's machine, as `run.checkpoint` answered.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub location: String,
    /// The program's path, absolute, which an Attached engine writes
    /// itself; any other engine leaves it to the SDK and sends the file's
    /// bytes instead.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub path: Option<String>,
}

/// What `checkpoint.save_as` answers once the file is whole.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CheckpointSaved {
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
    /// The run epoch of the writer that took the Checkpoint (ADR 0003).
    #[schemars(extend("arrived_in" = "0.1"))]
    pub run_epoch: u64,
}

/// `checkpoint.save_as`.
pub const CHECKPOINT_SAVE_AS: Operation = Operation {
    name: "checkpoint.save_as",
    description: "Saves a Checkpoint's file, which the engine keeps on its machine, whole or not at all (ADRs 0102 and 0116). An Attached engine given the program's path writes it through a hidden temporary file beside it, synced and renamed over it, creating missing folders and replacing a file there; any other engine writes nothing on its own machine and sends the file in `$/chunk` notifications of at most 64 KiB, one per credit granted with `$/credit`, for the SDK to write. Answers the file's size, its SHA-256 digest and its run epoch. A path that cannot be written raises `path_not_writable`, and full or failing storage `file_write_failed`, leaving the path as it was.",
    class: OperationClass::IdempotentWrite,
    kind: OperationKind::RoundTrip,
    reach: Reach::SdkOnly,
    params: schema_of::<CheckpointSaveAsParams>,
    result: schema_of::<CheckpointSaved>,
    events: &[],
    cancellable: true,
    input: None,
    bound: None,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// What `checkpoint.stream` takes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CheckpointStreamParams {
    /// The Run's ID.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub run_id: RunId,
    /// Where the Checkpoint is on the engine's machine, as `run.checkpoint` answered.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub location: String,
}

/// What `checkpoint.stream` answers once it sent every byte.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CheckpointStreamed {
    /// The file's size in bytes.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub size: u64,
    /// The SHA-256 of its bytes, in lowercase hexadecimal.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub digest: String,
    /// The run epoch of the writer that took the Checkpoint (ADR 0003).
    #[schemars(extend("arrived_in" = "0.1"))]
    pub run_epoch: u64,
}

/// `checkpoint.stream`.
pub const CHECKPOINT_STREAM: Operation = Operation {
    name: "checkpoint.stream",
    description: "Sends a Checkpoint's bytes in order, in `$/chunk` notifications of at most 64 KiB, one per credit granted with `$/credit`, and answers its size, its SHA-256 digest and its run epoch once every byte is sent (ADRs 0102 and 0116). Nothing is written on either machine: a program that writes the bytes itself meets its own language's file errors.",
    class: OperationClass::Read,
    kind: OperationKind::Sequence,
    reach: Reach::SdkOnly,
    params: schema_of::<CheckpointStreamParams>,
    result: schema_of::<CheckpointStreamed>,
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
    operations: &[
        RUN_CHECKPOINT,
        RUN_PAUSE,
        RUN_MIGRATE,
        RUN_RESUME,
        CHECKPOINT_SAVE_AS,
        CHECKPOINT_STREAM,
    ],
    events: &[RUN_CHECKPOINTING, RUN_MIGRATING, RUN_ABORTING, RUN_STOPPING],
    handler_requests: &[],
    error_codes: &[
        FILE_WRITE_FAILED,
        PATH_NOT_WRITABLE,
        FORMAT_VERSION_UNSUPPORTED,
        SESSION_BOUND_TO_MACHINE,
        CHECKPOINT_BEHIND,
    ],
    plugin_kinds: &[],
    metrics: &[],
    settings: &[
        CHECKPOINT_INTERVAL,
        CHECKPOINT_OBJECT_STORAGE_S3_ENDPOINT,
        CHECKPOINT_OBJECT_STORAGE_S3_REGION,
        CHECKPOINT_OBJECT_STORAGE_S3_ACCESS_KEY_ID,
        CHECKPOINT_OBJECT_STORAGE_S3_SECRET_ACCESS_KEY,
        CHECKPOINT_OBJECT_STORAGE_S3_SESSION_TOKEN,
        CHECKPOINT_OBJECT_STORAGE_GS_ENDPOINT,
        CHECKPOINT_OBJECT_STORAGE_GS_ACCESS_TOKEN,
        CHECKPOINT_OBJECT_STORAGE_AZ_ENDPOINT,
        CHECKPOINT_OBJECT_STORAGE_AZ_ACCOUNT,
        CHECKPOINT_OBJECT_STORAGE_AZ_ACCOUNT_KEY,
        CHECKPOINT_OBJECT_STORAGE_AZ_SAS_TOKEN,
    ],
    hook_stages: &[],
    shared_type_values: &[],
};

#[cfg(test)]
mod tests;
