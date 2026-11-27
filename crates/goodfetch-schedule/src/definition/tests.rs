#![expect(
    clippy::doc_markdown,
    reason = "Covers: declarations carry raw Feature IDs (ADR 0072)"
)]

use std::num::NonZeroU64;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use goodfetch_engine::settings::{HostDefaults, MachineLayers, Policy, SettingsError, Values};
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::run::RunLimit;
use goodfetch_protocol::schedule::{
    CronTrigger, EveryTrigger, RecipeTarget, SCHEDULE_AUTO_PAUSE_FAILURES, SCHEDULE_JITTER,
    SCHEDULE_LIMITS_MAX, SCHEDULE_MAX_COUNT, SCHEDULE_MIN_INTERVAL, SCHEDULE_OVERLAP,
    SCHEDULE_RETENTION_RUNS, SCHEDULE_TICK, SCHEDULE_WAKE_THRESHOLD, ScheduleCapExceededDetails,
    ScheduleCapKind, ScheduleCatchUp, ScheduleDefinition, ScheduleOverlap, SchedulePauseTimeout,
    SchedulePeriodLimits, ScheduleTarget, ScheduleTrigger,
};
use goodfetch_protocol::settings::SettingsLayer;
use rstest::rstest;
use serde_json::{Map, Value, json};

use super::{CheckContext, DefinitionError, ZoneUnknown, check, generated_id, time_zone};

fn wall() -> SystemTime {
    UNIX_EPOCH + Duration::from_millis(1_764_061_234_567)
}

fn counter() -> impl FnMut() -> u64 {
    let mut next = 0x0123_4567_89ab_cdef_u64;
    move || {
        next = next.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
        next
    }
}

fn map_of(value: Value) -> Map<String, Value> {
    let Value::Object(map) = value else {
        panic!("expected JSON object, got {value}");
    };
    map
}

fn base_definition(trigger: ScheduleTrigger) -> ScheduleDefinition {
    ScheduleDefinition {
        id: None,
        target: ScheduleTarget::Recipe(RecipeTarget {
            recipe: "./prices.recipe.ts".to_owned(),
            inputs: None,
            settings: None,
            timeout: None,
        }),
        trigger,
        time_zone: Some("Europe/Paris".to_owned()),
        settings: None,
        limits: None,
    }
}

fn machine_with(
    policy_settings: Option<Value>,
    policy_lock: Option<Value>,
    policy_cap: Option<Value>,
    policy_version: Option<&str>,
    user_settings: Option<Value>,
) -> MachineLayers {
    let operator_policy = Policy {
        settings: policy_settings
            .map(|v| {
                Values::read(SettingsLayer::OperatorPolicy, &map_of(v), Product::Collie).unwrap()
            })
            .unwrap_or_default(),
        lock: policy_lock
            .map(|v| {
                Values::read(SettingsLayer::OperatorPolicy, &map_of(v), Product::Collie).unwrap()
            })
            .unwrap_or_default(),
        cap: policy_cap
            .map(|v| {
                Values::read(SettingsLayer::OperatorPolicy, &map_of(v), Product::Collie).unwrap()
            })
            .unwrap_or_default(),
        version: policy_version.map(str::to_owned),
        managed: None,
    };
    let user_configuration = user_settings
        .map(|v| {
            Values::read(
                SettingsLayer::UserConfiguration,
                &map_of(v),
                Product::Collie,
            )
            .unwrap()
        })
        .unwrap_or_default();
    MachineLayers {
        operator_policy,
        user_configuration,
        files: Vec::new(),
        version_defaults: HostDefaults::default(),
    }
}

fn run_check(
    definition: ScheduleDefinition,
    machine: &MachineLayers,
    existing_count: u64,
) -> Result<super::CheckedDefinition, DefinitionError> {
    check(
        definition,
        CheckContext {
            machine,
            engine_product: Product::Collie,
            host_zone: "UTC",
            existing_count: Some(existing_count),
            wall: wall(),
        },
        counter(),
    )
}

/// Covers: story:10-7
#[test]
fn a_generated_id_is_s_and_a_lowercase_ulid_of_the_wall_time() {
    let id = generated_id(wall(), counter());

    assert_eq!(id.as_str().len(), 27);
    assert!(id.as_str().starts_with('s'));
    let ulid = ulid::Ulid::from_string(&id.as_str()[1..].to_uppercase()).unwrap();
    assert_eq!(ulid.timestamp_ms(), 1_764_061_234_567);
}

/// Covers: story:10-7
#[test]
fn the_same_draws_generate_the_same_id() {
    assert_eq!(
        generated_id(wall(), counter()),
        generated_id(wall(), counter())
    );
}

/// Covers: story:10-8
#[rstest]
#[case::the_hosts_when_none_is_given(None, "Asia/Kathmandu")]
#[case::the_given_one(Some("Europe/Paris"), "Europe/Paris")]
#[case::utc_given(Some("UTC"), "UTC")]
fn a_schedule_stores_an_explicit_zone(#[case] given: Option<&str>, #[case] stored: &str) {
    assert_eq!(time_zone(given, "Asia/Kathmandu"), Ok(stored.to_owned()));
}

/// Covers: story:10-8
#[rstest]
#[case::local("local")]
#[case::a_posix_rule("CET-1CEST")]
#[case::unknown("Europe/Atlantis")]
fn a_name_the_zone_database_does_not_know_is_refused(#[case] given: &str) {
    assert_eq!(
        time_zone(Some(given), "UTC"),
        Err(ZoneUnknown(given.to_owned()))
    );
}

/// Every Operator policy cap (`schedule.max_count`, `schedule.min_interval`,
/// `schedule.limits_max`) refuses a violating Schedule with
/// `schedule_cap_exceeded` naming the cap, the policy key and policy
/// version.
///
/// Covers: story:10-10, error:schedule_cap_exceeded, setting:schedule.max_count, setting:schedule.min_interval, setting:schedule.limits_max
#[rstest]
#[case::count_at_cap(
    json!({ "schedule": { "max_count": 2 } }),
    ScheduleTrigger::Every(EveryTrigger { every: 3_600_000, anchor: None }),
    None,
    2,
    ScheduleCapKind::Count,
    SCHEDULE_MAX_COUNT.name,
)]
#[case::count_above_cap(
    json!({ "schedule": { "max_count": 0 } }),
    ScheduleTrigger::Every(EveryTrigger { every: 3_600_000, anchor: None }),
    None,
    0,
    ScheduleCapKind::Count,
    SCHEDULE_MAX_COUNT.name,
)]
#[case::min_interval_every(
    json!({ "schedule": { "min_interval": 300_000 } }),
    ScheduleTrigger::Every(EveryTrigger { every: 60_000, anchor: None }),
    None,
    0,
    ScheduleCapKind::MinInterval,
    SCHEDULE_MIN_INTERVAL.name,
)]
#[case::min_interval_cron(
    json!({ "schedule": { "min_interval": 600_000 } }),
    ScheduleTrigger::Cron(CronTrigger { cron: "*/5 * * * *".to_owned() }),
    None,
    0,
    ScheduleCapKind::MinInterval,
    SCHEDULE_MIN_INTERVAL.name,
)]
#[case::min_interval_cron_rare_date(
    json!({ "schedule": { "min_interval": 300_000 } }),
    ScheduleTrigger::Cron(CronTrigger { cron: "0,1 0 29 2 *".to_owned() }),
    None,
    0,
    ScheduleCapKind::MinInterval,
    SCHEDULE_MIN_INTERVAL.name,
)]
#[case::period_limit_day_exceeded(
    json!({ "schedule": { "limits_max": { "day": [{ "usage_kind": "pages", "amount": 100 }] } } }),
    ScheduleTrigger::Every(EveryTrigger { every: 3_600_000, anchor: None }),
    Some(SchedulePeriodLimits {
        day: Some(vec![RunLimit { usage_kind: "pages".to_owned(), currency: None, amount: 101 }]),
        week: None,
        month: None,
    }),
    0,
    ScheduleCapKind::PeriodLimit,
    SCHEDULE_LIMITS_MAX.name,
)]
#[case::period_limit_missing_when_capped(
    json!({ "schedule": { "limits_max": { "day": [{ "usage_kind": "pages", "amount": 100 }] } } }),
    ScheduleTrigger::Every(EveryTrigger { every: 3_600_000, anchor: None }),
    None,
    0,
    ScheduleCapKind::PeriodLimit,
    SCHEDULE_LIMITS_MAX.name,
)]
#[case::period_limit_week_currency_exceeded(
    json!({ "schedule": { "limits_max": { "week": [{ "usage_kind": "ai_tokens", "currency": "USD", "amount": 5_000_000 }] } } }),
    ScheduleTrigger::Every(EveryTrigger { every: 3_600_000, anchor: None }),
    Some(SchedulePeriodLimits {
        day: None,
        week: Some(vec![RunLimit { usage_kind: "ai_tokens".to_owned(), currency: Some("USD".to_owned()), amount: 6_000_000 }]),
        month: None,
    }),
    0,
    ScheduleCapKind::PeriodLimit,
    SCHEDULE_LIMITS_MAX.name,
)]
#[case::period_limit_month_exceeded(
    json!({ "schedule": { "limits_max": { "month": [{ "usage_kind": "pages", "amount": 1_000 }] } } }),
    ScheduleTrigger::Every(EveryTrigger { every: 3_600_000, anchor: None }),
    Some(SchedulePeriodLimits {
        day: None,
        week: None,
        month: Some(vec![RunLimit { usage_kind: "pages".to_owned(), currency: None, amount: 2_000 }]),
    }),
    0,
    ScheduleCapKind::PeriodLimit,
    SCHEDULE_LIMITS_MAX.name,
)]
fn operator_policy_caps_refuse_violating_schedules(
    #[case] policy: Value,
    #[case] trigger: ScheduleTrigger,
    #[case] limits: Option<SchedulePeriodLimits>,
    #[case] existing_count: u64,
    #[case] expected_cap: ScheduleCapKind,
    #[case] expected_policy: &str,
) {
    let machine = machine_with(Some(policy), None, None, Some("2026.11"), None);
    let mut def = base_definition(trigger);
    def.limits = limits;

    let error = run_check(def, &machine, existing_count).expect_err("cap must refuse");
    match error {
        DefinitionError::CapExceeded(refused) => {
            assert_eq!(
                refused.details,
                ScheduleCapExceededDetails {
                    cap: expected_cap,
                    policy: expected_policy.to_owned(),
                    policy_version: Some("2026.11".to_owned()),
                }
            );
            assert!(
                refused.message.contains(expected_policy),
                "message should name policy: {}",
                refused.message
            );
        }
        other => panic!("expected CapExceeded, got {other:?}"),
    }
}

/// A Schedule within all three Operator policy caps is accepted and
/// normalized.
///
/// Covers: story:10-10
#[test]
fn schedule_within_all_operator_policy_caps_is_accepted() {
    let policy = json!({
        "schedule": {
            "max_count": 3,
            "min_interval": 300_000,
            "limits_max": {
                "day": [
                    { "usage_kind": "pages", "amount": 500 },
                    { "usage_kind": "ai_tokens", "currency": "USD", "amount": 5_000_000 }
                ],
                "month": [
                    { "usage_kind": "pages", "amount": 10_000 }
                ]
            }
        }
    });
    let machine = machine_with(Some(policy), None, None, Some("2026.11"), None);
    let mut def = base_definition(ScheduleTrigger::Cron(CronTrigger {
        cron: "*/15 * * * *".to_owned(),
    }));
    def.limits = Some(SchedulePeriodLimits {
        day: Some(vec![
            RunLimit {
                usage_kind: "pages".to_owned(),
                currency: None,
                amount: 500,
            },
            RunLimit {
                usage_kind: "ai_tokens".to_owned(),
                currency: Some("USD".to_owned()),
                amount: 4_000_000,
            },
        ]),
        week: None,
        month: Some(vec![RunLimit {
            usage_kind: "pages".to_owned(),
            currency: None,
            amount: 8_000,
        }]),
    });

    let checked = run_check(def, &machine, 2).expect("within caps");
    assert_eq!(checked.max_count, Some(3));
    assert_eq!(checked.policy_version.as_deref(), Some("2026.11"));
}

/// The three Operator-policy-only keys answer `setting_not_applicable` in
/// user configuration and in a Schedule's `settings`; `schedule.tick` and
/// `schedule.wake_threshold` are refused in a Schedule's `settings` too;
/// non-`schedule.*` keys are refused in a Schedule's `settings`.
///
/// Covers: story:10-10, error:setting_not_applicable
#[rstest]
#[case::max_count_in_user_config(
    Some(json!({ "schedule": { "max_count": 5 } })),
    None,
    SCHEDULE_MAX_COUNT.name,
    SettingsLayer::UserConfiguration,
)]
#[case::min_interval_in_user_config(
    Some(json!({ "schedule": { "min_interval": 60_000 } })),
    None,
    SCHEDULE_MIN_INTERVAL.name,
    SettingsLayer::UserConfiguration,
)]
#[case::limits_max_in_user_config(
    Some(json!({ "schedule": { "limits_max": { "day": [{ "usage_kind": "pages", "amount": 10 }] } } })),
    None,
    SCHEDULE_LIMITS_MAX.name,
    SettingsLayer::UserConfiguration,
)]
#[case::max_count_in_schedule_settings(
    None,
    Some(json!({ "schedule": { "max_count": 5 } })),
    SCHEDULE_MAX_COUNT.name,
    SettingsLayer::Run,
)]
#[case::min_interval_in_schedule_settings(
    None,
    Some(json!({ "schedule": { "min_interval": 60_000 } })),
    SCHEDULE_MIN_INTERVAL.name,
    SettingsLayer::Run,
)]
#[case::limits_max_in_schedule_settings(
    None,
    Some(json!({ "schedule": { "limits_max": { "day": [{ "usage_kind": "pages", "amount": 10 }] } } })),
    SCHEDULE_LIMITS_MAX.name,
    SettingsLayer::Run,
)]
#[case::tick_in_schedule_settings(
    None,
    Some(json!({ "schedule": { "tick": 30_000 } })),
    SCHEDULE_TICK.name,
    SettingsLayer::Run,
)]
#[case::wake_threshold_in_schedule_settings(
    None,
    Some(json!({ "schedule": { "wake_threshold": 60_000 } })),
    SCHEDULE_WAKE_THRESHOLD.name,
    SettingsLayer::Run,
)]
#[case::non_schedule_key_in_schedule_settings(
    None,
    Some(json!({ "timeout": { "navigation": 5_000 } })),
    "timeout.navigation",
    SettingsLayer::Run,
)]
fn restricted_settings_refuse_wrong_layers_with_setting_not_applicable(
    #[case] user_config: Option<Value>,
    #[case] schedule_settings: Option<Value>,
    #[case] expected_key: &str,
    #[case] expected_layer: SettingsLayer,
) {
    let machine = machine_with(None, None, None, None, user_config);
    let mut def = base_definition(ScheduleTrigger::Every(EveryTrigger {
        every: 3_600_000,
        anchor: None,
    }));
    def.settings = schedule_settings.map(map_of);

    let error = run_check(def, &machine, 0).expect_err("wrong layer must be refused");
    match error {
        DefinitionError::Settings(SettingsError::NotApplicable { key, layer, .. }) => {
            assert_eq!(key, expected_key);
            assert_eq!(layer, expected_layer);
        }
        other => panic!("expected Settings(NotApplicable), got {other:?}"),
    }
}

/// A Schedule's own `settings` accept all nine per-Schedule keys, override
/// user configuration, and honour Operator policy locks and caps
/// (`setting_locked`).
///
/// Covers: story:10-10, error:setting_locked
#[rstest]
#[case::locked_overlap(
    Some(json!({ "schedule": { "overlap": "skip" } })),
    None,
    json!({ "schedule": { "overlap": "allow" } }),
    SCHEDULE_OVERLAP.name,
)]
#[case::capped_jitter(
    None,
    Some(json!({ "schedule": { "jitter": 10_000 } })),
    json!({ "schedule": { "jitter": 60_000 } }),
    SCHEDULE_JITTER.name,
)]
#[case::capped_retention_runs(
    None,
    Some(json!({ "schedule": { "retention": { "runs": 5 } } })),
    json!({ "schedule": { "retention": { "runs": 20 } } }),
    SCHEDULE_RETENTION_RUNS.name,
)]
#[case::capped_auto_pause_failures(
    None,
    Some(json!({ "schedule": { "auto_pause": { "failures": 5 } } })),
    json!({ "schedule": { "auto_pause": { "failures": 10 } } }),
    SCHEDULE_AUTO_PAUSE_FAILURES.name,
)]
fn operator_policy_lock_and_cap_refuse_schedule_settings(
    #[case] lock: Option<Value>,
    #[case] cap: Option<Value>,
    #[case] schedule_settings: Value,
    #[case] expected_key: &str,
) {
    let machine = machine_with(None, lock, cap, Some("2026.11"), None);
    let mut def = base_definition(ScheduleTrigger::Every(EveryTrigger {
        every: 3_600_000,
        anchor: None,
    }));
    def.settings = Some(map_of(schedule_settings));

    let error = run_check(def, &machine, 0).expect_err("lock/cap must refuse");
    match error {
        DefinitionError::Settings(SettingsError::Locked {
            key, layer, limit, ..
        }) => {
            assert_eq!(key, expected_key);
            assert_eq!(layer, SettingsLayer::Run);
            assert_eq!(limit.layer, SettingsLayer::OperatorPolicy);
        }
        other => panic!("expected Settings(Locked), got {other:?}"),
    }
}

/// All nine per-Schedule settings resolve cleanly from a Schedule's own
/// `settings`, while machine timing keys (`schedule.tick`,
/// `schedule.wake_threshold`) are accepted in user configuration.
///
/// Covers: story:10-10, setting:schedule.overlap, setting:schedule.catch_up, setting:schedule.jitter, setting:schedule.auto_pause.failures, setting:schedule.pause_timeout, setting:schedule.retention.runs, setting:schedule.retention.watch_runs, setting:schedule.retention.changed_watches, setting:schedule.retention.history, setting:schedule.tick, setting:schedule.wake_threshold
#[test]
fn schedule_own_settings_resolve_all_nine_keys() {
    let user = json!({
        "schedule": {
            "overlap": "queue_one",
            "tick": 30_000,
            "wake_threshold": 90_000
        }
    });
    let schedule_settings = json!({
        "schedule": {
            "overlap": "replace",
            "catch_up": 600_000,
            "jitter": 15_000,
            "auto_pause": { "failures": 5 },
            "pause_timeout": 3_600_000,
            "retention": {
                "runs": 25,
                "watch_runs": 12,
                "changed_watches": 8,
                "history": 86_400_000
            }
        }
    });
    let machine = machine_with(None, None, None, None, Some(user));
    let mut def = base_definition(ScheduleTrigger::Every(EveryTrigger {
        every: 3_600_000,
        anchor: None,
    }));
    def.settings = Some(map_of(schedule_settings));

    let checked = run_check(def, &machine, 0).expect("valid schedule settings");
    assert_eq!(checked.resolved_settings.overlap, ScheduleOverlap::Replace);
    assert_eq!(
        checked.resolved_settings.catch_up,
        ScheduleCatchUp::Duration(600_000)
    );
    assert_eq!(checked.resolved_settings.jitter_ms, 15_000);
    assert_eq!(checked.resolved_settings.auto_pause_failures, 5);
    assert_eq!(
        checked.resolved_settings.pause_timeout,
        Some(SchedulePauseTimeout::Duration(
            NonZeroU64::new(3_600_000).unwrap()
        ))
    );
    assert_eq!(checked.resolved_settings.retention_runs, 25);
    assert_eq!(checked.resolved_settings.retention_watch_runs, 12);
    assert_eq!(checked.resolved_settings.retention_changed_watches, 8);
    assert_eq!(
        checked.resolved_settings.retention_history,
        Duration::from_hours(24)
    );
}
