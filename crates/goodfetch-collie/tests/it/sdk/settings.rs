//! A scope's settings in code through the Rust SDK (ADR 0083), the
//! `settings-three-states` client conformance case in `linked` mode: the
//! connection's own settings reach the engine in `hello`, a Run's own sit
//! over them in three states, a key left out inheriting, `None` clearing
//! and a value setting, and `explain_settings()` on the connection and on
//! the Run answers what each runs with, async and through `blocking`; a
//! key the Operator policy locked refuses the connection itself. It plays
//! the `settings-explained-with-their-source` case in `linked` mode: each
//! key's origin, the Durability level whose preset gave it and the layer
//! that locked it through a lock on that level. It also plays the
//! `settings-refusals-say-how-to-fix` case in `linked` mode: a
//! configuration-only key set in user configuration is explained from that
//! file, and a Run setting a locked key is refused naming the layer that
//! locked it. The typed setting constants leave a configuration-only or
//! misspelled key out at compile time, so the case's other refusals have no
//! Rust form. TypeScript's `settings.test.ts` and Python's
//! `test_settings_attached.py` are its twins, played there by the Scripted
//! engine.

use std::time::Duration;

use goodfetch_collie::settings::{Settings, checkpoint, timeout};
use goodfetch_collie::{Collie, ErrorCode, ExplainedSetting, SettingsExplanation, blocking};
use goodfetch_conformance::{ClientCaseRun, RustMode};
use goodfetch_test_client::HomeLayout;
use goodfetch_test_support::{TestEnv, case_logs, real_time_runtime};
use serde_json::{Value, json};

use super::options;

/// The keys both explanations ask for, by their dotted names.
const KEYS: [&str; 3] = ["checkpoint.interval", "timeout.navigation", "timeout.wait"];

/// The connection's own settings.
fn connection_settings() -> Settings {
    Settings::new()
        .set(timeout::NAVIGATION, Duration::from_secs(1))
        .set(timeout::WAIT, Duration::from_secs(3))
        .set(checkpoint::INTERVAL, Duration::from_secs(60))
}

/// The Run's own settings over the connection's: `timeout.navigation` left
/// out, `checkpoint.interval` cleared and `timeout.wait` set.
fn run_settings() -> Settings {
    Settings::new()
        .set(timeout::WAIT, Duration::from_secs(5))
        .set(checkpoint::INTERVAL, None)
}

/// Each explained setting as its key, value, layer and origin.
fn keyed(explanation: &SettingsExplanation) -> Vec<Value> {
    explanation
        .settings
        .iter()
        .map(
            |ExplainedSetting {
                 key,
                 value,
                 layer,
                 origin,
                 ..
             }| { json!([key, value, layer, origin]) },
        )
        .collect()
}

/// What the connection and the Run explain: the connection's settings as
/// its `hello` carried them, then the Run's three states over them.
fn expected() -> (Vec<Value>, Vec<Value>) {
    (
        vec![
            json!(["checkpoint.interval", 60_000, "run", "settings"]),
            json!(["timeout.navigation", 1_000, "run", "settings"]),
            json!(["timeout.wait", 3_000, "run", "settings"]),
        ],
        vec![
            json!(["checkpoint.interval", null, "run", "settings"]),
            json!(["timeout.navigation", 1_000, "run", "settings"]),
            json!(["timeout.wait", 5_000, "run", "settings"]),
        ],
    )
}

/// Covers: story:11-70, story:11-72
#[test]
fn a_runs_own_settings_inherit_clear_and_set_over_its_connections() {
    let _case = ClientCaseRun::start(
        case_logs().as_deref(),
        "settings-three-states",
        RustMode::Linked,
    )
    .unwrap();
    let env = TestEnv::new();
    let explained = real_time_runtime().block_on(async {
        let collie = Collie::connect(options(&env).settings(connection_settings()))
            .await
            .unwrap();

        let connection = collie.explain_settings().keys(KEYS).await.unwrap();
        let run = collie.start().settings(run_settings()).await.unwrap();
        let own = run.explain_settings().keys(KEYS).await.unwrap();
        run.finish().await.unwrap();
        collie.close().await.unwrap();
        (keyed(&connection), keyed(&own))
    });

    assert_eq!(explained, expected());
}

/// A program without async gives and explains the same settings.
/// Covers: story:11-70, story:11-72
#[test]
fn a_program_without_async_explains_its_connections_and_its_runs_settings() {
    let _case = ClientCaseRun::start(
        case_logs().as_deref(),
        "settings-three-states",
        RustMode::Linked,
    )
    .unwrap();
    let env = TestEnv::new();
    let collie = blocking::Collie::connect(options(&env).settings(connection_settings())).unwrap();

    let connection = collie.explain_settings().keys(KEYS).send().unwrap();
    let run = collie.start().settings(run_settings()).send().unwrap();
    let own = run.explain_settings().keys(KEYS).send().unwrap();
    run.finish().send().unwrap();
    collie.close().unwrap();

    assert_eq!((keyed(&connection), keyed(&own)), expected());
}

/// Explaining with no `keys` answers every key, and a closed connection's
/// explanation fails with `closed`.
/// Covers: story:11-72
#[test]
fn explaining_every_key_answers_each_and_a_closed_connection_explains_nothing() {
    let env = TestEnv::new();
    real_time_runtime().block_on(async {
        let collie = Collie::connect(options(&env)).await.unwrap();

        let every = collie.explain_settings().await.unwrap();
        collie.close().await.unwrap();
        let closed = collie.explain_settings().await.unwrap_err();

        assert!(
            every
                .settings
                .iter()
                .any(|setting| setting.key == "timeout.wait"),
            "{every:?}"
        );
        assert_eq!(closed.code(), ErrorCode::CLOSED);
    });
}

/// A connection setting a key the Operator policy locked is refused at
/// `hello`, and `connect` fails with the engine's `setting_locked`.
/// Covers: story:11-70, error:setting_locked
#[test]
fn a_connection_setting_a_locked_key_fails_to_connect_with_setting_locked() {
    let _case = ClientCaseRun::start(
        case_logs().as_deref(),
        "settings-three-states",
        RustMode::Linked,
    )
    .unwrap();
    let env = TestEnv::new();
    let layout = HomeLayout::of(env.home());
    #[expect(
        clippy::disallowed_methods,
        reason = "the test writes the Goodfetch home's Operator policy itself"
    )]
    {
        std::fs::create_dir_all(layout.per_major()).unwrap();
        std::fs::write(
            layout.operator_policy(),
            "version = \"0.1\"\n\n[lock.timeout]\nnavigation = 10000\n",
        )
        .unwrap();
    }

    let refused = real_time_runtime().block_on(async {
        Collie::connect(options(&env).settings(connection_settings()))
            .await
            .unwrap_err()
    });

    assert_eq!(refused.code(), ErrorCode::SETTING_LOCKED, "{refused:?}");
}

/// The keys the `settings-explained-with-their-source` case asks for: a
/// Durability level, a key of its preset and a key set in code.
const SOURCED: [&str; 3] = [
    "run_state.batch.sync",
    "run_state.durability",
    "timeout.wait",
];

/// Each explained setting as its key, value, layer, origin, the preset that
/// gave it and the layer that locked it.
fn sourced(explanation: &SettingsExplanation) -> Vec<Value> {
    explanation
        .settings
        .iter()
        .map(|setting| {
            json!([
                setting.key,
                setting.value,
                setting.layer,
                setting.origin,
                setting.preset,
                setting.locked_by
            ])
        })
        .collect()
}

/// `explain_settings()` on the connection and on the Run reports each key's
/// value, the layer and origin that set it, the Durability level whose
/// preset gave it and the layer that locked it: the Operator policy's lock
/// on `run_state.durability` locks `run_state.batch.sync`, which its preset
/// gives, and a key set in code names its origin, the connection's value
/// under the Run's own.
/// Covers: story:11-74, setting:run_state.durability, setting:run_state.batch.sync
#[test]
fn explain_settings_names_each_keys_layer_origin_preset_and_lock() {
    let _case = ClientCaseRun::start(
        case_logs().as_deref(),
        "settings-explained-with-their-source",
        RustMode::Linked,
    )
    .unwrap();
    let env = TestEnv::new();
    let layout = HomeLayout::of(env.home());
    #[expect(
        clippy::disallowed_methods,
        reason = "the test writes the Goodfetch home's Operator policy itself"
    )]
    {
        std::fs::create_dir_all(layout.per_major()).unwrap();
        std::fs::write(
            layout.operator_policy(),
            "version = \"0.1\"\n\n[lock.run_state]\ndurability = \"full\"\n",
        )
        .unwrap();
    }

    let explained = real_time_runtime().block_on(async {
        let collie = Collie::connect(
            options(&env).settings(Settings::new().set(timeout::WAIT, Duration::from_secs(3))),
        )
        .await
        .unwrap();
        let connection = collie.explain_settings().keys(SOURCED).await.unwrap();
        let run = collie
            .start()
            .settings(Settings::new().set(timeout::WAIT, Duration::from_secs(5)))
            .await
            .unwrap();
        let own = run.explain_settings().keys(SOURCED).await.unwrap();
        run.finish().await.unwrap();
        collie.close().await.unwrap();
        (sourced(&connection), sourced(&own))
    });

    let level = json!({ "key": "run_state.durability", "value": "full" });
    let locked = |wait: u64| {
        vec![
            json!([
                "run_state.batch.sync",
                "each",
                "operator_policy",
                null,
                level,
                "operator_policy"
            ]),
            json!([
                "run_state.durability",
                "full",
                "operator_policy",
                null,
                null,
                "operator_policy"
            ]),
            json!(["timeout.wait", wait, "run", "settings", null, null]),
        ]
    };
    assert_eq!(explained, (locked(3_000), locked(5_000)));
}

/// A configuration-only key set in user configuration is explained from
/// that file, a key the Operator policy locked is explained as locked by
/// it, and a Run setting that key is refused with `setting_locked` in the
/// `refused` group, its `limit` naming the layer that locked it.
/// Covers: story:11-71, story:11-76, error:setting_locked
#[test]
fn a_configuration_only_key_is_explained_from_its_file_and_a_lock_names_its_layer() {
    let _case = ClientCaseRun::start(
        case_logs().as_deref(),
        "settings-refusals-say-how-to-fix",
        RustMode::Linked,
    )
    .unwrap();
    let env = TestEnv::new();
    env.user_configuration("storage.low_free = 1000\n");
    let layout = HomeLayout::of(env.home());
    #[expect(
        clippy::disallowed_methods,
        reason = "the test writes the Goodfetch home's Operator policy itself"
    )]
    std::fs::write(
        layout.operator_policy(),
        "version = \"0.1\"\n\n[lock.timeout]\nnavigation = 10000\n",
    )
    .unwrap();

    let (explained, refused) = real_time_runtime().block_on(async {
        let collie = Collie::connect(options(&env)).await.unwrap();
        let explained = collie
            .explain_settings()
            .keys(["storage.low_free", "timeout.navigation"])
            .await
            .unwrap();
        let refused = collie
            .start()
            .settings(Settings::new().set(timeout::NAVIGATION, Duration::from_secs(1)))
            .await
            .unwrap_err();
        collie.close().await.unwrap();
        (explained, refused)
    });

    assert_eq!(
        (
            explained
                .settings
                .iter()
                .map(|setting| {
                    json!([setting.key, setting.value, setting.layer, setting.locked_by])
                })
                .collect::<Vec<_>>(),
            refused.code(),
            refused.group().as_str(),
            serde_json::to_value(refused.limit()).unwrap(),
        ),
        (
            vec![
                json!(["storage.low_free", 1000, "user_configuration", null]),
                json!([
                    "timeout.navigation",
                    10_000,
                    "operator_policy",
                    "operator_policy"
                ]),
            ],
            ErrorCode::SETTING_LOCKED,
            "refused",
            json!({ "key": "timeout.navigation", "value": 10_000, "layer": "operator_policy" }),
        )
    );
}
