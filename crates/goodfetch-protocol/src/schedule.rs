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
    /// At a fixed interval.
    Every(EveryTrigger),
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

/// A trigger that does not parse.
pub const SCHEDULE_TRIGGER_INVALID: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "schedule_trigger_invalid",
    description: "The Schedule's trigger does not parse, such as an `every` of 0. The details give the `expression`, the `position` where reading stopped and the `reason`.",
    group: ErrorGroup::Program,
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
        SCHEDULE_INFO,
        SCHEDULE_DELETE,
    ],
    events: &[],
    handler_requests: &[],
    error_codes: &[
        SCHEDULE_NOT_FOUND,
        SCHEDULE_EXISTS,
        SCHEDULE_TRIGGER_INVALID,
    ],
    plugin_kinds: &[],
    metrics: &[],
    settings: &[],
    hook_stages: &[],
    shared_type_values: &[],
};

#[cfg(test)]
mod tests;
