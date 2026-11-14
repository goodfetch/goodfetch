//! The settings flags through Collie's executable (ADRs 0080 and 0083):
//! `--set <key>=<value>` and `--settings <file|->` on every command whose
//! operation takes `settings`, both the run layer's `flag` origin, above the
//! project file; durations as milliseconds or `45s`, `2m`, `1h`; a bad value
//! and a flag on a command that takes no settings are usage errors, exit
//! `2`, before any engine call; and a key only configuration holds is
//! refused with `setting_not_applicable` naming the configuration file.
#![expect(
    clippy::disallowed_methods,
    reason = "the tests write the project's settings file themselves"
)]
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use goodfetch_test_support::{Mode, TestEnv, assert_cli_snapshot, cli};
use rstest::rstest;
use serde_json::{Value, json};

const COLLIE: &str = env!("CARGO_BIN_EXE_goodfetch-collie");

/// Writes a project configuration that sets three timeouts in the test's
/// folder.
fn project(env: &TestEnv) {
    std::fs::write(
        env.folder().join("goodfetch.toml"),
        "version = \"0.1\"\n\n[settings.timeout]\nnavigation = 60000\nfirst_byte = 10000\nrun = 3600000\n",
    )
    .unwrap();
}

/// Covers: story:11-203, command:config.explain
#[rstest]
#[case::attached(Mode::Attached)]
#[case::daemon(Mode::Daemon)]
fn the_settings_flags_are_the_run_layers_flag_origin_above_the_project_file(#[case] mode: Mode) {
    let env = TestEnv::new();
    project(&env);

    let checked = cli(&env, COLLIE)
        .args(["config", "explain", "timeout", "--settings", "-"])
        .args(["--set", "timeout.first_byte=2s", "--json"])
        .stdin(r#"{ "timeout": { "navigation": "45s", "first_byte": 1000 } }"#)
        .mode(mode)
        .json("SettingsExplanation")
        .unwrap();

    let timeouts: Vec<Value> = checked.documents[0]["settings"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|setting| {
            ["timeout.first_byte", "timeout.navigation", "timeout.run"]
                .contains(&setting["key"].as_str().unwrap())
        })
        .map(|setting| {
            json!([
                setting["key"],
                setting["value"],
                setting["layer"],
                setting["origin"]
            ])
        })
        .collect();
    assert_eq!(
        timeouts,
        [
            json!(["timeout.first_byte", 2000, "run", "flag"]),
            json!(["timeout.navigation", 45000, "run", "flag"]),
            json!(["timeout.run", 3_600_000, "project_configuration", null]),
        ],
        "{}",
        checked.output
    );
}

/// Covers: story:11-203, command:config.explain
#[test]
fn config_explain_names_a_flags_origin() {
    let env = TestEnv::new();

    let output = cli(&env, COLLIE)
        .args(["config", "explain", "timeout.navigation"])
        .args(["--set", "timeout.navigation=2m"])
        .run()
        .unwrap();

    assert_cli_snapshot!(output.without_dispatch_records(), @r"
    exit: 0
    ----- stdout -----
    timeout.navigation = 120000  (run from flag)
    ----- stderr -----
    ");
}

/// A key a Durability level's preset gives names that level as its source
/// (spec #2138).
///
/// Covers: command:config.explain, setting:run_state.batch.sync
#[test]
fn config_explain_names_the_preset_a_key_comes_from() {
    let env = TestEnv::new();

    let output = cli(&env, COLLIE)
        .args(["config", "explain", "run_state.batch.sync"])
        .args(["--set", "run_state.durability=fast"])
        .run()
        .unwrap();

    assert_cli_snapshot!(output.without_dispatch_records(), @r#"
    exit: 0
    ----- stdout -----
    run_state.batch.sync = "never"  (run from flag, preset of run_state.durability: fast, weakens a guarantee)
    ----- stderr -----
    "#);
}

/// Covers: story:11-203, command:run.start
#[test]
fn a_bad_duration_is_a_usage_error_before_any_engine_call() {
    let env = TestEnv::new();

    let output = cli(&env, COLLIE)
        .args([
            "run",
            "start",
            "--set",
            "timeout.navigation=45x",
            "--attached",
        ])
        .run()
        .unwrap();

    assert_cli_snapshot!(output, @"
    exit: 2
    ----- stdout -----
    ----- stderr -----
    error: invalid value 'timeout.navigation=45x' for '--set <KEY=VALUE>': `timeout.navigation` is a duration: milliseconds, or a whole number with `ms`, `s`, `m` or `h`, such as `[DURATION]`, `[DURATION]` or `[DURATION]`

    For more information, try '--help'.
    ");
}

/// Covers: story:11-203, command:engine.info
#[test]
fn set_on_a_command_that_takes_no_settings_is_a_usage_error_before_any_engine_call() {
    let env = TestEnv::new();

    let output = cli(&env, COLLIE)
        .args(["engine", "info", "--set", "timeout.navigation=45000"])
        .run()
        .unwrap();

    assert_cli_snapshot!(output, @r"
    exit: 2
    ----- stdout -----
    ----- stderr -----
    error: unexpected argument '--set' found

    Usage: goodfetch-collie engine info [OPTIONS]

    For more information, try '--help'.
    ");
}

/// The run's settings reach the engine, which checks their keys.
///
/// Covers: story:11-203, command:run.start, error:setting_unknown
#[test]
fn run_start_sends_the_settings_flags_as_the_runs_settings() {
    let env = TestEnv::new();

    let checked = cli(&env, COLLIE)
        .args([
            "run",
            "start",
            "--set",
            "timeout.navigation_=45s",
            "--json",
            "--attached",
        ])
        .json("CommandFailed")
        .unwrap();

    assert_eq!(
        (
            checked.output.exit,
            &checked.documents[0]["error"]["code"],
            &checked.documents[0]["error"]["closest"],
        ),
        (
            Some(3),
            &json!("setting_unknown"),
            &json!("timeout.navigation")
        ),
        "{}",
        checked.output
    );
}

/// Covers: story:11-203, command:run.start, error:setting_not_applicable
#[test]
fn a_key_only_configuration_holds_is_not_applicable_and_names_the_configuration_file() {
    let env = TestEnv::new();

    let output = cli(&env, COLLIE)
        .args([
            "run",
            "start",
            "--set",
            "browser_cache.max_unused_age=1h",
            "--attached",
        ])
        .run()
        .unwrap();

    assert_cli_snapshot!(output, @r#"
    exit: 3
    ----- stdout -----
    ----- stderr -----
    [INSTANT] TRACE goodfetch::rpc::dispatch: request answered method="hello"
    [INSTANT] TRACE goodfetch::rpc::dispatch: request failed method="run.start" error_code="setting_not_applicable"
    `browser_cache.max_unused_age` cannot be set in run: set `browser_cache.max_unused_age` in user configuration, `[FOLDER]/home/0.0/configuration.toml`, or an Operator policy, `[FOLDER]/home/0.0/operator-policy.toml`
    code: setting_not_applicable
    retryable: false
    fix: set `browser_cache.max_unused_age` in user configuration, `[FOLDER]/home/0.0/configuration.toml`, or an Operator policy, `[FOLDER]/home/0.0/operator-policy.toml`
    "#);
}
