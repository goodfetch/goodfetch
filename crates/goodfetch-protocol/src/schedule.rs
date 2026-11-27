//! Schedules (ADR 0036; spec #589): what starts a Run on its own, kept in
//! the per-user, per-major Schedule store of the Daemon, so every Client
//! finds it again. A Schedule names one target, what it starts, and one
//! trigger, when; each stores an explicit IANA time zone. `schedule.create`
//! records one, `schedule.list` and `schedule.get` read them, and a
//! Schedule's own `schedule.info` and `schedule.delete` act on it.
//!
//! Each field records the protocol minor that added it, as `arrived_in` in
//! its schema (ADR 0096).

use std::borrow::Cow;
use std::fmt;
use std::time::SystemTime;

use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::catalog::{
    Entries, ErrorCodeDefinition, ErrorGroup, Operation, OperationClass, OperationKind,
    ProtocolVersion, Reach, Stability, schema_of,
};
use crate::connection::WaitedFor;
use crate::open_value::UnknownValue;
use crate::point_in_time::PointInTime;

pub mod settings;

pub use settings::{
    CATCH_UP_UNTIL_NEXT, DEFAULT_RETENTION_HISTORY_MS, DEFAULT_TICK_MS, DEFAULT_WAKE_THRESHOLD_MS,
    PAUSE_TIMEOUT_NEXT_OCCURRENCE, SCHEDULE_AUTO_PAUSE_FAILURES, SCHEDULE_CATCH_UP,
    SCHEDULE_CATCH_UP_SHAPE, SCHEDULE_JITTER, SCHEDULE_LIMITS_MAX, SCHEDULE_MAX_COUNT,
    SCHEDULE_MIN_INTERVAL, SCHEDULE_OVERLAP, SCHEDULE_PAUSE_TIMEOUT, SCHEDULE_PAUSE_TIMEOUT_SHAPE,
    SCHEDULE_PERIOD_LIMITS_SHAPE, SCHEDULE_RETENTION_CHANGED_WATCHES, SCHEDULE_RETENTION_HISTORY,
    SCHEDULE_RETENTION_RUNS, SCHEDULE_RETENTION_WATCH_RUNS, SCHEDULE_TICK, SCHEDULE_WAKE_THRESHOLD,
    ScheduleCatchUp, ScheduleOverlap, SchedulePauseTimeout, SchedulePeriodLimits,
};

const ARRIVED_IN: ProtocolVersion = ProtocolVersion { major: 0, minor: 1 };

/// What a `schedule.*` call past its `timeout.wait` waited for: the
/// Schedule store's answer.
pub const SCHEDULE_STORE_ANSWER: WaitedFor = WaitedFor::named("schedule store");

/// The pattern every Schedule ID matches (ADR 0036): lowercase letters,
/// digits and `-`, at most 63, starting and ending with a letter or a digit.
pub const SCHEDULE_ID_PATTERN: &str = "^[a-z0-9]([a-z0-9-]{0,61}[a-z0-9])?$";

/// The name that identifies one Schedule in its store for good.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ScheduleId(String);

/// Why a text is no Schedule ID.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("a Schedule ID matches `{SCHEDULE_ID_PATTERN}`")]
pub struct ScheduleIdInvalid;

impl ScheduleId {
    /// Reads `text` as a Schedule ID.
    pub fn parse(text: &str) -> Result<Self, ScheduleIdInvalid> {
        let bytes = text.as_bytes();
        let alphanumeric = |byte: &u8| byte.is_ascii_lowercase() || byte.is_ascii_digit();
        let well_formed = (1..=63).contains(&bytes.len())
            && bytes.iter().all(|byte| alphanumeric(byte) || *byte == b'-')
            && bytes.first().is_some_and(alphanumeric)
            && bytes.last().is_some_and(alphanumeric);
        if well_formed {
            Ok(Self(text.to_owned()))
        } else {
            Err(ScheduleIdInvalid)
        }
    }

    /// The ID as written.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ScheduleId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Serialize for ScheduleId {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for ScheduleId {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = Cow::<str>::deserialize(deserializer)?;
        Self::parse(&text).map_err(serde::de::Error::custom)
    }
}

impl JsonSchema for ScheduleId {
    fn schema_name() -> Cow<'static, str> {
        "ScheduleId".into()
    }

    fn json_schema(_generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "description": "A Schedule ID (ADR 0036): lowercase letters, digits and `-`, at most 63, starting and ending with a letter or a digit.",
            "type": "string",
            "pattern": SCHEDULE_ID_PATTERN,
        })
    }
}

/// What a Schedule starts: a Recipe with its inputs, or a code-free
/// catalog run by its own start parameters, each with the run's settings
/// and timeout (ADRs 0036 and 0080). The scheduler sets the run's ID,
/// labels and Limits.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum ScheduleTarget {
    /// A Recipe.
    Recipe(RecipeTarget),
    /// A code-free catalog run.
    Run(RunTarget),
}

/// A Recipe a Schedule starts.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RecipeTarget {
    /// The Recipe: published, by its digest, or local, by its path.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub recipe: String,
    /// Its inputs, by name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub inputs: Option<Map<String, Value>>,
    /// The run's settings, by key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub settings: Option<Map<String, Value>>,
    /// The run's `timeout.run`, in milliseconds, at least 1: absent for
    /// the setting's value, `null` for none (ADR 0080).
    #[serde(
        default,
        deserialize_with = "crate::settings::present",
        skip_serializing_if = "Option::is_none"
    )]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub timeout: Option<Option<u64>>,
}

/// A code-free catalog run a Schedule starts.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RunTarget {
    /// The run's own start parameters, as its kind defines them, without
    /// its Run ID, labels and Limits.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub run: Map<String, Value>,
    /// The run's settings, by key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub settings: Option<Map<String, Value>>,
    /// The run's `timeout.run`, in milliseconds, at least 1: absent for
    /// the setting's value, `null` for none (ADR 0080).
    #[serde(
        default,
        deserialize_with = "crate::settings::present",
        skip_serializing_if = "Option::is_none"
    )]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub timeout: Option<Option<u64>>,
}

/// When a Schedule's occurrences fall (ADR 0036).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum ScheduleTrigger {
    /// A cron expression.
    Cron(CronTrigger),
    /// At a fixed interval.
    Every(EveryTrigger),
}

/// A cron expression: five fields (minute, hour, day of month, month, day
/// of week) plus optional seconds first, `L`, `W`, `#` and the
/// `@daily`-style macros (ADR 0036).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CronTrigger {
    /// The cron expression, such as `0 9 * * MON-FRI` or `@daily`.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub cron: String,
}

/// A fixed interval: an occurrence every `every` milliseconds from the
/// anchor.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EveryTrigger {
    /// The interval, in milliseconds, at least 1.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub every: u64,
    /// The instant the intervals count from; the Schedule's creation when
    /// absent.
    #[serde(
        with = "crate::point_in_time::nullable",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    #[schemars(with = "Option<PointInTime>", extend("arrived_in" = "0.1"))]
    pub anchor: Option<SystemTime>,
}

/// How many occurrences `schedule.preview` returns when `count` is absent.
pub const DEFAULT_PREVIEW_COUNT: u32 = 5;

/// The most occurrences one `schedule.preview` call may ask for, bounding
/// per-call work (STYLE.md).
pub const MAX_PREVIEW_COUNT: u32 = 100;

/// The plain-words description of a trigger (ADR 0073): a structured form
/// a Client can render in its own locale, and the engine's English
/// rendering.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TriggerDescription {
    /// The structured form of the trigger, for a Client's own translation.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub structured: StructuredTriggerDescription,
    /// The trigger in plain English, such as `Every 15 minutes` or
    /// `At 09:00, Monday through Friday`.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub english: String,
}

/// The structured form of a trigger's plain-words description, which a
/// Client translates into its own locale (ADR 0073).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum StructuredTriggerDescription {
    /// A fixed-interval trigger: every `every` milliseconds from `anchor`.
    Every {
        /// The interval, in milliseconds.
        #[schemars(extend("arrived_in" = "0.1"))]
        every: u64,
        /// The anchor instant, when the trigger names one.
        #[serde(
            with = "crate::point_in_time::nullable",
            default,
            skip_serializing_if = "Option::is_none"
        )]
        #[schemars(with = "Option<PointInTime>", extend("arrived_in" = "0.1"))]
        anchor: Option<SystemTime>,
    },
    /// A cron expression, split into its normalized fields and DST class.
    Cron {
        /// The expression as given.
        #[schemars(extend("arrived_in" = "0.1"))]
        expression: String,
        /// The seconds field, when the expression named one.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[schemars(extend("arrived_in" = "0.1"))]
        seconds: Option<String>,
        /// The minutes field.
        #[schemars(extend("arrived_in" = "0.1"))]
        minutes: String,
        /// The hours field.
        #[schemars(extend("arrived_in" = "0.1"))]
        hours: String,
        /// The day-of-month field.
        #[schemars(extend("arrived_in" = "0.1"))]
        day_of_month: String,
        /// The month field.
        #[schemars(extend("arrived_in" = "0.1"))]
        month: String,
        /// The day-of-week field.
        #[schemars(extend("arrived_in" = "0.1"))]
        day_of_week: String,
        /// Whether DST treats this expression as fixed-time (runs once
        /// after a spring gap and once in an autumn overlap) rather than
        /// wildcard or step (skips a gap and runs on both passes of an
        /// overlap).
        #[schemars(extend("arrived_in" = "0.1"))]
        fixed_time: bool,
    },
}

/// One previewed Occurrence of a trigger: its UTC instant and its local
/// civil date-time with offset in the preview's IANA time zone.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PreviewOccurrence {
    /// When the Occurrence falls, as a UTC instant.
    #[serde(with = "crate::point_in_time")]
    #[schemars(with = "PointInTime", extend("arrived_in" = "0.1"))]
    pub at: SystemTime,
    /// The same instant written in the preview's IANA time zone with its
    /// offset, such as `2026-03-29T03:00:00+02:00[Europe/Paris]`.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub local: String,
}

/// What `schedule.preview` takes: a trigger, an optional IANA time zone
/// (the Daemon host's when absent), an optional instant after which
/// occurrences are listed (now when absent) and how many to list (5 when
/// absent, at most 100).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SchedulePreviewParams {
    /// The trigger to preview.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub trigger: ScheduleTrigger,
    /// The IANA time zone, such as `Europe/Paris`; the Daemon host's when
    /// absent, never "local".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub time_zone: Option<String>,
    /// The instant strictly after which occurrences are listed; now when
    /// absent.
    #[serde(
        with = "crate::point_in_time::nullable",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    #[schemars(with = "Option<PointInTime>", extend("arrived_in" = "0.1"))]
    pub after: Option<SystemTime>,
    /// How many occurrences to list; 5 when absent, at most 100.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub count: Option<u32>,
    /// How long the call may take, in milliseconds, at least 1: absent for
    /// `timeout.wait`, `null` for none (ADR 0080).
    #[serde(
        default,
        deserialize_with = "crate::settings::present",
        skip_serializing_if = "Option::is_none"
    )]
    #[schemars(extend("arrived_in" = "0.1", "setting" = "timeout.wait"))]
    pub timeout: Option<Option<u64>>,
}

/// What `schedule.preview` answers.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SchedulePreview {
    /// The plain-words description of the trigger.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub description: TriggerDescription,
    /// The IANA time zone used.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub time_zone: String,
    /// The next occurrences strictly after `after` (or now), in order.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub occurrences: Vec<PreviewOccurrence>,
}

/// What a Schedule is, as a program writes it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ScheduleDefinition {
    /// Its ID; one is made when absent: `s` and a lowercase ULID.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub id: Option<ScheduleId>,
    /// What it starts.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub target: ScheduleTarget,
    /// When.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub trigger: ScheduleTrigger,
    /// The IANA time zone its triggers read, such as `Europe/Paris`; the
    /// Daemon host's when absent, never "local".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub time_zone: Option<String>,
    /// The Schedule's own `schedule.*` settings, nested by key, layered
    /// above user configuration and held to Operator policy locks and caps.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub settings: Option<Map<String, Value>>,
    /// The Schedule's period Limits (`day`, `week`, `month`), counted in
    /// its time zone and bounded by Operator policy's `schedule.limits_max`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub limits: Option<SchedulePeriodLimits>,
}

/// Whether a Schedule fires.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[schemars(extend("open" = true), transform = crate::open_value::known_members)]
#[non_exhaustive]
pub enum ScheduleState {
    /// It fires at its occurrences.
    On,
    /// A state this reader does not know, kept as received (ADR 0079).
    #[serde(untagged)]
    #[schemars(skip)]
    Unknown(UnknownValue),
}

impl ScheduleState {
    /// Its name in the catalog and on the wire.
    pub const fn as_str(&self) -> &str {
        match self {
            Self::On => "on",
            Self::Unknown(value) => value.as_str(),
        }
    }
}

/// A Schedule as its store holds it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ScheduleInfo {
    /// Its ID.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub schedule_id: ScheduleId,
    /// What it starts, as given.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub target: ScheduleTarget,
    /// When.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub trigger: ScheduleTrigger,
    /// The IANA time zone its triggers read.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub time_zone: String,
    /// Its definition's version: 1 at creation, one more at each edit.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub version: u64,
    /// Whether it fires.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub state: ScheduleState,
    /// The person who created it, for whom its occurrences fire.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub creator: String,
    /// When it was created.
    #[serde(with = "crate::point_in_time")]
    #[schemars(with = "PointInTime", extend("arrived_in" = "0.1"))]
    pub created_at: SystemTime,
    /// The Schedule's own `schedule.*` settings, when the definition set any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub settings: Option<Map<String, Value>>,
    /// The Schedule's period Limits, when the definition set any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub limits: Option<SchedulePeriodLimits>,
}

/// Something about a created Schedule its program should know, which did
/// not stop its creation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ScheduleWarning {
    /// What it is about, such as `env_unresolved`.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub kind: String,
    /// What it says, in the engine's English.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub message: String,
}

/// What `schedule.create` takes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ScheduleCreateParams {
    /// The Schedule.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub definition: ScheduleDefinition,
    /// How long the call may take, in milliseconds, at least 1: absent for
    /// `timeout.wait`, `null` for none (ADR 0080).
    #[serde(
        default,
        deserialize_with = "crate::settings::present",
        skip_serializing_if = "Option::is_none"
    )]
    #[schemars(extend("arrived_in" = "0.1", "setting" = "timeout.wait"))]
    pub timeout: Option<Option<u64>>,
}

/// What `schedule.create` answers.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ScheduleCreated {
    /// The Schedule's ID, as given or made.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub schedule_id: ScheduleId,
    /// What its program should know.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub warnings: Vec<ScheduleWarning>,
}

/// What `schedule.list` takes.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ScheduleListParams {
    /// How long the call may take, in milliseconds, at least 1: absent for
    /// `timeout.wait`, `null` for none (ADR 0080).
    #[serde(
        default,
        deserialize_with = "crate::settings::present",
        skip_serializing_if = "Option::is_none"
    )]
    #[schemars(extend("arrived_in" = "0.1", "setting" = "timeout.wait"))]
    pub timeout: Option<Option<u64>>,
}

/// What `schedule.list` answers.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ScheduleList {
    /// Every Schedule of the store, by ID.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub schedules: Vec<ScheduleInfo>,
}

/// What `schedule.get` takes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ScheduleGetParams {
    /// The Schedule's ID.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub id: ScheduleId,
    /// How long the call may take, in milliseconds, at least 1: absent for
    /// `timeout.wait`, `null` for none (ADR 0080).
    #[serde(
        default,
        deserialize_with = "crate::settings::present",
        skip_serializing_if = "Option::is_none"
    )]
    #[schemars(extend("arrived_in" = "0.1", "setting" = "timeout.wait"))]
    pub timeout: Option<Option<u64>>,
}

/// What `schedule.info` and `schedule.delete` take: the Schedule.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ScheduleParams {
    /// The Schedule's ID.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub schedule_id: ScheduleId,
    /// How long the call may take, in milliseconds, at least 1: absent for
    /// `timeout.wait`, `null` for none (ADR 0080).
    #[serde(
        default,
        deserialize_with = "crate::settings::present",
        skip_serializing_if = "Option::is_none"
    )]
    #[schemars(extend("arrived_in" = "0.1", "setting" = "timeout.wait"))]
    pub timeout: Option<Option<u64>>,
}

/// What `schedule.delete` answers.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ScheduleDeleted {
    /// Whether it deleted the Schedule: false when the store held none of
    /// that ID, as after a resent delete.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub deleted: bool,
}

/// What `schedule_not_found` details.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ScheduleNotFoundDetails {
    /// The Schedule ID.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub schedule_id: ScheduleId,
}

/// What `schedule_exists` details.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ScheduleExistsDetails {
    /// The Schedule ID.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub schedule_id: ScheduleId,
}

/// Why a trigger does not parse.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[schemars(extend("open" = true), transform = crate::open_value::known_members)]
#[non_exhaustive]
pub enum TriggerInvalidReason {
    /// A field it does not know.
    UnknownField,
    /// A value out of its field's range, such as an interval of 0.
    OutOfRange,
    /// A step that does not divide its range.
    BadStep,
    /// A macro it does not know.
    BadMacro,
    /// Too few or too many fields.
    FieldCount,
    /// Another reason, which the message gives.
    Other,
    /// A reason this reader does not know, kept as received (ADR 0079).
    #[serde(untagged)]
    #[schemars(skip)]
    Unknown(UnknownValue),
}

/// What `schedule_trigger_invalid` details.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ScheduleTriggerInvalidDetails {
    /// The trigger's expression, as given.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub expression: String,
    /// The character of the expression where it stops reading, from 0.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub position: u64,
    /// Why.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub reason: TriggerInvalidReason,
}

/// No Schedule has the ID.
pub const SCHEDULE_NOT_FOUND: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "schedule_not_found",
    description: "No Schedule has this Schedule ID: the Schedule store does not hold it. The details name the ID.",
    group: ErrorGroup::Program,
    retryable: false,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// A Schedule has the ID already.
pub const SCHEDULE_EXISTS: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "schedule_exists",
    description: "A Schedule has this Schedule ID already: create it under another ID, or omit the ID for a new one. The details name the ID.",
    group: ErrorGroup::Program,
    retryable: false,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// Which Operator policy cap a Schedule exceeded (`schedule_cap_exceeded`).
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[schemars(extend("open" = true), transform = crate::open_value::known_members)]
#[non_exhaustive]
pub enum ScheduleCapKind {
    /// `schedule.max_count`: maximum number of Schedules held by the Daemon.
    Count,
    /// `schedule.min_interval`: minimum interval between consecutive occurrences.
    MinInterval,
    /// `schedule.limits_max`: maximum period Limit on a Schedule.
    PeriodLimit,
    /// A cap kind this reader does not know, kept as received (ADR 0079).
    #[serde(untagged)]
    #[schemars(skip)]
    Unknown(UnknownValue),
}

impl ScheduleCapKind {
    /// Its canonical wire name.
    pub const fn as_str(&self) -> &str {
        match self {
            Self::Count => "count",
            Self::MinInterval => "min_interval",
            Self::PeriodLimit => "period_limit",
            Self::Unknown(value) => value.as_str(),
        }
    }
}

/// What `schedule_cap_exceeded` details: which Operator policy cap was
/// exceeded and which Operator policy setting (and version, when present)
/// imposed it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ScheduleCapExceededDetails {
    /// Which cap was exceeded: `count`, `min_interval` or `period_limit`.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub cap: ScheduleCapKind,
    /// The Operator policy key that set the cap (`schedule.max_count`,
    /// `schedule.min_interval` or `schedule.limits_max`).
    #[schemars(extend("arrived_in" = "0.1"))]
    pub policy: String,
    /// The Operator policy's version, when the policy carries one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub policy_version: Option<String>,
}

/// A trigger that does not parse.
pub const SCHEDULE_TRIGGER_INVALID: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "schedule_trigger_invalid",
    description: "The Schedule's trigger does not parse, such as an `every` of 0. The details give the `expression`, the `position` where reading stopped and the `reason`.",
    group: ErrorGroup::Program,
    retryable: false,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// An Operator policy cap on Schedules was exceeded.
pub const SCHEDULE_CAP_EXCEEDED: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "schedule_cap_exceeded",
    description: "A Schedule creation or edit exceeds an Operator policy cap (`schedule.max_count`, `schedule.min_interval` or `schedule.limits_max`). The details name the `cap` and its `policy`.",
    group: ErrorGroup::Refused,
    retryable: false,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// `schedule.create`.
pub const SCHEDULE_CREATE: Operation = Operation {
    name: "schedule.create",
    description: "Records a Schedule in the Schedule store, which the Daemon keeps, and answers its ID, as given or made (`s` and a lowercase ULID), with warnings. Nothing fires yet. A taken ID fails with `schedule_exists`, a trigger that does not parse with `schedule_trigger_invalid`. Its time zone is the Daemon host's when the definition names none. One store transaction: a cancel stops it only before it commits.",
    class: OperationClass::Effect,
    kind: OperationKind::RoundTrip,
    reach: Reach::AllSurfaces,
    params: schema_of::<ScheduleCreateParams>,
    result: schema_of::<ScheduleCreated>,
    events: &[],
    cancellable: false,
    input: None,
    bound: None,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `schedule.list`.
pub const SCHEDULE_LIST: Operation = Operation {
    name: "schedule.list",
    description: "Lists every Schedule of the Schedule store, by ID.",
    class: OperationClass::Read,
    kind: OperationKind::RoundTrip,
    reach: Reach::AllSurfaces,
    params: schema_of::<ScheduleListParams>,
    result: schema_of::<ScheduleList>,
    events: &[],
    cancellable: true,
    input: None,
    bound: None,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `schedule.get`.
pub const SCHEDULE_GET: Operation = Operation {
    name: "schedule.get",
    description: "Reads the Schedule of an ID. An ID the store does not hold fails with `schedule_not_found`.",
    class: OperationClass::Read,
    kind: OperationKind::RoundTrip,
    reach: Reach::AllSurfaces,
    params: schema_of::<ScheduleGetParams>,
    result: schema_of::<ScheduleInfo>,
    events: &[],
    cancellable: true,
    input: None,
    bound: None,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `schedule.preview`.
pub const SCHEDULE_PREVIEW: Operation = Operation {
    name: "schedule.preview",
    description: "Describes a Schedule's trigger in plain words and lists its next Occurrences in a time zone strictly after `after` (now when absent; 5 by default, at most 100), including DST transitions. A trigger that does not parse fails with `schedule_trigger_invalid`.",
    class: OperationClass::Read,
    kind: OperationKind::RoundTrip,
    reach: Reach::AllSurfaces,
    params: schema_of::<SchedulePreviewParams>,
    result: schema_of::<SchedulePreview>,
    events: &[],
    cancellable: true,
    input: None,
    bound: None,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `schedule.info`.
pub const SCHEDULE_INFO: Operation = Operation {
    name: "schedule.info",
    description: "Reads the Schedule as its store holds it now. One the store no longer holds fails with `schedule_not_found`.",
    class: OperationClass::Read,
    kind: OperationKind::RoundTrip,
    reach: Reach::AllSurfaces,
    params: schema_of::<ScheduleParams>,
    result: schema_of::<ScheduleInfo>,
    events: &[],
    cancellable: true,
    input: None,
    bound: None,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `schedule.delete`.
pub const SCHEDULE_DELETE: Operation = Operation {
    name: "schedule.delete",
    description: "Deletes the Schedule from its store, so it never fires again, and answers whether it did: false when the store held none of that ID, as after a resent delete. One store transaction: a cancel stops it only before it commits.",
    class: OperationClass::IdempotentWrite,
    kind: OperationKind::RoundTrip,
    reach: Reach::AllSurfaces,
    params: schema_of::<ScheduleParams>,
    result: schema_of::<ScheduleDeleted>,
    events: &[],
    cancellable: false,
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
        SCHEDULE_CREATE,
        SCHEDULE_LIST,
        SCHEDULE_GET,
        SCHEDULE_PREVIEW,
        SCHEDULE_INFO,
        SCHEDULE_DELETE,
    ],
    events: &[],
    handler_requests: &[],
    error_codes: &[
        SCHEDULE_NOT_FOUND,
        SCHEDULE_EXISTS,
        SCHEDULE_TRIGGER_INVALID,
        SCHEDULE_CAP_EXCEEDED,
    ],
    plugin_kinds: &[],
    metrics: &[],
    settings: settings::ALL,
    hook_stages: &[],
    shared_type_values: &[],
};

#[cfg(test)]
mod tests;
