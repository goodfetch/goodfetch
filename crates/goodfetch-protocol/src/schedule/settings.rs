//! The `schedule.*` settings and their shapes (ADR 0036; spec #589):
//! - Per-schedule settings (`Authority::Every`): `schedule.overlap`,
//!   `schedule.catch_up`, `schedule.jitter`, `schedule.auto_pause.failures`,
//!   `schedule.pause_timeout`, `schedule.retention.runs`,
//!   `schedule.retention.watch_runs`, `schedule.retention.changed_watches`,
//!   `schedule.retention.history`.
//! - Machine-level scheduler timing (`Authority::Machine`, user configuration
//!   and Operator policy only): `schedule.tick`, `schedule.wake_threshold`.
//! - Operator policy caps (`Authority::OperatorPolicyAlone`,
//!   `setting_not_applicable` elsewhere): `schedule.max_count`,
//!   `schedule.min_interval`, `schedule.limits_max`.

use std::collections::BTreeSet;
use std::num::NonZeroU64;

use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::catalog::{
    Authority, ProtocolVersion, Setting, SettingJson, ValueKind, ValueShape, shape_json,
};
use crate::run::RunLimit;

const ARRIVED_IN: ProtocolVersion = ProtocolVersion { major: 0, minor: 1 };

/// The word for `schedule.catch_up`'s default window: catch up the latest
/// missed occurrence until the occurrence after it is due.
pub const CATCH_UP_UNTIL_NEXT: &str = "until_next";

/// The word for `schedule.pause_timeout`'s default on time and Watch
/// triggers: time out a Human pause at the Schedule's next occurrence.
pub const PAUSE_TIMEOUT_NEXT_OCCURRENCE: &str = "next_occurrence";

/// One day in milliseconds, used for `schedule.retention.history` (90 days)
/// and after-a-run `schedule.pause_timeout` (24 hours).
const ONE_DAY_MS: u64 = 86_400_000;

/// Ninety days in milliseconds: `schedule.retention.history`'s default.
pub const DEFAULT_RETENTION_HISTORY_MS: u64 = 90 * ONE_DAY_MS;

/// Sixty seconds in milliseconds: `schedule.tick`'s default.
pub const DEFAULT_TICK_MS: u64 = 60_000;

/// Two minutes in milliseconds: `schedule.wake_threshold`'s default.
pub const DEFAULT_WAKE_THRESHOLD_MS: u64 = 120_000;

/// How a Schedule handles an occurrence due while its previous run is still
/// live (`schedule.overlap`, ADR 0036).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ScheduleOverlap {
    /// Skip the new occurrence (`skipped: overlap`), the default.
    Skip,
    /// Queue at most one waiting occurrence behind the live run.
    QueueOne,
    /// Allow concurrent runs under per-domain admission.
    Allow,
    /// Abort the live run cleanly at its next safe point and start the new one.
    Replace,
}

impl ScheduleOverlap {
    /// Every allowed word of `schedule.overlap`.
    pub const WORDS: &'static [&'static str] = &["skip", "queue_one", "allow", "replace"];

    /// Reads `word` as a [`ScheduleOverlap`].
    pub fn parse(word: &str) -> Option<Self> {
        match word {
            "skip" => Some(Self::Skip),
            "queue_one" => Some(Self::QueueOne),
            "allow" => Some(Self::Allow),
            "replace" => Some(Self::Replace),
            _ => None,
        }
    }

    /// Its canonical wire word.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Skip => "skip",
            Self::QueueOne => "queue_one",
            Self::Allow => "allow",
            Self::Replace => "replace",
        }
    }
}

/// How far back a Schedule catches up its latest missed occurrence after
/// downtime or sleep (`schedule.catch_up`, ADR 0036): `"until_next"` (until
/// the occurrence after it is due), a duration in whole milliseconds, or
/// `0` for never.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScheduleCatchUp {
    /// Catch up until the occurrence after the missed one is due.
    UntilNext,
    /// Catch up within this many milliseconds (`0` means never).
    Duration(u64),
}

impl ScheduleCatchUp {
    /// Reads `value` from JSON / TOML.
    ///
    /// # Errors
    ///
    /// What `value` lacks when it is neither `"until_next"` nor a whole
    /// number of milliseconds from 0.
    pub fn read(value: &Value) -> Result<Self, String> {
        match value {
            Value::String(word) if word == CATCH_UP_UNTIL_NEXT => Ok(Self::UntilNext),
            Value::Number(number) => number.as_u64().map(Self::Duration).ok_or_else(|| {
                format!(
                    "expected `{CATCH_UP_UNTIL_NEXT}` or a whole number of milliseconds from 0, found `{value}`"
                )
            }),
            _ => Err(format!(
                "expected `{CATCH_UP_UNTIL_NEXT}` or a whole number of milliseconds from 0, found `{value}`"
            )),
        }
    }
}

impl Serialize for ScheduleCatchUp {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::UntilNext => serializer.serialize_str(CATCH_UP_UNTIL_NEXT),
            Self::Duration(ms) => serializer.serialize_u64(*ms),
        }
    }
}

impl<'de> Deserialize<'de> for ScheduleCatchUp {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = Value::deserialize(deserializer)?;
        Self::read(&value).map_err(serde::de::Error::custom)
    }
}

impl JsonSchema for ScheduleCatchUp {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "ScheduleCatchUp".into()
    }

    fn json_schema(_generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "oneOf": [
                {
                    "type": "integer",
                    "minimum": 0,
                    "description": "Catch-up window in whole milliseconds; 0 means never catch up.",
                },
                {
                    "const": CATCH_UP_UNTIL_NEXT,
                    "description": "Catch up the latest missed occurrence until the occurrence after it is due.",
                },
            ],
        })
    }
}

impl SettingJson for ScheduleCatchUp {
    fn into_setting_json(self) -> Value {
        shape_json(self)
    }
}

/// The shape of `schedule.catch_up`.
pub const SCHEDULE_CATCH_UP_SHAPE: ValueShape = ValueShape {
    name: "ScheduleCatchUp",
    check: |value| ScheduleCatchUp::read(value).map(drop),
    schema: || {
        json!({
            "oneOf": [
                {
                    "type": "integer",
                    "minimum": 0,
                    "description": "Catch-up window in whole milliseconds; 0 means never catch up.",
                },
                {
                    "const": CATCH_UP_UNTIL_NEXT,
                    "description": "Catch up the latest missed occurrence until the occurrence after it is due.",
                },
            ],
        })
    },
    examples: || vec![json!(CATCH_UP_UNTIL_NEXT), json!(300_000)],
};

/// A scheduled run's Human pause timeout (`schedule.pause_timeout`, ADRs
/// 0036 and 0080): `"next_occurrence"` (until the Schedule's next due
/// occurrence; 24 hours for an after-a-run trigger), a duration of at least
/// 1 millisecond, or `null` (cleared) for no timeout. `0` is refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SchedulePauseTimeout {
    /// Time out at the Schedule's next occurrence (or 24 hours for after-a-run).
    NextOccurrence,
    /// Time out after this many milliseconds (at least 1).
    Duration(NonZeroU64),
}

impl SchedulePauseTimeout {
    /// Reads `value` from JSON / TOML.
    ///
    /// # Errors
    ///
    /// What `value` lacks when it is neither `"next_occurrence"` nor a
    /// whole number of milliseconds of at least 1.
    pub fn read(value: &Value) -> Result<Self, String> {
        match value {
            Value::String(word) if word == PAUSE_TIMEOUT_NEXT_OCCURRENCE => {
                Ok(Self::NextOccurrence)
            }
            Value::Number(number) => match number.as_u64().and_then(NonZeroU64::new) {
                Some(ms) => Ok(Self::Duration(ms)),
                None => Err(format!(
                    "expected `{PAUSE_TIMEOUT_NEXT_OCCURRENCE}` or a duration of at least 1 millisecond (use `null` for no timeout), found `{value}`"
                )),
            },
            _ => Err(format!(
                "expected `{PAUSE_TIMEOUT_NEXT_OCCURRENCE}` or a duration of at least 1 millisecond (use `null` for no timeout), found `{value}`"
            )),
        }
    }
}

impl Serialize for SchedulePauseTimeout {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::NextOccurrence => serializer.serialize_str(PAUSE_TIMEOUT_NEXT_OCCURRENCE),
            Self::Duration(ms) => serializer.serialize_u64(ms.get()),
        }
    }
}

impl<'de> Deserialize<'de> for SchedulePauseTimeout {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = Value::deserialize(deserializer)?;
        Self::read(&value).map_err(serde::de::Error::custom)
    }
}

impl JsonSchema for SchedulePauseTimeout {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "SchedulePauseTimeout".into()
    }

    fn json_schema(_generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "oneOf": [
                {
                    "type": "integer",
                    "minimum": 1,
                    "description": "Human pause timeout in whole milliseconds, at least 1; 0 is refused and `null` clears it.",
                },
                {
                    "const": PAUSE_TIMEOUT_NEXT_OCCURRENCE,
                    "description": "Time out at the Schedule's next due occurrence (24 hours for an after-a-run trigger).",
                },
            ],
        })
    }
}

impl SettingJson for SchedulePauseTimeout {
    fn into_setting_json(self) -> Value {
        shape_json(self)
    }
}

/// The shape of `schedule.pause_timeout`.
pub const SCHEDULE_PAUSE_TIMEOUT_SHAPE: ValueShape = ValueShape {
    name: "SchedulePauseTimeout",
    check: |value| SchedulePauseTimeout::read(value).map(drop),
    schema: || {
        json!({
            "oneOf": [
                {
                    "type": "integer",
                    "minimum": 1,
                    "description": "Human pause timeout in whole milliseconds, at least 1; 0 is refused and `null` clears it.",
                },
                {
                    "const": PAUSE_TIMEOUT_NEXT_OCCURRENCE,
                    "description": "Time out at the Schedule's next due occurrence (24 hours for an after-a-run trigger).",
                },
            ],
        })
    },
    examples: || vec![json!(PAUSE_TIMEOUT_NEXT_OCCURRENCE), json!(ONE_DAY_MS)],
};

/// Period Limits on a Schedule, or Operator policy caps on them
/// (`schedule.limits_max`, ADRs 0036, 0041 and 0080): caps on total Usage
/// across the Schedule's runs per `day`, `week` or `month` in the
/// Schedule's time zone.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SchedulePeriodLimits {
    /// Limits per day, counted in the Schedule's time zone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub day: Option<Vec<RunLimit>>,
    /// Limits per week, counted in the Schedule's time zone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub week: Option<Vec<RunLimit>>,
    /// Limits per month, counted in the Schedule's time zone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub month: Option<Vec<RunLimit>>,
}

impl SchedulePeriodLimits {
    /// Reads and validates a [`SchedulePeriodLimits`] from JSON / TOML.
    ///
    /// # Errors
    ///
    /// What `value` lacks when it does not match the period-limits shape,
    /// contains an empty Usage kind, a malformed ISO 4217 currency code or
    /// duplicate entries for one `(usage_kind, currency)` within a period.
    pub fn read(value: &Value) -> Result<Self, String> {
        let parsed: Self = serde_json::from_value(value.clone())
            .map_err(|error| format!("expected period limits (`day`, `week`, `month`): {error}"))?;
        parsed.check()?;
        Ok(parsed)
    }

    /// Validates every period's list of [`RunLimit`]s.
    ///
    /// # Errors
    ///
    /// Describes the first invalid Usage kind, currency or duplicate entry.
    pub fn check(&self) -> Result<(), String> {
        for (period, list) in [
            ("day", self.day.as_deref()),
            ("week", self.week.as_deref()),
            ("month", self.month.as_deref()),
        ] {
            let Some(limits) = list else {
                continue;
            };
            let mut seen = BTreeSet::new();
            for limit in limits {
                if limit.usage_kind.trim().is_empty() {
                    return Err(format!("`{period}` has a Limit with an empty `usage_kind`"));
                }
                if let Some(currency) = &limit.currency {
                    let valid = currency.len() == 3
                        && currency.bytes().all(|byte| byte.is_ascii_uppercase());
                    if !valid {
                        return Err(format!(
                            "`{period}` has a Limit with invalid currency `{currency}` (expected a 3-letter uppercase ISO 4217 code such as `USD`)"
                        ));
                    }
                }
                let key = (limit.usage_kind.as_str(), limit.currency.as_deref());
                if !seen.insert(key) {
                    return Err(format!(
                        "`{period}` names `{}`{} more than once",
                        limit.usage_kind,
                        limit
                            .currency
                            .as_deref()
                            .map_or_else(String::new, |code| format!(" ({code})"))
                    ));
                }
            }
        }
        Ok(())
    }

    /// Whether no period carries any Limit.
    pub fn is_empty(&self) -> bool {
        self.day.as_deref().is_none_or(<[_]>::is_empty)
            && self.week.as_deref().is_none_or(<[_]>::is_empty)
            && self.month.as_deref().is_none_or(<[_]>::is_empty)
    }
}

/// Inline JSON Schema for one [`RunLimit`] item inside `SCHEDULE_PERIOD_LIMITS_SHAPE`.
fn run_limit_inline_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["usage_kind", "amount"],
        "properties": {
            "usage_kind": {
                "type": "string",
                "minLength": 1,
                "description": "The Usage kind capped.",
            },
            "currency": {
                "type": "string",
                "pattern": "^[A-Z]{3}$",
                "description": "Optional ISO 4217 currency code for a price Limit.",
            },
            "amount": {
                "type": "integer",
                "minimum": 0,
                "description": "Maximum count of the Usage kind, or millionths of the currency.",
            },
        },
    })
}

/// The shape of `schedule.limits_max`.
pub const SCHEDULE_PERIOD_LIMITS_SHAPE: ValueShape = ValueShape {
    name: "SchedulePeriodLimits",
    check: |value| SchedulePeriodLimits::read(value).map(drop),
    schema: || {
        let period = json!({
            "type": "array",
            "items": run_limit_inline_schema(),
        });
        json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "day": period,
                "week": {
                    "type": "array",
                    "items": run_limit_inline_schema(),
                },
                "month": {
                    "type": "array",
                    "items": run_limit_inline_schema(),
                },
            },
        })
    },
    examples: || {
        vec![json!({
            "day": [
                { "usage_kind": "pages", "amount": 500 },
                { "usage_kind": "ai_tokens", "currency": "USD", "amount": 5_000_000 }
            ],
            "week": [
                { "usage_kind": "pages", "amount": 2_500 },
                { "usage_kind": "ai_tokens", "currency": "USD", "amount": 20_000_000 }
            ],
            "month": [
                { "usage_kind": "pages", "amount": 10_000 },
                { "usage_kind": "ai_tokens", "currency": "USD", "amount": 50_000_000 }
            ]
        })]
    },
};

fn default_overlap() -> Value {
    Value::String(ScheduleOverlap::Skip.as_str().to_owned())
}

fn default_catch_up() -> Value {
    Value::String(CATCH_UP_UNTIL_NEXT.to_owned())
}

fn default_zero() -> Value {
    Value::from(0_u64)
}

fn default_auto_pause_failures() -> Value {
    Value::from(3_u64)
}

fn default_pause_timeout() -> Value {
    Value::String(PAUSE_TIMEOUT_NEXT_OCCURRENCE.to_owned())
}

fn default_retention_count() -> Value {
    Value::from(10_u64)
}

fn default_retention_history() -> Value {
    Value::from(DEFAULT_RETENTION_HISTORY_MS)
}

fn default_tick() -> Value {
    Value::from(DEFAULT_TICK_MS)
}

fn default_wake_threshold() -> Value {
    Value::from(DEFAULT_WAKE_THRESHOLD_MS)
}

fn default_null() -> Value {
    Value::Null
}

/// `schedule.overlap` (ADR 0036).
pub const SCHEDULE_OVERLAP: Setting = Setting {
    name: "schedule.overlap",
    description: "How a Schedule handles an occurrence due while its previous run is still live: `skip` (default), `queue_one`, `allow` or `replace`.",
    kind: ValueKind::Choice(ScheduleOverlap::WORDS),
    default: default_overlap,
    clearable: false,
    authority: Authority::Every,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `schedule.catch_up` (ADR 0036).
pub const SCHEDULE_CATCH_UP: Setting = Setting {
    name: "schedule.catch_up",
    description: "How far back a Schedule catches up its latest missed occurrence after downtime or sleep: `until_next` (default, until the occurrence after it is due), a duration in whole milliseconds, or `0` for never.",
    kind: ValueKind::Shape(&SCHEDULE_CATCH_UP_SHAPE),
    default: default_catch_up,
    clearable: false,
    authority: Authority::Every,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `schedule.jitter` (ADR 0036).
pub const SCHEDULE_JITTER: Setting = Setting {
    name: "schedule.jitter",
    description: "Upper bound in milliseconds of the fixed random delay drawn once at Schedule creation and added to every occurrence; `0` by default.",
    kind: ValueKind::Duration,
    default: default_zero,
    clearable: false,
    authority: Authority::Every,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `schedule.auto_pause.failures` (ADR 0036).
pub const SCHEDULE_AUTO_PAUSE_FAILURES: Setting = Setting {
    name: "schedule.auto_pause.failures",
    description: "How many consecutive failed, rejected or failed-to-start occurrences auto-pause a Schedule: `3` by default, `0` for never.",
    kind: ValueKind::Count,
    default: default_auto_pause_failures,
    clearable: false,
    authority: Authority::Every,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `schedule.pause_timeout` (ADRs 0036 and 0080).
pub const SCHEDULE_PAUSE_TIMEOUT: Setting = Setting {
    name: "schedule.pause_timeout",
    description: "A scheduled run's Human pause timeout: `next_occurrence` by default (24 hours for an after-a-run trigger), a duration of at least 1 millisecond, or `null` for none; `0` is refused.",
    kind: ValueKind::Shape(&SCHEDULE_PAUSE_TIMEOUT_SHAPE),
    default: default_pause_timeout,
    clearable: true,
    authority: Authority::Every,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `schedule.retention.runs` (ADR 0036).
pub const SCHEDULE_RETENTION_RUNS: Setting = Setting {
    name: "schedule.retention.runs",
    description: "How many recent target runs a Schedule keeps before retention discards older ones: `10` by default.",
    kind: ValueKind::Count,
    default: default_retention_count,
    clearable: false,
    authority: Authority::Every,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `schedule.retention.watch_runs` (ADR 0036).
pub const SCHEDULE_RETENTION_WATCH_RUNS: Setting = Setting {
    name: "schedule.retention.watch_runs",
    description: "How many recent Watch runs a Schedule keeps before retention discards older ones: `10` by default.",
    kind: ValueKind::Count,
    default: default_retention_count,
    clearable: false,
    authority: Authority::Every,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `schedule.retention.changed_watches` (ADR 0036).
pub const SCHEDULE_RETENTION_CHANGED_WATCHES: Setting = Setting {
    name: "schedule.retention.changed_watches",
    description: "How many recent Watch runs that detected a change a Schedule keeps: `10` by default.",
    kind: ValueKind::Count,
    default: default_retention_count,
    clearable: false,
    authority: Authority::Every,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `schedule.retention.history` (ADR 0036).
pub const SCHEDULE_RETENTION_HISTORY: Setting = Setting {
    name: "schedule.retention.history",
    description: "How long a Schedule's occurrence history is kept before retention drops older records, in milliseconds: 90 days by default.",
    kind: ValueKind::Duration,
    default: default_retention_history,
    clearable: false,
    authority: Authority::Every,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `schedule.tick` (ADR 0036).
pub const SCHEDULE_TICK: Setting = Setting {
    name: "schedule.tick",
    description: "Maximum sleep interval of the Daemon's scheduler loop in milliseconds before waking to re-check due occurrences: 60 s by default. Set only in user configuration or an Operator policy.",
    kind: ValueKind::Duration,
    default: default_tick,
    clearable: false,
    authority: Authority::Machine,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `schedule.wake_threshold` (ADR 0036).
pub const SCHEDULE_WAKE_THRESHOLD: Setting = Setting {
    name: "schedule.wake_threshold",
    description: "Wall-clock advance beyond monotonic time, in milliseconds, that the scheduler treats as a machine wake from sleep: 2 minutes by default. Set only in user configuration or an Operator policy.",
    kind: ValueKind::Duration,
    default: default_wake_threshold,
    clearable: false,
    authority: Authority::Machine,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `schedule.max_count` (ADR 0036).
pub const SCHEDULE_MAX_COUNT: Setting = Setting {
    name: "schedule.max_count",
    description: "Maximum number of Schedules the Daemon may hold; unset by default. Set only in an Operator policy; a creation past it fails with `schedule_cap_exceeded`.",
    kind: ValueKind::Count,
    default: default_null,
    clearable: true,
    authority: Authority::OperatorPolicyAlone,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `schedule.min_interval` (ADR 0036).
pub const SCHEDULE_MIN_INTERVAL: Setting = Setting {
    name: "schedule.min_interval",
    description: "Minimum interval in milliseconds allowed between consecutive occurrences of a Schedule; unset by default. Set only in an Operator policy; a creation below it fails with `schedule_cap_exceeded`.",
    kind: ValueKind::Duration,
    default: default_null,
    clearable: true,
    authority: Authority::OperatorPolicyAlone,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `schedule.limits_max` (ADR 0036).
pub const SCHEDULE_LIMITS_MAX: Setting = Setting {
    name: "schedule.limits_max",
    description: "Operator policy cap on a Schedule's period Limits (`day`, `week`, `month`); unset by default. Set only in an Operator policy; a Schedule whose period Limits exceed it or omit a capped period Limit fails with `schedule_cap_exceeded`.",
    kind: ValueKind::Shape(&SCHEDULE_PERIOD_LIMITS_SHAPE),
    default: default_null,
    clearable: true,
    authority: Authority::OperatorPolicyAlone,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// Every `schedule.*` setting declared by the catalog.
pub const ALL: &[Setting] = &[
    SCHEDULE_OVERLAP,
    SCHEDULE_CATCH_UP,
    SCHEDULE_JITTER,
    SCHEDULE_AUTO_PAUSE_FAILURES,
    SCHEDULE_PAUSE_TIMEOUT,
    SCHEDULE_RETENTION_RUNS,
    SCHEDULE_RETENTION_WATCH_RUNS,
    SCHEDULE_RETENTION_CHANGED_WATCHES,
    SCHEDULE_RETENTION_HISTORY,
    SCHEDULE_TICK,
    SCHEDULE_WAKE_THRESHOLD,
    SCHEDULE_MAX_COUNT,
    SCHEDULE_MIN_INTERVAL,
    SCHEDULE_LIMITS_MAX,
];
