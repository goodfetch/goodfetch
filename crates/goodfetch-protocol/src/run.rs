//! A Run's operations, which every Product carries (ADRs 0029, 0036 and
//! 0102): `run.start`, which begins a Run or returns the one its Run ID
//! names, `run.finish`, which waits for its end, and `run.abort`, which stops
//! it; the Run report's three axes; the Run ID and the Run's tags; the
//! setting that places run files; the Error codes they answer with; and the
//! rows of run state and of the Run index (ADR 0031), the two formats this
//! module's rows describe (ADR 0066).

use std::borrow::Cow;
use std::fmt;

use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{Deserialize, Serialize};

use crate::catalog::{
    Authority, Entries, ErrorCodeDefinition, ErrorGroup, Event, EventClass, Operation,
    OperationClass, Product, ProtocolVersion, Setting, Stability, ValueKind, schema_of,
};
use crate::connection::StoppedBy;
use crate::settings::{ProjectSettings, SettingLimit};

const ARRIVED_IN: ProtocolVersion = ProtocolVersion { major: 0, minor: 1 };

/// The characters of a Run ID, which also names its folder on every OS
/// (ADR 0036): lowercase letters, digits, `.`, `_` and `-`, at most 128,
/// starting and ending with a letter or a digit.
pub const RUN_ID_PATTERN: &str = "^[a-z0-9]([a-z0-9._-]{0,126}[a-z0-9])?$";

/// The most tags a Run carries (ADR 0036).
pub const MAX_TAGS: usize = 16;

/// The most characters one tag holds (ADR 0036).
pub const MAX_TAG_CHARACTERS: usize = 64;

/// The names Windows reserves for devices, refused as a Run ID alone or
/// before an extension, since they cannot name a folder there.
const WINDOWS_RESERVED: [&str; 22] = [
    "con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7", "com8",
    "com9", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9",
];

/// The name that identifies one Run in the Run index for good (ADRs 0036
/// and 0102), safe as a folder name on every OS.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RunId(String);

/// Why a text is no Run ID.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum RunIdInvalid {
    /// It does not match [`RUN_ID_PATTERN`].
    #[error("a Run ID matches `{RUN_ID_PATTERN}`")]
    Characters,
    /// It is a name Windows reserves for a device.
    #[error("a Run ID is never a name Windows reserves for a device")]
    Reserved,
}

impl RunId {
    /// Reads `text` as a Run ID.
    pub fn parse(text: &str) -> Result<Self, RunIdInvalid> {
        let bytes = text.as_bytes();
        let allowed = |byte: &u8| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'.' | b'_' | b'-')
        };
        let alphanumeric = |byte: &u8| byte.is_ascii_lowercase() || byte.is_ascii_digit();
        let well_formed = (1..=128).contains(&bytes.len())
            && bytes.iter().all(allowed)
            && bytes.first().is_some_and(alphanumeric)
            && bytes.last().is_some_and(alphanumeric);
        if !well_formed {
            return Err(RunIdInvalid::Characters);
        }
        let stem = text.split('.').next().unwrap_or(text);
        if WINDOWS_RESERVED.contains(&stem) {
            return Err(RunIdInvalid::Reserved);
        }
        Ok(Self(text.to_owned()))
    }

    /// The ID as written.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for RunId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Serialize for RunId {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for RunId {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = Cow::<str>::deserialize(deserializer)?;
        Self::parse(&text).map_err(serde::de::Error::custom)
    }
}

impl JsonSchema for RunId {
    fn schema_name() -> Cow<'static, str> {
        "RunId".into()
    }

    fn json_schema(_generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "description": "A Run ID (ADR 0036): lowercase letters, digits, `.`, `_` and `-`, starting and ending with a letter or a digit, never a name Windows reserves for a device.",
            "type": "string",
            "pattern": RUN_ID_PATTERN,
        })
    }
}

/// One opaque text a program or a Schedule tags a Run with (ADR 0036):
/// stored in run state and shown in its Run index row, at most
/// [`MAX_TAG_CHARACTERS`] characters.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(transparent)]
pub struct Tag(String);

impl Tag {
    /// The tag as written.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<'de> Deserialize<'de> for Tag {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        if text.chars().count() > MAX_TAG_CHARACTERS {
            return Err(serde::de::Error::custom(format!(
                "a tag holds at most {MAX_TAG_CHARACTERS} characters"
            )));
        }
        Ok(Self(text))
    }
}

impl JsonSchema for Tag {
    fn schema_name() -> Cow<'static, str> {
        "Tag".into()
    }

    fn json_schema(_generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "description": "An opaque text a Run is tagged with (ADR 0036).",
            "type": "string",
            "maxLength": MAX_TAG_CHARACTERS,
        })
    }
}

/// A Run's tags, at most [`MAX_TAGS`], in the order given.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct Tags(Vec<Tag>);

impl Tags {
    /// Each tag, in order.
    pub fn iter(&self) -> impl Iterator<Item = &Tag> {
        self.0.iter()
    }

    /// Whether there is none.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl<'de> Deserialize<'de> for Tags {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let tags = Vec::<Tag>::deserialize(deserializer)?;
        if tags.len() > MAX_TAGS {
            return Err(serde::de::Error::custom(format!(
                "a Run carries at most {MAX_TAGS} tags"
            )));
        }
        Ok(Self(tags))
    }
}

impl JsonSchema for Tags {
    fn schema_name() -> Cow<'static, str> {
        "Tags".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        let tag = generator.subschema_for::<Tag>();
        json_schema!({
            "description": "A Run's tags, in the order given.",
            "type": "array",
            "items": tag,
            "maxItems": MAX_TAGS,
        })
    }
}

/// A point in time, in UTC, written as RFC 3339 (ADR 0092).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PointInTime(String);

impl PointInTime {
    /// The point that `rfc3339` writes, which its writer formats.
    pub fn from_rfc3339(rfc3339: String) -> Self {
        Self(rfc3339)
    }

    /// Its RFC 3339 text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl JsonSchema for PointInTime {
    fn schema_name() -> Cow<'static, str> {
        "PointInTime".into()
    }

    fn json_schema(_generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "description": "A point in time in UTC, as RFC 3339.",
            "type": "string",
            "format": "date-time",
        })
    }
}

/// Where a Run is (ADR 0029).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Lifecycle {
    /// It runs.
    Running,
    /// It ended early: by a Limit, an abort, its timeout, the loss of its
    /// engine or exhausted resources.
    Stopped,
    /// It reached its end.
    Finished,
    /// An error in the engine or the program's code ended it.
    Failed,
}

impl Lifecycle {
    /// Its name on the wire and in run state.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Stopped => "stopped",
            Self::Finished => "finished",
            Self::Failed => "failed",
        }
    }
}

/// Whether a Run's own checks prove what it committed (ADR 0029).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    /// Every check the Run relies on passed, none was switched off, and no
    /// Effect is left uncertain.
    Verified,
    /// A declared Success check failed.
    Rejected,
    /// Everything else: a person reviews it.
    NeedsReview,
}

impl Verdict {
    /// Its name on the wire and in run state.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Verified => "verified",
            Self::Rejected => "rejected",
            Self::NeedsReview => "needs_review",
        }
    }
}

/// Whether a Run's results cover everything in its scope (ADRs 0005 and
/// 0029).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Completeness {
    /// They do.
    Complete,
    /// The Run ended early.
    Incomplete,
    /// It finished, but the engine cannot vouch that nothing was missed.
    Unverified,
}

impl Completeness {
    /// Its name on the wire and in run state.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Complete => "complete",
            Self::Incomplete => "incomplete",
            Self::Unverified => "unverified",
        }
    }
}

/// Why a Run stopped (ADR 0102); the other causes join with the tickets that
/// stop Runs for them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum StopReason {
    /// Its program, a person or a Client aborted it, a Connection's close
    /// included (ADR 0078).
    Abort {
        /// Who stopped it.
        #[schemars(extend("arrived_in" = "0.1"))]
        by: StoppedBy,
    },
}

/// What kind of Run it is: a Collie Run its program drives, or a crawl
/// (ADR 0102), which its start's inputs name.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RunKind {
    /// A Run its program drives page by page.
    Collie,
    /// A crawl, whose work items the engine leases to its handlers.
    Crawl,
}

impl RunKind {
    /// Its name on the wire and in run state.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Collie => "collie",
            Self::Crawl => "crawl",
        }
    }
}

/// What `run.start` takes.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RunStartParams {
    /// The Run's ID; the engine generates a lowercase ULID when it is
    /// absent. An existing ID returns that Run when the caller and the
    /// inputs match, and fails with `run_conflict` otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1", "pattern" = RUN_ID_PATTERN))]
    pub run_id: Option<String>,
    /// The Run's opaque tags.
    #[serde(default, skip_serializing_if = "Tags::is_empty")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub tags: Tags,
    /// The Run's settings, nested by key: the run layer (ADR 0083).
    #[serde(default, skip_serializing_if = "serde_json::Map::is_empty")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub settings: serde_json::Map<String, serde_json::Value>,
    /// The project configuration the Client found, layer 3 (ADR 0083).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub project: Option<ProjectSettings>,
}

/// Whether a start began its Run or found it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum StartedAs {
    /// The start began it.
    New,
    /// Its Run ID named a Run started before with the same inputs, which is
    /// returned as it is.
    Existing,
}

/// What a start answers once its Run is started, never at its end.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RunStarted {
    /// The Run's ID.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub run_id: RunId,
    /// Whether the start began it.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub started_as: StartedAs,
    /// Where it is now.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub lifecycle: Lifecycle,
    /// The Product it needs (ADR 0098): Collie until it uses an entry the
    /// catalog marks Foxhound's, Foxhound from then on.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub needs: Product,
}

/// What `run.finish` and `run.abort` take: the Run.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RunParams {
    /// The Run's ID.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub run_id: RunId,
}

/// What a Run states about itself when it ends (ADR 0102): its three axes
/// (ADR 0029) and when it ran.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RunReport {
    /// The Run's ID.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub run_id: RunId,
    /// Where it ended.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub lifecycle: Lifecycle,
    /// Why it stopped, when it did.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub stop_reason: Option<StopReason>,
    /// Whether its own checks prove what it committed.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub verdict: Verdict,
    /// Whether its results cover its scope.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub completeness: Completeness,
    /// When it started.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub started_at: PointInTime,
    /// When it ended.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub ended_at: PointInTime,
    /// The version of the Operator policy it ran under, when one applied
    /// (ADR 0028).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub policy_version: Option<String>,
    /// Every setting it ran with that weakens a guarantee class or switches
    /// off a costly control, with its value and layer (ADR 0028).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub weakened_by: Vec<SettingLimit>,
}

/// A Run's three axes as they are now (ADR 0029): what
/// `run.axes_changed` reports, and what a resync gives of each Run.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RunAxes {
    /// The Run's ID.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub run_id: RunId,
    /// Where it is.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub lifecycle: Lifecycle,
    /// Whether its own checks prove what it committed, once it ended.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub verdict: Option<Verdict>,
    /// Whether its results cover its scope, once it ended.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub completeness: Option<Completeness>,
}

/// A Run's axes changed: it started, or it ended (ADRs 0029 and 0031).
pub const RUN_AXES_CHANGED: Event = Event {
    name: "run.axes_changed",
    description: "A Run's Lifecycle, Verdict or Completeness changed: it started running, or it ended. Lossless: while a subscriber's queue is full, the Run waits.",
    class: EventClass::Lossless,
    payload: schema_of::<RunAxes>,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `run.start`.
pub const RUN_START: Operation = Operation {
    name: "run.start",
    description: "Starts a Run its program drives, and answers once it is started. A Run ID names the Run for good: a start with an existing Run ID returns that Run as it is when the caller and the inputs match, and fails with `run_conflict` otherwise.",
    class: OperationClass::IdempotentWrite,
    params: schema_of::<RunStartParams>,
    result: schema_of::<RunStarted>,
    events: &[],
    cancellable: false,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `run.finish`.
pub const RUN_FINISH: Operation = Operation {
    name: "run.finish",
    description: "Lets a Run finish and answers its Run report: a Run its program drives ends at once, `finished` and `complete`; a crawl ends once its work items are done. Cancelling the call ends only the wait.",
    class: OperationClass::IdempotentWrite,
    params: schema_of::<RunParams>,
    result: schema_of::<RunReport>,
    events: &[],
    cancellable: true,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `run.abort`.
pub const RUN_ABORT: Operation = Operation {
    name: "run.abort",
    description: "Stops a Run at its next safe point, `stopped` and `incomplete`: no attempt in flight commits, and its work items return to pending. Answers its Run report; an ended Run's report is answered as it is.",
    class: OperationClass::IdempotentWrite,
    params: schema_of::<RunParams>,
    result: schema_of::<RunReport>,
    events: &[],
    cancellable: false,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// A Run ID the engine refuses (ADR 0102).
pub const RUN_ID_INVALID: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "run_id_invalid",
    description: "The Run ID is not one the engine takes: lowercase letters, digits, `.`, `_` and `-`, at most 128, starting and ending with a letter or a digit, never a name Windows reserves for a device.",
    group: ErrorGroup::Program,
    retryable: false,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// What `run_id_invalid` details.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RunIdInvalidDetails {
    /// The pattern a Run ID matches.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub pattern: String,
}

/// A start whose Run ID names a Run of other inputs (ADR 0102).
pub const RUN_CONFLICT: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "run_conflict",
    description: "The Run ID names a Run started with other inputs, or by another caller; the details list each difference. Start under another Run ID.",
    group: ErrorGroup::Program,
    retryable: false,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// An input of a start that differs from the Run's.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum StartInput {
    /// The Product the Run needs.
    Product,
    /// A crawl or a Collie Run.
    Kind,
    /// Who started it.
    Caller,
    /// A crawl's seeds.
    Seeds,
}

/// What `run_conflict` details.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RunConflictDetails {
    /// The Run ID.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub run_id: RunId,
    /// Each input that differs from the Run's.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub differences: Vec<StartInput>,
}

/// A Run this engine knows nothing of (ADR 0102).
pub const RUN_NOT_FOUND: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "run_not_found",
    description: "No Run has this Run ID: the Run index does not list it.",
    group: ErrorGroup::Program,
    retryable: false,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// A Run whose writer's lock another holds (ADRs 0031 and 0102).
pub const RUN_IN_USE: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "run_in_use",
    description: "Another engine holds the Run's writer lock, so this one cannot write the Run; the other may end.",
    group: ErrorGroup::Refused,
    retryable: true,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// A Run whose state cannot be read or written (ADR 0102).
pub const RUN_STATE_UNAVAILABLE: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "run_state_unavailable",
    description: "The Run's state is missing, damaged or cannot be written at its location; the details give the path. The engine never starts afresh under that Run ID.",
    group: ErrorGroup::Unavailable,
    retryable: false,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// What `run_not_found`, `run_in_use` and `run_state_unavailable` detail.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RunDetails {
    /// The Run ID.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub run_id: RunId,
    /// The Run's folder, when the engine knows it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub path: Option<String>,
}

fn no_folder() -> serde_json::Value {
    serde_json::Value::Null
}

/// `run_state.location`.
pub const RUN_STATE_LOCATION: Setting = Setting {
    name: "run_state.location",
    description: "The folder that holds the run folders, each named by its Run ID; by default the per-user, per-major data folder of the Goodfetch home.",
    kind: ValueKind::Text,
    default: no_folder,
    clearable: true,
    authority: Authority::Every,
    weakens: None,
    arrived_in: ARRIVED_IN,
};

/// One row of a Run's run state about itself: its `run` table (ADR 0002).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RunStateRow {
    /// The Run's ID.
    pub run_id: RunId,
    /// What kind of Run it is.
    pub kind: RunKind,
    /// The Product it needs (ADR 0098).
    pub needs: Product,
    /// Who started it (ADR 0102).
    pub caller: String,
    /// The current run epoch (ADR 0003).
    pub epoch: u64,
    /// Its start's inputs, as canonical JSON, which a later start compares.
    pub inputs: String,
    /// Where it is.
    pub lifecycle: Lifecycle,
    /// Why it stopped.
    pub stop_reason: Option<StopReason>,
    /// Its Verdict, once it ended.
    pub verdict: Option<Verdict>,
    /// Its Completeness, once it ended.
    pub completeness: Option<Completeness>,
    /// When it started.
    pub started_at: PointInTime,
    /// When it ended.
    pub ended_at: Option<PointInTime>,
    /// The version of the Operator policy it ran under (ADR 0028).
    pub policy_version: Option<String>,
    /// The settings it ran with that weaken a guarantee or switch off a
    /// costly control, as a JSON list of each one's key, value and layer
    /// (ADR 0028).
    pub weakened_by: String,
}

/// One row of the Run index (ADR 0031): a Run, where its state is, and its
/// three axes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RunIndexRow {
    /// The Run's ID.
    pub run_id: RunId,
    /// Its folder: relative to the per-major data folder when it is inside,
    /// absolute otherwise.
    pub location: String,
    /// What kind of Run it is.
    pub kind: RunKind,
    /// The Product it needs (ADR 0098).
    pub needs: Product,
    /// Its tags (ADR 0036).
    pub tags: Tags,
    /// Where it is.
    pub lifecycle: Lifecycle,
    /// Its Verdict, once it ended.
    pub verdict: Option<Verdict>,
    /// Its Completeness, once it ended.
    pub completeness: Option<Completeness>,
    /// When it started.
    pub started_at: PointInTime,
    /// When it ended.
    pub ended_at: Option<PointInTime>,
}

/// Every entry this module defines.
pub const ENTRIES: Entries = Entries {
    defined_in: module_path!(),
    operations: &[RUN_START, RUN_FINISH, RUN_ABORT],
    events: &[RUN_AXES_CHANGED],
    handler_requests: &[],
    error_codes: &[
        RUN_ID_INVALID,
        RUN_CONFLICT,
        RUN_NOT_FOUND,
        RUN_IN_USE,
        RUN_STATE_UNAVAILABLE,
    ],
    plugin_kinds: &[],
    metrics: &[],
    settings: &[RUN_STATE_LOCATION],
    hook_stages: &[],
    shared_type_values: &[],
};

#[cfg(test)]
mod tests;
