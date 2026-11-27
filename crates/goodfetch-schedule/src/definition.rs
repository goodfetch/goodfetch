//! Validating and normalizing a [`ScheduleDefinition`] (ADR 0036; spec #589):
//! the one check `create`, `edit` and `apply` share.
//!
//! It validates the trigger ([`crate::trigger`]), resolves the explicit IANA
//! time zone (never "local"), validates the Schedule's own `schedule.*`
//! settings against the machine's Operator policy and user configuration
//! layers (refusing non-`schedule.*` keys and restricted layers with
//! `setting_not_applicable`, and enforcing Operator policy locks and caps),
//! and enforces the three Operator policy caps (`schedule.max_count`,
//! `schedule.min_interval` and `schedule.limits_max`) with
//! `schedule_cap_exceeded`.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use goodfetch_engine::settings::{Layers, MachineLayers, Resolved, SettingsError, Values, resolve};
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::run::RunLimit;
use goodfetch_protocol::schedule::{
    SCHEDULE_AUTO_PAUSE_FAILURES, SCHEDULE_CATCH_UP, SCHEDULE_JITTER, SCHEDULE_LIMITS_MAX,
    SCHEDULE_MAX_COUNT, SCHEDULE_MIN_INTERVAL, SCHEDULE_OVERLAP, SCHEDULE_PAUSE_TIMEOUT,
    SCHEDULE_RETENTION_CHANGED_WATCHES, SCHEDULE_RETENTION_HISTORY, SCHEDULE_RETENTION_RUNS,
    SCHEDULE_RETENTION_WATCH_RUNS, ScheduleCapExceededDetails, ScheduleCapKind, ScheduleCatchUp,
    ScheduleDefinition, ScheduleId, ScheduleOverlap, SchedulePauseTimeout, SchedulePeriodLimits,
    ScheduleTarget, ScheduleTrigger, ScheduleWarning,
};
use goodfetch_protocol::settings::SettingsLayer;
use jiff::tz::TimeZone;
use serde_json::{Map, Value};

use crate::trigger::{self, TriggerInvalid};

/// Prefix of every setting a Schedule's own `settings` may hold.
const SCHEDULE_SETTING_PREFIX: &str = "schedule.";

/// A Schedule ID for a definition that gives none: `s` and a lowercase
/// ULID of the wall time `wall` and 80 bits of `draw` (ADR 0059), so the
/// same seed makes the same ID.
pub(crate) fn generated_id(wall: SystemTime, mut draw: impl FnMut() -> u64) -> ScheduleId {
    let milliseconds = wall.duration_since(UNIX_EPOCH).map_or(0, |since| {
        u64::try_from(since.as_millis()).unwrap_or(u64::MAX)
    });
    let random = (u128::from(draw()) << 64 | u128::from(draw())) & ((1_u128 << 80) - 1);
    let ulid = ulid::Ulid::from_parts(milliseconds, random);
    ScheduleId::parse(&format!("s{}", ulid.to_string().to_lowercase()))
        .expect("invariant: `s` and a lowercase ULID is a Schedule ID")
}

/// A time zone no IANA name of the zone database names.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("`{0}` names no IANA time zone, such as `Europe/Paris`")]
pub(crate) struct ZoneUnknown(pub(crate) String);

/// The zone a Schedule stores: `given`, when the zone database knows it,
/// else the host's zone `host`.
///
/// # Errors
///
/// [`ZoneUnknown`] for a `given` name the zone database does not know.
pub(crate) fn time_zone(given: Option<&str>, host: &str) -> Result<String, ZoneUnknown> {
    let Some(given) = given else {
        return Ok(host.to_owned());
    };
    let zone = TimeZone::get(given).map_err(|_| ZoneUnknown(given.to_owned()))?;
    Ok(zone.iana_name().unwrap_or(given).to_owned())
}

/// The nine per-Schedule settings resolved across Operator policy, user
/// configuration and the Schedule's own `settings`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ScheduleSettings {
    pub(crate) overlap: ScheduleOverlap,
    pub(crate) catch_up: ScheduleCatchUp,
    pub(crate) jitter_ms: u64,
    pub(crate) auto_pause_failures: u64,
    pub(crate) pause_timeout: Option<SchedulePauseTimeout>,
    pub(crate) retention_runs: u64,
    pub(crate) retention_watch_runs: u64,
    pub(crate) retention_changed_watches: u64,
    pub(crate) retention_history: Duration,
}

impl ScheduleSettings {
    /// Reads the nine per-Schedule settings from `resolved`.
    pub(crate) fn from_resolved(resolved: &Resolved) -> Self {
        let overlap_word: &str = resolved.get(&SCHEDULE_OVERLAP);
        let overlap = ScheduleOverlap::parse(overlap_word)
            .expect("invariant: resolver validated schedule.overlap choice");
        let catch_up = ScheduleCatchUp::read(resolved.value(&SCHEDULE_CATCH_UP))
            .expect("invariant: resolver validated schedule.catch_up shape");
        let jitter: Duration = resolved.get(&SCHEDULE_JITTER);
        let jitter_ms = u64::try_from(jitter.as_millis()).unwrap_or(u64::MAX);
        let auto_pause_failures: u64 = resolved.get(&SCHEDULE_AUTO_PAUSE_FAILURES);
        let pause_timeout_value = resolved.value(&SCHEDULE_PAUSE_TIMEOUT);
        let pause_timeout = if pause_timeout_value.is_null() {
            None
        } else {
            Some(
                SchedulePauseTimeout::read(pause_timeout_value)
                    .expect("invariant: resolver validated schedule.pause_timeout shape"),
            )
        };
        let retention_runs: u64 = resolved.get(&SCHEDULE_RETENTION_RUNS);
        let retention_watch_runs: u64 = resolved.get(&SCHEDULE_RETENTION_WATCH_RUNS);
        let retention_changed_watches: u64 = resolved.get(&SCHEDULE_RETENTION_CHANGED_WATCHES);
        let retention_history: Duration = resolved.get(&SCHEDULE_RETENTION_HISTORY);
        Self {
            overlap,
            catch_up,
            jitter_ms,
            auto_pause_failures,
            pause_timeout,
            retention_runs,
            retention_watch_runs,
            retention_changed_watches,
            retention_history,
        }
    }
}

/// An Operator policy cap refusal (`schedule_cap_exceeded`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CapExceeded {
    pub(crate) message: String,
    pub(crate) details: ScheduleCapExceededDetails,
}

/// Why a [`ScheduleDefinition`] was refused.
#[derive(Debug)]
pub(crate) enum DefinitionError {
    TriggerInvalid(TriggerInvalid),
    ZoneUnknown(ZoneUnknown),
    LimitsInvalid(String),
    Settings(SettingsError),
    CapExceeded(CapExceeded),
}

/// Everything [`check`] reads from its caller, with no I/O and no clock.
#[derive(Clone, Copy)]
pub(crate) struct CheckContext<'a> {
    pub(crate) machine: &'a MachineLayers,
    pub(crate) engine_product: Product,
    pub(crate) host_zone: &'a str,
    /// When `Some(n)`, [`check`] enforces `schedule.max_count` against `n`.
    /// Callers that enforce count atomically inside `store::create` (so an
    /// existing ID reports `schedule_exists` first with no extra read) pass
    /// `None` and pass `checked.max_count` to the store.
    pub(crate) existing_count: Option<u64>,
    pub(crate) wall: SystemTime,
}

/// A validated, normalized Schedule definition ready to store.
#[derive(Debug)]
pub(crate) struct CheckedDefinition {
    pub(crate) schedule_id: ScheduleId,
    pub(crate) target: ScheduleTarget,
    pub(crate) trigger: ScheduleTrigger,
    pub(crate) time_zone: String,
    pub(crate) settings: Option<Map<String, Value>>,
    pub(crate) limits: Option<SchedulePeriodLimits>,
    pub(crate) resolved_settings: ScheduleSettings,
    pub(crate) max_count: Option<u64>,
    pub(crate) policy_version: Option<String>,
    pub(crate) warnings: Vec<ScheduleWarning>,
}

/// Validates and normalizes `definition` against the machine's Operator
/// policy, user configuration and existing Schedule count.
///
/// # Errors
///
/// [`DefinitionError`] on an invalid trigger, unknown time zone, malformed
/// period Limits, refused setting layer/lock/cap, or exceeded Operator
/// policy cap (`schedule_cap_exceeded`).
pub(crate) fn check(
    definition: ScheduleDefinition,
    context: CheckContext<'_>,
    draw_id: impl FnMut() -> u64,
) -> Result<CheckedDefinition, DefinitionError> {
    let parsed_trigger =
        trigger::parse(&definition.trigger).map_err(DefinitionError::TriggerInvalid)?;
    let time_zone = time_zone(definition.time_zone.as_deref(), context.host_zone)
        .map_err(DefinitionError::ZoneUnknown)?;
    if let Some(limits) = &definition.limits {
        limits.check().map_err(DefinitionError::LimitsInvalid)?;
    }

    let schedule_values =
        read_schedule_settings(definition.settings.as_ref(), context.engine_product)
            .map_err(DefinitionError::Settings)?;

    let layers = Layers {
        operator_policy: context.machine.operator_policy.clone(),
        user_configuration: context.machine.user_configuration.clone(),
        run: schedule_values,
        ..Layers::default()
    };
    let resolved = resolve(&layers, context.engine_product, context.engine_product)
        .map_err(DefinitionError::Settings)?;
    let policy_version = resolved.policy_version().map(str::to_owned);

    let resolved_settings = ScheduleSettings::from_resolved(&resolved);

    let max_count: Option<u64> = resolved.get(&SCHEDULE_MAX_COUNT);
    if let (Some(cap), Some(existing_count)) = (max_count, context.existing_count)
        && existing_count >= cap
    {
        return Err(DefinitionError::CapExceeded(count_cap_exceeded(
            existing_count,
            cap,
            policy_version,
        )));
    }

    let min_interval: Option<Duration> = resolved.get(&SCHEDULE_MIN_INTERVAL);
    if let Some(min_duration) = min_interval {
        let min_ms = u64::try_from(min_duration.as_millis()).unwrap_or(u64::MAX);
        if let Some(actual_ms) = trigger::min_interval_ms(&parsed_trigger)
            && actual_ms < min_ms
        {
            return Err(DefinitionError::CapExceeded(CapExceeded {
                message: format!(
                    "The Schedule's interval ({actual_ms} ms) is shorter than Operator policy's `{}` ({min_ms} ms).",
                    SCHEDULE_MIN_INTERVAL.name
                ),
                details: ScheduleCapExceededDetails {
                    cap: ScheduleCapKind::MinInterval,
                    policy: SCHEDULE_MIN_INTERVAL.name.to_owned(),
                    policy_version,
                },
            }));
        }
    }

    let limits_max_value = resolved.value(&SCHEDULE_LIMITS_MAX);
    if !limits_max_value.is_null() {
        let limits_max = SchedulePeriodLimits::read(limits_max_value)
            .expect("invariant: resolver validated schedule.limits_max shape");
        check_period_limits_cap(
            definition.limits.as_ref(),
            &limits_max,
            policy_version.as_deref(),
        )?;
    }

    let schedule_id = definition
        .id
        .unwrap_or_else(|| generated_id(context.wall, draw_id));
    let settings = definition.settings.filter(|map| !map.is_empty());
    let limits = definition
        .limits
        .filter(|period_limits| !period_limits.is_empty());

    Ok(CheckedDefinition {
        schedule_id,
        target: definition.target,
        trigger: definition.trigger,
        time_zone,
        settings,
        limits,
        resolved_settings,
        max_count,
        policy_version,
        warnings: Vec::new(),
    })
}

/// Builds the `schedule_cap_exceeded` refusal for `schedule.max_count`,
/// shared between [`check`] and the store's atomic write-transaction check.
pub(crate) fn count_cap_exceeded(
    existing_count: u64,
    cap: u64,
    policy_version: Option<String>,
) -> CapExceeded {
    CapExceeded {
        message: format!(
            "The Daemon already holds {existing_count} Schedule(s), reaching Operator policy's `{}` cap of {cap}.",
            SCHEDULE_MAX_COUNT.name
        ),
        details: ScheduleCapExceededDetails {
            cap: ScheduleCapKind::Count,
            policy: SCHEDULE_MAX_COUNT.name.to_owned(),
            policy_version,
        },
    }
}

/// Reads a Schedule's own `settings` map at the run layer, refusing any
/// key outside the `schedule.*` namespace with `setting_not_applicable`.
fn read_schedule_settings(
    given: Option<&Map<String, Value>>,
    engine_product: Product,
) -> Result<Values, SettingsError> {
    let Some(map) = given else {
        return Ok(Values::default());
    };
    let values = Values::read(SettingsLayer::Run, map, engine_product)?;
    for key in values.keys() {
        if !key.starts_with(SCHEDULE_SETTING_PREFIX) {
            return Err(SettingsError::NotApplicable {
                key: (*key).to_owned(),
                layer: SettingsLayer::Run,
                fix: format!(
                    "`{key}` is not a `schedule.*` setting; set a run's settings in `target.settings` instead"
                ),
            });
        }
    }
    Ok(values)
}

/// Verifies that every period Limit capped by Operator policy's
/// `schedule.limits_max` is declared on the Schedule at or below the cap.
fn check_period_limits_cap(
    given: Option<&SchedulePeriodLimits>,
    cap: &SchedulePeriodLimits,
    policy_version: Option<&str>,
) -> Result<(), DefinitionError> {
    let periods = [
        (
            "day",
            cap.day.as_deref(),
            given.and_then(|l| l.day.as_deref()),
        ),
        (
            "week",
            cap.week.as_deref(),
            given.and_then(|l| l.week.as_deref()),
        ),
        (
            "month",
            cap.month.as_deref(),
            given.and_then(|l| l.month.as_deref()),
        ),
    ];

    for (period, capped_list, given_list) in periods {
        let Some(capped) = capped_list else {
            continue;
        };
        for max_limit in capped {
            let matching = given_list.and_then(|list| find_limit(list, max_limit));
            let message = match matching {
                None => Some(format!(
                    "Operator policy's `{}` requires a `{period}` period Limit for {} of at most {}; none was given.",
                    SCHEDULE_LIMITS_MAX.name,
                    limit_label(max_limit),
                    max_limit.amount,
                )),
                Some(actual) if actual.amount > max_limit.amount => Some(format!(
                    "The Schedule's `{period}` period Limit for {} ({}) exceeds Operator policy's `{}` cap ({}).",
                    limit_label(max_limit),
                    actual.amount,
                    SCHEDULE_LIMITS_MAX.name,
                    max_limit.amount,
                )),
                Some(_) => None,
            };
            if let Some(message) = message {
                return Err(DefinitionError::CapExceeded(CapExceeded {
                    message,
                    details: ScheduleCapExceededDetails {
                        cap: ScheduleCapKind::PeriodLimit,
                        policy: SCHEDULE_LIMITS_MAX.name.to_owned(),
                        policy_version: policy_version.map(str::to_owned),
                    },
                }));
            }
        }
    }
    Ok(())
}

fn find_limit<'a>(list: &'a [RunLimit], target: &RunLimit) -> Option<&'a RunLimit> {
    list.iter()
        .find(|entry| entry.usage_kind == target.usage_kind && entry.currency == target.currency)
}

fn limit_label(limit: &RunLimit) -> String {
    match &limit.currency {
        Some(currency) => format!("`{}` ({currency})", limit.usage_kind),
        None => format!("`{}`", limit.usage_kind),
    }
}

#[cfg(test)]
mod tests;
