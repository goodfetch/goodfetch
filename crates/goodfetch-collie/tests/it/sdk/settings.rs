//! A scope's settings in code through the Rust SDK (ADR 0083), the
//! `settings-three-states` client conformance case in `linked` mode: the
//! connection's own settings reach the engine in `hello`, a Run's own sit
//! over them in three states, a key left out inheriting, `None` clearing
//! and a value setting, and `explain_settings()` on the connection and on
//! the Run answers what each runs with, async and through `blocking`.
//! TypeScript's `settings.test.ts` and Python's `test_settings_attached.py`
//! are its twins, played there by the Scripted engine.

use std::time::Duration;

use goodfetch_collie::settings::{Settings, checkpoint, timeout};
use goodfetch_collie::{Collie, ExplainedSetting, SettingsExplanation, blocking};
use goodfetch_test_support::{TestEnv, real_time_runtime};
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
    let env = TestEnv::new();
    let collie = blocking::Collie::connect(options(&env).settings(connection_settings())).unwrap();

    let connection = collie.explain_settings().keys(KEYS).send().unwrap();
    let run = collie.start().settings(run_settings()).send().unwrap();
    let own = run.explain_settings().keys(KEYS).send().unwrap();
    run.finish().send().unwrap();
    collie.close().unwrap();

    assert_eq!((keyed(&connection), keyed(&own)), expected());
}

/// Explaining every key leaves `keys` out, and a closed connection's
/// explanation fails with `closed` before anything is sent.
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
        assert_eq!(closed.code().to_string(), "closed");
    });
}
