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
    /// Where to write the Checkpoint, a path on the engine's machine; by
    /// default where the engine keeps the Run's Checkpoints, its own folder.
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
    /// A Checkpoint to resume from, a path on the engine's machine, which
    /// becomes the Run's state in this engine's run folder; by default the
    /// Run resumes from its run file on this machine.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub checkpoint: Option<String>,
    /// The Run's settings from the resume on, nested by key: the run layer
    /// (ADRs 0083 and 0102).
    #[serde(default, skip_serializing_if = "serde_json::Map::is_empty")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub settings: serde_json::Map<String, serde_json::Value>,
    /// The project configuration the Client found, layer 3 (ADR 0083).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub project: Option<ProjectSettings>,
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
    description: "Writes a Checkpoint of a live Run now, through SQLite's online backup, where `location` says or where the engine keeps the Run's Checkpoints, and answers it once its file is complete, with its size, digest, run epoch and journal position. A Checkpoint the engine cannot write fails with `file_write_failed` and leaves no file.",
    class: OperationClass::Effect,
    kind: OperationKind::RoundTrip,
    reach: Reach::AllSurfaces,
    params: schema_of::<RunCheckpointParams>,
    result: schema_of::<Checkpoint>,
    events: &[&RUN_CHECKPOINTING],
    cancellable: false,
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
    params: schema_of::<RunCheckpointParams>,
    result: schema_of::<RunReport>,
    events: &[&RUN_MIGRATING, &RUN_CHECKPOINTING, &RUN_STOPPING],
    cancellable: false,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `run.resume`.
pub const RUN_RESUME: Operation = Operation {
    name: "run.resume",
    description: "Resumes an interrupted Run from its committed state, its run file on this machine or the Checkpoint `checkpoint` names: it takes the Run's writer lock and the next run epoch, so every write of an earlier writer is refused, and redoes the work since that state; its recorded Clean-up rules digest is kept. A live Run is answered as it is, `existing`; a finished one as it is. A Run bound to another machine fails with `session_bound_to_machine`, and a Checkpoint of a format version this engine cannot read with `format_version_unsupported`.",
    class: OperationClass::IdempotentWrite,
    kind: OperationKind::Opener,
    reach: Reach::AllSurfaces,
    params: schema_of::<RunResumeParams>,
    result: schema_of::<RunStarted>,
    events: &[],
    cancellable: false,
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
    arrived_in: ARRIVED_IN,
};

/// Every entry this module defines.
pub const ENTRIES: Entries = Entries {
    defined_in: module_path!(),
    operations: &[RUN_CHECKPOINT, RUN_PAUSE, RUN_MIGRATE, RUN_RESUME],
    events: &[RUN_CHECKPOINTING, RUN_MIGRATING, RUN_ABORTING, RUN_STOPPING],
    handler_requests: &[],
    error_codes: &[
        FILE_WRITE_FAILED,
        PATH_NOT_WRITABLE,
        FORMAT_VERSION_UNSUPPORTED,
        SESSION_BOUND_TO_MACHINE,
    ],
    plugin_kinds: &[],
    metrics: &[],
    settings: &[CHECKPOINT_INTERVAL],
    hook_stages: &[],
    shared_type_values: &[],
};
