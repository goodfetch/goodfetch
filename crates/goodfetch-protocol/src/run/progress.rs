//! A Run's progress (ADR 0102): what it does and why it waits, never a
//! count with no reason. `run.progress` answers it at once, then, given
//! the position of the value a reader last read, the latest value after a
//! change, the changes in between coalesced as telemetry, until the last
//! value, which says how the Run ended or that no engine works it. The
//! engine produces each value; the SDKs, the CLI and the Kennel render it.
//! A crawl's parts, Foxhound's `CrawlRunProgress`, sit beside every Run's,
//! and a Collie reader keeps them as it got them.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::catalog::{
    ErrorCodeDefinition, ErrorGroup, Event, EventClass, Operation, OperationClass, OperationKind,
    ProtocolVersion, Reach, Stability, schema_of,
};
use crate::storage::StorageLevel;

use super::{Lifecycle, PointInTime, ProgressMessage, RunId, RunParams, RunReport, StopReason};

const ARRIVED_IN: ProtocolVersion = ProtocolVersion { major: 0, minor: 1 };

/// A Run's progress as it is now (ADR 0102): what `run.progress` answers
/// and the `run.progress` event carries.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RunProgress {
    /// The Run's ID.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub run_id: RunId,
    /// Its place among the Run's progress values, which only goes up:
    /// `run.progress`'s `after` takes it.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub position: u64,
    /// When the engine read it.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub at: PointInTime,
    /// Where the Run is.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub lifecycle: Lifecycle,
    /// Whether an engine works it.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub writer: RunWriter,
    /// Its run epoch (ADR 0003), which a resume or a migration advances.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub epoch: u64,
    /// When its last Checkpoint was written, once one was.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub checkpoint_at: Option<PointInTime>,
    /// Each Human pause waiting (ADR 0092).
    #[schemars(extend("arrived_in" = "0.1"))]
    pub pauses: Vec<HumanPause>,
    /// The program's own status line, once it set one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub message: Option<ProgressMessage>,
    /// The worst storage level among the filesystems the Run writes to,
    /// once one is above `normal` (ADR 0122).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub storage: Option<RunStorage>,
    /// Why it stopped, once it did.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub stop_reason: Option<StopReason>,
    /// Whether it is the sequence's last value: the Run ended, or no
    /// engine this one reaches works it, so nothing more comes until a
    /// start with its Run ID resumes it.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub last: bool,
    /// What its kind of Run adds, such as a crawl's parts, kept as the
    /// engine sent them.
    #[serde(flatten)]
    pub kind_parts: Map<String, Value>,
}

/// Whether an engine works a Run (ADR 0102).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum RunWriter {
    /// An engine holds its writer lock.
    #[serde(rename = "live")]
    Live,
    /// Nothing works it, as after a crash, until a start with its Run ID
    /// resumes it.
    #[serde(rename = "none")]
    Absent,
}

/// The worst storage level among the filesystems a Run writes to, with
/// that filesystem's folder and free space (ADR 0122).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RunStorage {
    /// The level.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub level: StorageLevel,
    /// A folder the Run writes to on that filesystem.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub folder: String,
    /// The space left on it to the engine's user, in bytes.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub free_bytes: u64,
}

/// A Human pause waiting for a person (ADR 0092).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct HumanPause {
    /// The Session whose page waits.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub session: String,
    /// The page that waits.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub page: String,
    /// Why, as the program or the engine said it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub reason: Option<String>,
    /// The condition that ends it by itself, as text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub until: Option<String>,
    /// Whether the program or the engine raised it.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub raised_by: PauseOrigin,
    /// Since when it waits.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub since: PointInTime,
    /// When it ends unanswered: the first of `timeout.human_pause` and, in
    /// a scheduled run, its pause timeout; none when it waits without
    /// limit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub deadline: Option<PointInTime>,
}

/// Who raised a Human pause.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "by", rename_all = "snake_case")]
pub enum PauseOrigin {
    /// The program, with `page.pause()`.
    Program,
    /// The engine, at a step of its own.
    Engine {
        /// The step.
        #[schemars(extend("arrived_in" = "0.1"))]
        step: EngineStep,
    },
}

/// The engine's step that raised a Human pause.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum EngineStep {
    /// A login recovery (ADR 0006).
    LoginRecovery,
    /// The Challenge ladder (ADR 0035).
    ChallengeLadder,
}

/// What `run.progress` takes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RunProgressParams {
    /// The Run's ID.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub run_id: RunId,
    /// The position of the value the reader last read: the answer is the
    /// latest value after it, once one comes. Absent, the Run's progress
    /// at once.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub after: Option<u64>,
}

/// A Run's progress changed (ADR 0102).
pub const RUN_PROGRESS: Event = Event {
    name: "run.progress",
    description: "A Run's progress changed, such as the program's own status line, a Human pause beginning or ending, or its end. Telemetry: a newer value of the same Run replaces an unsent one.",
    class: EventClass::Telemetry,
    payload: schema_of::<RunProgress>,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `run.progress`.
pub const RUN_PROGRESS_READ: Operation = Operation {
    name: "run.progress",
    description: "Reads a Run's progress: what it does and why it waits. Without `after`, it answers at once; given `after`, the position of the value the reader last read, it waits for a change and answers the latest value, so values in between are coalesced. The value whose `last` is true ends the sequence: the Run ended, and its `lifecycle` and `stop_reason` say how, or no engine works it (`writer: none`) until a start with its Run ID resumes it. A Run another engine works is read through that engine, the owner the Run index names, and one no engine works from its committed state; a read never takes the Run's writer lock. A Run ID the Run index does not list fails with `run_not_found`.",
    class: OperationClass::Read,
    kind: OperationKind::RoundTrip,
    reach: Reach::AllSurfaces,
    params: schema_of::<RunProgressParams>,
    result: schema_of::<RunProgress>,
    events: &[],
    cancellable: true,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `run.report`.
pub const RUN_REPORT: Operation = Operation {
    name: "run.report",
    description: "Reads a Run's report: at once for a Run that ended, and at its end for a live one, which the read never ends. A Run another engine works is read through that engine, and one no engine works from its committed state, never taking its writer lock. A Run that has no live writer and has not ended fails with `run_interrupted`, whose `fix` names a start with its Run ID, and a Run ID the Run index does not list with `run_not_found`.",
    class: OperationClass::Read,
    kind: OperationKind::RoundTrip,
    reach: Reach::AllSurfaces,
    params: schema_of::<RunParams>,
    result: schema_of::<RunReport>,
    events: &[],
    cancellable: true,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// A report read of a Run no engine works (ADR 0102).
pub const RUN_INTERRUPTED: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "run_interrupted",
    description: "The Run has no live writer and has not ended, as after a crash, so it has no report and none comes: a start with its Run ID resumes it. The details name the Run.",
    group: ErrorGroup::Unavailable,
    retryable: false,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// What `run_interrupted`'s details hold.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RunInterruptedDetails {
    /// The Run.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub run_id: RunId,
    /// What resumes it: a start with its Run ID.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub fix: String,
}
