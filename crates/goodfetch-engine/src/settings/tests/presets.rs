//! A Durability level unfolds into its preset's keys at the layer that set
//! it, and a lock on the level locks them (ADR 0142, spec #2138).

use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::settings::{SettingPreset, SettingsLayer, WeakenedSetting};
use rstest::rstest;
use serde_json::{Value, json};

use super::{catalog_key, nested, only, policy, refusal, resolved, values};
use crate::settings::{Layers, SettingsError, Values};

const DURABILITY: &str = "run_state.durability";
const SYNC: &str = "run_state.batch.sync";

fn preset(level: &str) -> SettingPreset {
    SettingPreset {
        key: DURABILITY.to_owned(),
        value: json!(level),
    }
}

/// Each key's value and source as the resolver gives them.
fn sources(
    layers: &Layers,
    keys: &[&str],
) -> Vec<(String, Value, SettingsLayer, Option<SettingPreset>)> {
    let effective = resolved(layers);
    keys.iter()
        .map(|key| {
            let one = effective.effective(catalog_key(key)).unwrap();
            (
                (*key).to_owned(),
                one.value.clone(),
                one.layer,
                one.preset.clone(),
            )
        })
        .collect()
}

/// Covers: setting:run_state.durability, setting:run_state.batch.per_attempt,
/// setting:run_state.batch.pages, setting:run_state.batch.interval,
/// setting:run_state.batch.bytes, setting:run_state.batch.sync
#[test]
fn a_run_that_sets_nothing_unfolds_the_batch_preset() {
    let unfolded = sources(
        &Layers::default(),
        &[
            DURABILITY,
            "run_state.batch.per_attempt",
            "run_state.batch.pages",
            "run_state.batch.interval",
            "run_state.batch.bytes",
            SYNC,
        ],
    );

    let default = SettingsLayer::Default;
    assert_eq!(
        unfolded,
        [
            (DURABILITY.to_owned(), json!("batch"), default, None),
            (
                "run_state.batch.per_attempt".to_owned(),
                json!(false),
                default,
                Some(preset("batch"))
            ),
            (
                "run_state.batch.pages".to_owned(),
                json!(1_000),
                default,
                Some(preset("batch"))
            ),
            (
                "run_state.batch.interval".to_owned(),
                json!(1_000),
                default,
                Some(preset("batch"))
            ),
            (
                "run_state.batch.bytes".to_owned(),
                json!(8_388_608),
                default,
                Some(preset("batch"))
            ),
            (
                SYNC.to_owned(),
                json!("each"),
                default,
                Some(preset("batch"))
            ),
        ]
    );
}

/// Covers: setting:run_state.durability, setting:run_state.commit_interval
#[test]
fn a_level_set_in_user_configuration_unfolds_there() {
    let unfolded = sources(
        &only(
            SettingsLayer::UserConfiguration,
            DURABILITY,
            &json!("periodic"),
        ),
        &[
            "run_state.batch.per_attempt",
            SYNC,
            "run_state.commit_interval",
        ],
    );

    let user = SettingsLayer::UserConfiguration;
    assert_eq!(
        unfolded,
        [
            (
                "run_state.batch.per_attempt".to_owned(),
                json!(true),
                user,
                Some(preset("periodic"))
            ),
            (
                SYNC.to_owned(),
                json!("interval"),
                user,
                Some(preset("periodic"))
            ),
            (
                "run_state.commit_interval".to_owned(),
                json!(5_000),
                user,
                Some(preset("periodic"))
            ),
        ]
    );
}

/// A key set at the level's layer or a higher one overrides its preset; a
/// key set at a lower layer gives way to it.
///
/// Covers: setting:run_state.durability, setting:run_state.batch.sync
#[rstest]
#[case::at_a_higher_layer(SettingsLayer::Run, json!("never"), SettingsLayer::Run, None)]
#[case::at_the_same_layer(SettingsLayer::ProjectConfiguration, json!("never"), SettingsLayer::ProjectConfiguration, None)]
#[case::at_a_lower_layer(SettingsLayer::UserConfiguration, json!("off"), SettingsLayer::ProjectConfiguration, Some(preset("off")))]
fn a_key_overrides_the_preset_of_a_level_set_at_its_layer_or_below(
    #[case] sync_layer: SettingsLayer,
    #[case] value: Value,
    #[case] layer: SettingsLayer,
    #[case] source: Option<SettingPreset>,
) {
    let mut layers = only(
        SettingsLayer::ProjectConfiguration,
        DURABILITY,
        &json!("off"),
    );
    let sync = values(sync_layer, json!({ SYNC: "never" }));
    match sync_layer {
        SettingsLayer::UserConfiguration => layers.user_configuration = sync,
        SettingsLayer::ProjectConfiguration => {
            layers.project_configuration =
                values(sync_layer, json!({ DURABILITY: "off", SYNC: "never" }));
        }
        SettingsLayer::Run => layers.run = sync,
        _ => unreachable!("the cases set sync in these layers"),
    }

    let unfolded = sources(&layers, &[SYNC]);

    assert_eq!(unfolded, [(SYNC.to_owned(), value, layer, source)]);
}

/// Covers: setting:run_state.durability, setting:run_state.batch.sync
#[test]
fn a_policy_lock_on_the_level_refuses_a_runs_key_of_its_preset() {
    let mut layers = Layers {
        operator_policy: policy(json!({}), json!({ DURABILITY: "full" }), json!({})),
        ..Layers::default()
    };
    layers.run = values(SettingsLayer::Run, json!({ SYNC: "off" }));

    let refused = refusal(&layers);

    assert!(
        matches!(
            &refused,
            SettingsError::Locked { key, layer: SettingsLayer::Run, limit, .. }
                if key == SYNC
                    && limit.layer == SettingsLayer::OperatorPolicy
                    && limit.value == json!("each")
        ),
        "{refused:?}"
    );
}

/// Covers: setting:run_state.durability, setting:run_state.batch.sync
#[test]
fn a_policy_lock_on_the_level_locks_each_key_of_its_preset() {
    let layers = Layers {
        operator_policy: policy(json!({}), json!({ DURABILITY: "full" }), json!({})),
        ..Layers::default()
    };

    let effective = resolved(&layers);

    let sync = effective.effective(catalog_key(SYNC)).unwrap();
    let pages = effective
        .effective(catalog_key("run_state.batch.pages"))
        .unwrap();
    assert_eq!(
        (sync.locked_by, pages.locked_by),
        (Some(SettingsLayer::OperatorPolicy), None),
        "full's preset leaves `pages` unused, so its lock leaves it free"
    );
}

/// Covers: setting:run_state.batch.pages, setting:run_state.batch.interval,
/// setting:run_state.batch.bytes
#[rstest]
#[case::pages("run_state.batch.pages")]
#[case::interval("run_state.batch.interval")]
#[case::bytes("run_state.batch.bytes")]
fn a_zero_threshold_is_invalid_and_none_is_taken(#[case] key: &str) {
    let zero = Values::read(
        SettingsLayer::Run,
        &nested(json!({ key: 0 })),
        Product::Foxhound,
    );
    let none = sources(&only(SettingsLayer::Run, key, &json!("none")), &[key]);

    assert!(
        matches!(&zero, Err(SettingsError::Invalid { key: refused, .. }) if refused == key),
        "{zero:?}"
    );
    assert_eq!(
        none,
        [(key.to_owned(), json!("none"), SettingsLayer::Run, None)]
    );
}

fn weakened(layers: &Layers) -> Vec<(String, Value)> {
    resolved(layers)
        .weakened()
        .into_iter()
        .map(|WeakenedSetting { key, value, .. }| (key, value))
        .collect()
}

/// Covers: setting:run_state.durability, setting:run_state.batch.sync
#[rstest]
#[case::batch("batch", &[])]
#[case::full("full", &[])]
#[case::fast("fast", &[(SYNC, "never")])]
#[case::periodic("periodic", &[(SYNC, "interval")])]
#[case::off("off", &[(SYNC, "off"), (DURABILITY, "off")])]
fn weakened_lists_off_and_a_sync_other_than_each(
    #[case] level: &str,
    #[case] listed: &[(&str, &str)],
) {
    let listed: Vec<(String, Value)> = listed
        .iter()
        .map(|(key, value)| ((*key).to_owned(), json!(value)))
        .collect();

    assert_eq!(
        weakened(&only(SettingsLayer::Run, DURABILITY, &json!(level))),
        listed
    );
}

/// Covers: setting:run_state.batch.pages, setting:run_state.batch.interval,
/// setting:run_state.batch.bytes
#[rstest]
#[case::pages_above("run_state.batch.pages", json!(1_001), vec![("run_state.batch.pages".to_owned(), json!(1_001))])]
#[case::pages_none("run_state.batch.pages", json!("none"), vec![("run_state.batch.pages".to_owned(), json!("none"))])]
#[case::pages_below("run_state.batch.pages", json!(10), vec![])]
#[case::interval_above("run_state.batch.interval", json!(2_000), vec![("run_state.batch.interval".to_owned(), json!(2_000))])]
#[case::interval_at("run_state.batch.interval", json!(1_000), vec![])]
#[case::bytes_none("run_state.batch.bytes", json!("none"), vec![("run_state.batch.bytes".to_owned(), json!("none"))])]
#[case::bytes_below("run_state.batch.bytes", json!(1_024), vec![])]
fn weakened_lists_a_threshold_above_its_default_or_none(
    #[case] key: &str,
    #[case] value: Value,
    #[case] expected: Vec<(String, Value)>,
) {
    assert_eq!(weakened(&only(SettingsLayer::Run, key, &value)), expected);
}

/// The source `weakened` and `config explain` give a key the preset set.
///
/// Covers: setting:run_state.durability, setting:run_state.batch.sync
#[test]
fn weakened_and_explain_name_the_preset_as_the_source() {
    let effective = resolved(&only(
        SettingsLayer::UserConfiguration,
        DURABILITY,
        &json!("fast"),
    ));

    let weakened = effective.weakened();
    let explained = effective.explain(&[SYNC.to_owned()], Vec::new());

    assert_eq!(
        weakened,
        [WeakenedSetting {
            key: SYNC.to_owned(),
            value: json!("never"),
            layer: SettingsLayer::UserConfiguration,
            origin: None,
            preset: Some(preset("fast")),
        }]
    );
    assert_eq!(
        explained
            .settings
            .iter()
            .map(|setting| (setting.key.as_str(), setting.preset.clone()))
            .collect::<Vec<_>>(),
        [(SYNC, Some(preset("fast")))]
    );
}

const BUDGET: &str = "run_state.memory.budget";
const THRESHOLDS: [&str; 3] = [
    "run_state.batch.pages",
    "run_state.batch.interval",
    "run_state.batch.bytes",
];

/// The Run's layer with `budget` and each threshold in `none` set to `none`.
fn budget_and_none(budget: Option<u64>, none: &[&str]) -> Layers {
    let mut set = serde_json::Map::new();
    if let Some(budget) = budget {
        set.insert(BUDGET.to_owned(), json!(budget));
    }
    for key in none {
        set.insert((*key).to_owned(), json!("none"));
    }
    Layers {
        run: values(SettingsLayer::Run, Value::Object(set)),
        ..Layers::default()
    }
}

/// Nothing would ever leave memory: no batch is sealed and nothing written
/// may stay (spec #2138, G-11 Q9).
///
/// Covers: setting:run_state.memory.budget, setting:run_state.batch.pages,
/// setting:run_state.batch.interval, setting:run_state.batch.bytes
#[test]
fn a_zero_budget_with_every_threshold_none_is_refused_naming_the_four() {
    let refused = refusal(&budget_and_none(Some(0), &THRESHOLDS));

    let SettingsError::Invalid { key, layer, reason } = &refused else {
        panic!("{refused:?}");
    };
    assert_eq!(
        (key.as_str(), *layer, refused.code()),
        (BUDGET, Some(SettingsLayer::Run), "setting_invalid")
    );
    for threshold in THRESHOLDS {
        assert!(reason.contains(threshold), "{reason}");
    }
}

/// Covers: setting:run_state.memory.budget, setting:run_state.batch.pages,
/// setting:run_state.batch.interval, setting:run_state.batch.bytes
#[rstest]
#[case::zero_budget_alone(Some(0), &[])]
#[case::thresholds_none_alone(None, &THRESHOLDS)]
#[case::one_threshold_left(Some(0), &THRESHOLDS[..2])]
#[case::a_budget_of_one_byte(Some(1), &THRESHOLDS)]
fn a_zero_budget_or_every_threshold_none_alone_is_taken(
    #[case] budget: Option<u64>,
    #[case] none: &[&str],
) {
    let effective = resolved(&budget_and_none(budget, none));

    assert_eq!(
        effective.effective(catalog_key(BUDGET)).unwrap().value,
        json!(budget.unwrap_or(64 * 1024 * 1024))
    );
}
