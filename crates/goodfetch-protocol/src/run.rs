//! A Run's operations, which every Product carries (ADRs 0029, 0036 and
//! 0102): `run.start`, which begins a Run or returns the one its Run ID
//! names, `run.finish`, which waits for its end, and `run.abort`, which stops
//! it, taking the writer role of a Run whose engine died or forwarding the
//! abort to the engine that owns it; the Run report's three axes; the Run
//! ID and the Run's tags; the setting that places run files; the Error codes
//! they answer with; the rows of run state and of the Run index (ADR 0031),
//! the two formats this module's rows describe (ADR 0066); `run.list`,
//! which answers the Run index's rows filtered and sorted; and what
//! `runs reindex --json` prints.
//!
//! Size: the contract of one Run's lifecycle and its rows, kept whole so the
//! operations, their Error codes and the run-state formats they write change
//! together (ADR 0066).

use std::borrow::Cow;
use std::fmt;

use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{Deserialize, Serialize};

use crate::catalog::{
    Authority, Entries, ErrorCodeDefinition, ErrorGroup, Event, EventClass, Operation,
    OperationClass, OperationKind, Product, ProtocolVersion, Reach, Setting, Stability, ValueKind,
    schema_of,
};
use crate::connection::{DaemonEndpoint, PrivateEndpoint, StoppedBy};
use crate::settings::{ProjectSettings, SettingLimit};

pub mod completeness;
pub mod durability;
mod list;
mod progress;
mod report;
pub mod warning;

pub use self::durability::{
    DURABILITY_BATCH, DURABILITY_FAST, DURABILITY_FULL, DURABILITY_OFF, DURABILITY_PERIODIC,
    RUN_STATE_BATCH_BYTES, RUN_STATE_BATCH_INTERVAL, RUN_STATE_BATCH_PAGES,
    RUN_STATE_BATCH_PER_ATTEMPT, RUN_STATE_BATCH_SYNC, RUN_STATE_COMMIT_INTERVAL,
    RUN_STATE_DURABILITY, RUN_STATE_MEMORY_BUDGET,
};
pub use self::list::{
    CREATOR_TAG, OCCURRENCE_TAG, RECIPE_TAG, RESERVED_TAG_PREFIX, ROLE_TAG, RUN_LIST, RunList,
    RunListParams, RunSortKey, RunTrigger, SCHEDULE_TAG, SortOrder, TRIGGER_TAG, TimeRange,
    WATCH_ROLE,
};
pub use self::progress::{
    EngineStep, HumanPause, PauseOrigin, RUN_INTERRUPTED, RUN_PROGRESS, RUN_PROGRESS_READ,
    RUN_REPORT, RunInterruptedDetails, RunProgress, RunProgressParams, RunStorage, RunWriter,
};

pub use self::report::{
    AppliedLimit, AxisReason, EndingError, KindUsage, LimitSetter, ModelAllowance, ModelCounts,
    ModelUsage, NothingDone, PlanReport, PlanStatus, PluginRun, ProxyKindBytes, RecordingReport,
    RunReport, RunUsage, RunWarning, SeedsKept, UncertainEffect, UsageAmount,
};

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

/// The most characters a Run's progress message holds (spec #213, RT-15).
pub const MAX_PROGRESS_MESSAGE_CHARACTERS: usize = 512;

/// A program's own status line about its Run (RT-15, ADR 0102), kept in run
/// state, carried on its progress and in its Run index row, at most
/// [`MAX_PROGRESS_MESSAGE_CHARACTERS`] characters.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct ProgressMessage(String);

impl ProgressMessage {
    /// Reads `text` as a progress message, `None` when it is too long.
    pub fn parse(text: &str) -> Option<Self> {
        (text.chars().count() <= MAX_PROGRESS_MESSAGE_CHARACTERS).then(|| Self(text.to_owned()))
    }

    /// The message as written.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<'de> Deserialize<'de> for ProgressMessage {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = Cow::<str>::deserialize(deserializer)?;
        Self::parse(&text).ok_or_else(|| {
            serde::de::Error::custom(format!(
                "a progress message holds at most {MAX_PROGRESS_MESSAGE_CHARACTERS} characters"
            ))
        })
    }
}

impl JsonSchema for ProgressMessage {
    fn schema_name() -> Cow<'static, str> {
        "ProgressMessage".into()
    }

    fn json_schema(_generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "description": "A program's own status line about its Run (RT-15).",
            "type": "string",
            "maxLength": MAX_PROGRESS_MESSAGE_CHARACTERS,
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
    /// Every value, in declaration order.
    pub const ALL: [Self; 4] = [Self::Running, Self::Stopped, Self::Finished, Self::Failed];

    /// Its name on the wire and in run state.
    pub fn as_str(self) -> &'static str {
        crate::wire_name::of(&self)
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

/// The code of the Verdict reason a Run that left an Effect uncertain is
/// given, `needs_review` (ADR 0029).
pub const UNCERTAIN_EFFECTS: &str = "uncertain_effects";

impl Verdict {
    /// Every value, in declaration order.
    pub const ALL: [Self; 3] = [Self::Verified, Self::Rejected, Self::NeedsReview];

    /// Its name on the wire and in run state.
    pub fn as_str(self) -> &'static str {
        crate::wire_name::of(&self)
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
    /// Every value, in declaration order.
    pub const ALL: [Self; 3] = [Self::Complete, Self::Incomplete, Self::Unverified];

    /// Its name on the wire and in run state.
    pub fn as_str(self) -> &'static str {
        crate::wire_name::of(&self)
    }
}

/// Why a Run stopped (ADRs 0029, 0080 and 0102).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum StopReason {
    /// Its program, a person or a Client aborted it, a Connection's close
    /// included (ADR 0078).
    Abort {
        /// Who stopped it.
        #[schemars(extend("arrived_in" = "0.1"))]
        by: StoppedBy,
    },
    /// It was paused: checkpointed and stopped, to be resumed by its Run ID
    /// (RT-11).
    Pause,
    /// It was migrated: checkpointed and stopped, to be resumed from its
    /// Checkpoint on another machine (ADR 0002).
    Migrate,
    /// Its `timeout.run` passed, and it stopped at its next safe point,
    /// resumable with a fresh clock (ADR 0080); no Error code of its own.
    Timeout,
    /// Its foreground Daemon drained on `SIGTERM`: checkpointed and
    /// stopped, to be resumed by its Run ID on the Daemon's next start
    /// (ADR 0049).
    Drain,
    /// It could not take another Slice of a Limit, and stopped at its next
    /// safe point, resumable once the period starts again or a person
    /// raises the Limit (ADR 0041).
    Limit(Box<ReachedLimit>),
    /// A Limit flagged *must understand* that its engine cannot evaluate
    /// came to cover it when it charged, and it stopped at its next safe
    /// point, resumable by an engine that can (ADR 0041): the stop of the
    /// Error code `limit_not_understood`.
    LimitNotUnderstood(Box<NotUnderstoodLimit>),
    /// Its engine was lost while it ran, and another engine stopped it.
    EngineLost,
    /// Its Attached engine stopped on a panic in a part every Run shares,
    /// and stopped it first, resumable by its Run ID (ADR 0148); a Daemon
    /// drains instead.
    EnginePanicked,
    /// The resource governor ran out of a resource for it, and it stopped
    /// at its next safe point, resumable (ADRs 0040 and 0122).
    Resources {
        /// The resource that ran out.
        #[schemars(extend("arrived_in" = "0.1"))]
        resource: Resource,
        /// For storage, the folder whose filesystem filled.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[schemars(extend("arrived_in" = "0.1"))]
        folder: Option<String>,
    },
}

/// A resource the governor can run out of (ADR 0122).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Resource {
    /// The machine's or the container's memory.
    Memory,
    /// A filesystem the Run writes to.
    Storage,
}

/// A Limit flagged *must understand* that a Run's engine cannot evaluate,
/// as the Run stopped for it names it (ADR 0041): its scope, Usage kind,
/// currency and period as the ledger holds them.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct NotUnderstoodLimit {
    /// The Limit's scope.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub scope: LimitScope,
    /// The Usage kind it caps.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub usage_kind: String,
    /// The currency it caps a price in, for a price Limit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub currency: Option<String>,
    /// Its period; none for a Limit that never starts again.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub period: Option<Period>,
}

/// What a Limit is on: a scope kind, a value of a type both Products share
/// (`install`, `daemon`, `person`, `token`, `connection`, and Foxhound's
/// `partitioned_crawl`), and which one of its kind (ADR 0041). A kind this
/// reader does not know is kept as it is.
#[derive(
    Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
pub struct LimitScope {
    /// The scope's kind.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub kind: String,
    /// Which one of its kind: the major's name for a Daemon, the person's
    /// name, the token's name or the Client connection's ID; empty for the
    /// install.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub id: String,
}

/// How long a period Limit counts before it starts again (ADR 0041).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PeriodLength {
    /// From one midnight to the next.
    Day,
    /// From Monday's midnight to the next Monday's.
    Week,
    /// From the first day's midnight to the next month's.
    Month,
}

impl PeriodLength {
    /// Every value, in declaration order.
    pub const ALL: [Self; 3] = [Self::Day, Self::Week, Self::Month];

    /// Its name on the wire and in the Usage ledger.
    pub fn as_str(self) -> &'static str {
        crate::wire_name::of(&self)
    }
}

/// The period a Limit counts over, in an explicit IANA time zone, so that
/// it never depends on the host's (ADR 0041).
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
pub struct Period {
    /// How long it counts.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub every: PeriodLength,
    /// The IANA time zone its midnights are in, such as `Europe/Paris`.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub time_zone: String,
}

/// The Limit a Run cannot take more of: what `limit_reached` details, and
/// what a Run stopped for it names (ADR 0041).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ReachedLimit {
    /// The Limit's scope.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub scope: LimitScope,
    /// The Usage kind it caps.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub usage_kind: String,
    /// The currency it caps a price in, for a price Limit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub currency: Option<String>,
    /// Its period; none for a Limit that never starts again.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub period: Option<Period>,
    /// When its period starts again; none for a Limit without one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub resets_at: Option<PointInTime>,
}

/// What kind of Run it is: a Collie Run its program drives, or a crawl
/// (ADR 0102), which its start's inputs name.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum RunKind {
    /// A Run its program drives page by page.
    Collie,
    /// A crawl, whose work items the engine leases to its handlers.
    Crawl,
}

impl RunKind {
    /// Every value, in declaration order.
    pub const ALL: [Self; 2] = [Self::Collie, Self::Crawl];

    /// Its name on the wire and in run state.
    pub fn as_str(self) -> &'static str {
        crate::wire_name::of(&self)
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
    /// It was interrupted and goes on from its committed state, its run
    /// file or a Checkpoint, at the next run epoch (ADRs 0002, 0003 and
    /// 0102).
    Resumed,
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

/// What `run.usage`, `run.pause` and `run.migrate` take: the Run.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RunParams {
    /// The Run's ID.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub run_id: RunId,
}

/// What `run.discard` answers: the Run it discarded (ADRs 0001 and 0036).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RunDiscarded {
    /// The Run's ID.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub run_id: RunId,
}

/// What `run.finish` and `run.stop` take: the Run, and how long the call
/// waits for its end.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RunWaitParams {
    /// The Run's ID.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub run_id: RunId,
    /// How long the call waits for the Run's end, in milliseconds; past it,
    /// it fails with `deadline_exceeded` and only the wait ends. None by
    /// default for `run.finish`, which the Run's own `timeout.run` bounds;
    /// `timeout.close` by default for `run.stop`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub timeout: Option<u64>,
}

/// What `run.abort` takes: the Run, and whether the abort was forwarded.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RunAbortParams {
    /// The Run's ID.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub run_id: RunId,
    /// Set by an engine that forwards the abort to the owner the Run index
    /// names (ADR 0031): an engine that does not own the Run then answers
    /// `run_not_owned` instead of taking its writer role.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub forwarded: bool,
}

/// A Checkpoint as the engine reports it (ADRs 0002, 0102 and 0116): a
/// consistent, self-contained copy of a Run's state, complete at its
/// location before it is reported, from which the Run resumes on this
/// machine or another one.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Checkpoint {
    /// The Run's ID.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub run_id: RunId,
    /// Where it is: the file's path on the engine's machine.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub location: String,
    /// Its size in bytes.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub size: u64,
    /// The SHA-256 of its bytes, in lowercase hexadecimal.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub digest: String,
    /// The run epoch of the writer that took it (ADR 0003), which a
    /// destination compares and sets: one older than it holds is refused.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub run_epoch: u64,
    /// The position of the run's journal it holds: how many commits of the
    /// run file it contains. A resume from it redoes the work after.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub journal_position: u64,
    /// When it was taken.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub time: PointInTime,
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

/// What `run.set_progress_message` takes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RunSetProgressMessageParams {
    /// The Run's ID.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub run_id: RunId,
    /// The program's status line, which replaces the one before.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub message: ProgressMessage,
}

/// `run.set_progress_message`.
pub const RUN_SET_PROGRESS_MESSAGE: Operation = Operation {
    name: "run.set_progress_message",
    description: "Sets the program's own status line about a live Run, at most 512 characters, which replaces the one before: kept in the Run's state, so a resume keeps it, shown in its Run index row and carried on its progress, `run.progress`. Answers the Run's progress. A Run this engine does not run fails with `run_not_found`.",
    class: OperationClass::IdempotentWrite,
    kind: OperationKind::RoundTrip,
    reach: Reach::SdkOnly,
    params: schema_of::<RunSetProgressMessageParams>,
    result: schema_of::<RunProgress>,
    events: &[&RUN_PROGRESS],
    cancellable: false,
    input: None,
    bound: None,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `run.start`.
pub const RUN_START: Operation = Operation {
    name: "run.start",
    description: "Starts a Run its program drives, and answers once it is started. A Run ID names the Run for good: a start with an existing Run ID returns that Run as it is when the caller and the inputs match, and fails with `run_conflict` otherwise.",
    class: OperationClass::IdempotentWrite,
    kind: OperationKind::Opener,
    reach: Reach::AllSurfaces,
    params: schema_of::<RunStartParams>,
    result: schema_of::<RunStarted>,
    events: &[],
    cancellable: false,
    input: None,
    bound: None,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `run.finish`.
pub const RUN_FINISH: Operation = Operation {
    name: "run.finish",
    description: "Lets a Run finish and answers its Run report: a Run its program drives ends at once, `finished` and `complete`; a crawl ends once its work items are done, and a keep-alive crawl (`crawl.keep_alive`), which waits for new work while nothing is left, ends once this call lets it and its work items are done. A Run something stopped first answers its report saying `stopped`. A Run that ended `failed` fails the call with the error that failed it, and a write at its end that failed, such as its Plan file's, with that write's error. Cancelling the call, or its `timeout` passing, which fails it with `deadline_exceeded`, ends only the wait.",
    class: OperationClass::IdempotentWrite,
    kind: OperationKind::RoundTrip,
    reach: Reach::AllSurfaces,
    params: schema_of::<RunWaitParams>,
    result: schema_of::<RunReport>,
    events: &[],
    cancellable: true,
    input: None,
    bound: None,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `run.stop`.
pub const RUN_STOP: Operation = Operation {
    name: "run.stop",
    description: "Stops a Run at its next safe point with a Checkpoint, `stopped` and `incomplete` as an abort by its program, resumable by its Run ID, and answers its Run report; an ended Run's report is answered as it is. The wait is bounded by `timeout.close`, or the call's `timeout`: past it, the call fails with `deadline_exceeded` and the Run still stops at its next safe point. A Run that ended `failed` fails the call as `run.finish` does.",
    class: OperationClass::IdempotentWrite,
    kind: OperationKind::RoundTrip,
    reach: Reach::AllSurfaces,
    params: schema_of::<RunWaitParams>,
    result: schema_of::<RunReport>,
    events: &[],
    cancellable: true,
    input: None,
    bound: None,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `run.usage`.
pub const RUN_USAGE: Operation = Operation {
    name: "run.usage",
    description: "Answers what a Run used, as the Usage ledger charged it: its model calls and tokens, browser time, proxy bytes, requests, paid solves and Paid fetches, each Limit that applies to it with what is left of it, and whether it is uncapped. An ended Run answers its report's Usage.",
    class: OperationClass::Read,
    kind: OperationKind::RoundTrip,
    reach: Reach::AllSurfaces,
    params: schema_of::<RunParams>,
    result: schema_of::<RunUsage>,
    events: &[],
    cancellable: false,
    input: None,
    bound: None,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `run.abort`.
pub const RUN_ABORT: Operation = Operation {
    name: "run.abort",
    description: "Stops a Run at its next safe point, `stopped` and `incomplete`: no attempt in flight commits, and its work items return to pending. Answers its Run report; an ended Run's report is answered as it is. A Run whose engine died is stopped by this engine, which takes its writer role at the next run epoch; a Run another engine writes is stopped by that engine, the owner the Run index names.",
    class: OperationClass::IdempotentWrite,
    kind: OperationKind::RoundTrip,
    reach: Reach::AllSurfaces,
    params: schema_of::<RunAbortParams>,
    result: schema_of::<RunReport>,
    events: &[],
    cancellable: false,
    input: None,
    bound: None,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `run.discard`.
pub const RUN_DISCARD: Operation = Operation {
    name: "run.discard",
    description: "Discards a Run that ended or was interrupted, for good, so that nothing of it is left behind (ADRs 0001, 0002, 0015 and 0036): its wrapped data key is overwritten and its run folder removed, with its run file, page bodies, browser profiles and the Checkpoints written there; in the per-install layout its rows are deleted from the install database with `secure_delete` on, then `wal_checkpoint(TRUNCATE)`, so neither the database nor its `-wal` and `-shm` files hold its bytes. Its Run index row stays, marked `erased`, without its tags or status line. A Profile directory it used in place and a Borrowed browser are the person's own and are never touched (ADR 0056), nor are Checkpoints written elsewhere. A later `run.start` under its Run ID starts a new Run; discarding a Run already discarded changes nothing. A Run this engine runs fails with `run_live`, one whose writer lock another engine holds with `run_in_use`, and a Run ID no Run has with `run_not_found`.",
    class: OperationClass::IdempotentWrite,
    kind: OperationKind::RoundTrip,
    reach: Reach::SdkOnly,
    params: schema_of::<RunParams>,
    result: schema_of::<RunDiscarded>,
    events: &[],
    cancellable: false,
    input: None,
    bound: None,
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

/// New work for a Run that ended (ADR 0102).
pub const RUN_ENDED: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "run_ended",
    description: "The Run ended `finished`, so it takes no new work, such as a page or a Session it would open: start a Run under a new Run ID. The details name the Run.",
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

/// A Run whose writer's lock stayed held while the engine retried it (ADRs
/// 0031 and 0037).
pub const RUN_BUSY: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "run_busy",
    description: "Another engine holds the Run's writer lock, and the owner the Run index names did not say it took the action: it did not answer, and may still be taking it, or does not own the Run. The engine retried the lock 5 times over about a second; the writer may end, so try again later.",
    group: ErrorGroup::Refused,
    retryable: true,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// A forwarded action that reached an engine that does not own the Run
/// (ADR 0031).
pub const RUN_NOT_OWNED: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "run_not_owned",
    description: "The action was forwarded to this engine as the Run's owner, and this engine does not own the Run: the Run index named an engine that has since ended or restarted. The engine that forwarded it retries the Run's writer lock.",
    group: ErrorGroup::Refused,
    retryable: false,
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

/// A discard of a Run its engine still runs (ADRs 0001 and 0036).
pub const RUN_LIVE: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "run_live",
    description: "The engine still runs the Run, so it cannot be discarded: stop it first with `run.stop` or `run.abort`, or let it finish, then discard it. The details name the Run.",
    group: ErrorGroup::Program,
    retryable: false,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// A Run its engine's resource governor stopped, or refused at its start,
/// for want of memory or storage (ADRs 0040 and 0122).
pub const RESOURCES_EXHAUSTED: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "resources_exhausted",
    description: "The engine ran out of memory or storage for the Run: it checkpointed and stopped it, or refused to start it where its run file would go. The details name the `resource`. Resume it once memory or space frees up, on an engine with a larger allowance, or with lower ceilings.",
    group: ErrorGroup::Unavailable,
    retryable: false,
    exit_code: Some(15),
    arrived_in: ARRIVED_IN,
};

/// What `resources_exhausted` details (ADRs 0040 and 0122).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ResourcesExhaustedDetails {
    /// The resource that ran out.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub resource: Resource,
    /// The Run, when one was stopped or refused.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub run_id: Option<RunId>,
    /// For storage, the folder on the full filesystem; none where the
    /// engine has no folder of its own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub path: Option<String>,
    /// For storage, the space left on that filesystem, in bytes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub free_bytes: Option<u64>,
    /// For storage, the setting that set the level, with its value and
    /// layer (ADR 0079).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub limit: Option<SettingLimit>,
    /// What to do: the folder to free, or the setting to lower, then a
    /// start with the Run's ID.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub fix: String,
}

/// What `run_not_found`, `run_in_use`, `run_busy`, `run_not_owned`, `run_live` and
/// `run_state_unavailable` detail.
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
    fixed_for_run: false,
    unbuilt: None,
    arrived_in: ARRIVED_IN,
};

fn per_run_layout() -> serde_json::Value {
    serde_json::Value::String(LAYOUT_PER_RUN.to_owned())
}

/// `run_state.layout`'s default: one SQLite file per Run (ADR 0002).
pub const LAYOUT_PER_RUN: &str = "per_run";
/// `run_state.layout`'s operator choice: one database of every Run of the
/// install and major, written by the Daemon (ADR 0002).
pub const LAYOUT_PER_INSTALL: &str = "per_install";

/// `run_state.layout` (ADR 0002).
pub const RUN_STATE_LAYOUT: Setting = Setting {
    name: "run_state.layout",
    description: "Where Runs keep their run state: `per_run`, the default, one SQLite file in each run folder; `per_install`, one database, `run-state.sqlite` in the per-major folder, holding every Run of the install and major and written by the Daemon alone, to which an Attached engine then hands its Runs. Export, import and Checkpoint give the same per-run file in both. Taken only from the machine's own layers, and read when the Daemon starts.",
    kind: ValueKind::Choice(&[LAYOUT_PER_RUN, LAYOUT_PER_INSTALL]),
    default: per_run_layout,
    clearable: false,
    authority: Authority::Machine,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
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
    /// costly control, as a JSON list of each one's key, value, layer and
    /// origin (ADR 0028).
    pub weakened: String,
    /// How many commits the run file holds: the position of its journal,
    /// which a Checkpoint reports (ADR 0002).
    pub journal_position: u64,
    /// What the Run is bound to (ADR 0056), as JSON, or none.
    pub binding: Option<String>,
    /// The digest of the Clean-up rules the Run holds to, which a resume
    /// keeps (ADR 0047), or none.
    pub clean_up_digest: Option<String>,
    /// The program's own status line, once it set one (RT-15).
    pub progress_message: Option<String>,
    /// Why the Run did nothing, as JSON, once its driver found it so (ADR
    /// 0102).
    pub nothing_done: Option<String>,
    /// Whether it spent money no Limit caps (ADR 0041).
    pub uncapped: bool,
    /// Its Run report, as JSON, once it ended (ADR 0102).
    pub report: Option<String>,
}

/// Whether the state a Run index row points at is there (ADRs 0031 and
/// 0036).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Presence {
    /// Its state is at its location.
    Present,
    /// `runs reindex` found no state at its location; the row is kept.
    Missing,
    /// The Run was discarded on purpose; `runs reindex` keeps its row
    /// erased.
    Erased,
}

impl Presence {
    /// Every value, in declaration order.
    pub const ALL: [Self; 3] = [Self::Present, Self::Missing, Self::Erased];

    /// Its name in the Run index.
    pub fn as_str(self) -> &'static str {
        crate::wire_name::of(&self)
    }
}

/// One row of the Run index (ADR 0031): a Run, where its state is, its three
/// axes and, while it is live, the engine that owns it.
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
    /// While it is live, where the engine that owns it is reached: a
    /// Daemon's endpoint, or an Attached engine's private endpoint. An
    /// engine that cannot take the Run's writer lock forwards live actions
    /// there.
    pub owner: Option<RunOwner>,
    /// Whether its state is at its location.
    pub presence: Presence,
    /// The program's own status line, once it set one (RT-15).
    pub progress_message: Option<String>,
    /// Why it stopped, once it stopped: "Needs you" lists a Run stopped by
    /// storage from it (ADR 0122).
    pub stop_reason: Option<StopReason>,
    /// Whether it replayed a Response recording (ADR 0105), which marks
    /// its row so that no one reads it as a live run.
    pub replayed: bool,
}

/// Where the engine that owns a live Run is reached (ADR 0031).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RunOwner {
    /// A Daemon, at the endpoint of its runtime file, which also holds its
    /// token.
    Daemon(DaemonEndpoint),
    /// An Attached engine, at its private endpoint.
    Attached(PrivateEndpoint),
}

impl fmt::Display for RunOwner {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Daemon(endpoint) => write!(formatter, "the Daemon at {endpoint}"),
            Self::Attached(private) => {
                write!(formatter, "the Attached engine at {}", private.address)
            }
        }
    }
}

/// What `runs reindex --json` prints: the Run index it rebuilt, and how many
/// of its rows are of each presence.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RunsReindexOutput {
    /// The Run index's path.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub run_index: String,
    /// Rows of Runs whose state it found.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub present: u64,
    /// Rows it kept whose state it did not find.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub missing: u64,
    /// Rows of Runs discarded on purpose, kept erased.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub erased: u64,
}

/// Every entry this module defines.
pub const ENTRIES: Entries = Entries {
    defined_in: module_path!(),
    operations: &[
        RUN_START,
        RUN_FINISH,
        RUN_STOP,
        RUN_ABORT,
        RUN_USAGE,
        RUN_SET_PROGRESS_MESSAGE,
        RUN_DISCARD,
        RUN_PROGRESS_READ,
        RUN_REPORT,
        RUN_LIST,
    ],
    events: &[RUN_AXES_CHANGED, RUN_PROGRESS],
    handler_requests: &[],
    error_codes: &[
        RUN_ID_INVALID,
        RUN_CONFLICT,
        RUN_NOT_FOUND,
        RUN_ENDED,
        RUN_IN_USE,
        RUN_BUSY,
        RUN_NOT_OWNED,
        RUN_STATE_UNAVAILABLE,
        RUN_LIVE,
        RESOURCES_EXHAUSTED,
        RUN_INTERRUPTED,
    ],
    plugin_kinds: &[],
    metrics: &[],
    settings: &[
        RUN_STATE_LOCATION,
        RUN_STATE_DURABILITY,
        RUN_STATE_COMMIT_INTERVAL,
        RUN_STATE_BATCH_PER_ATTEMPT,
        RUN_STATE_BATCH_PAGES,
        RUN_STATE_BATCH_INTERVAL,
        RUN_STATE_BATCH_BYTES,
        RUN_STATE_BATCH_SYNC,
        RUN_STATE_MEMORY_BUDGET,
        RUN_STATE_LAYOUT,
    ],
    hook_stages: &[],
    shared_type_values: &warning::WARNING_CODES,
};

#[cfg(test)]
mod tests;
