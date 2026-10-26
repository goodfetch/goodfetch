//! The CLI helper (ADRs 0069, 0072 and 0073): `cli(&env, executable)` fails
//! a run whose piped output holds an escape byte, gives one run's exit code,
//! stdout and stderr as one snapshot, and records each command it ran by its
//! `command:` Feature, or as unmatched, in the test's command record, which
//! teardown copies into the test's exercise record; its locale cases set
//! a locale and time zone of the shared table in the child's map only.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use goodfetch_test_support::{
    COMMAND_RECORD, CliError, RECORDS_VARIABLE, SEED_VARIABLE, SUITE_VARIABLE, Stream, TestEnv,
    Tool, Variables, assert_cli_snapshot, cli, current_test, tools,
};
use rstest::rstest;
use serde_json::{Value, json};

use crate::probe;

/// The test's setup with no exercise record, so the commands these tests run
/// on purpose, unmatched ones among them, never reach the real audit.
pub(crate) fn unrecorded() -> TestEnv {
    let process = Variables::from_process();
    let variables: Variables = [
        SEED_VARIABLE,
        SUITE_VARIABLE,
        "SystemRoot",
        "ComSpec",
        "PATHEXT",
    ]
    .into_iter()
    .filter_map(|name| process.get(name).map(|value| (name, value.to_owned())))
    .collect();
    TestEnv::create(&current_test(), &variables).unwrap()
}

/// The arguments that run one probe of `probe.rs`, this test binary run as
/// a child, as the fixture command.
pub(crate) fn probe_arguments(name: &str) -> [&str; 6] {
    [
        "--exact",
        name,
        "--ignored",
        "--nocapture",
        "--test-threads",
        "1",
    ]
}

/// The CLI helper running one probe of `probe.rs` as the fixture command.
fn probe_cli<'env>(env: &'env TestEnv, name: &str) -> goodfetch_test_support::Cli<'env> {
    cli(env, std::env::current_exe().unwrap()).args(probe_arguments(name))
}

/// The commit's Collie executable, from the test tool manifest.
pub(crate) fn collie() -> PathBuf {
    tools().unwrap().path(Tool::Collie).unwrap().to_owned()
}

/// Every JSON line of the file at `path`.
pub(crate) fn json_lines(path: &Path) -> Vec<Value> {
    std::fs::read_to_string(path)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

#[rstest]
#[case::stdout("probe::prints_an_escape_byte_to_stdout", Stream::Stdout)]
#[case::stderr("probe::prints_an_escape_byte_to_stderr", Stream::Stderr)]
fn an_escape_byte_in_piped_output_fails_the_run(#[case] name: &str, #[case] stream: Stream) {
    let env = unrecorded();

    let run = probe_cli(&env, name).run();

    let error = run.unwrap_err();
    assert!(
        matches!(error, CliError::EscapeByte { stream: held, .. } if held == stream),
        "{error}"
    );
}

#[test]
fn a_clean_run_snapshots_its_exit_code_stdout_and_stderr_as_one_value() {
    let env = unrecorded();

    let output = probe_cli(&env, "probe::prints_to_both_streams_then_exits_with_3")
        .run()
        .unwrap();

    assert_cli_snapshot!(output, @r"
    exit: 3
    ----- stdout -----

    running 1 test
    test probe::prints_to_both_streams_then_exits_with_3 ... out
    ----- stderr -----
    err
    ");
}

#[test]
fn the_command_record_holds_the_matched_command_and_its_exit_code() {
    let env = unrecorded();

    let output = cli(&env, collie())
        .args(["config", "explain", "timeout"])
        .run()
        .unwrap();

    assert_eq!(
        (output.exit, json_lines(&env.folder().join(COMMAND_RECORD))),
        (
            Some(0),
            vec![json!({
                "target": "goodfetch::test::command",
                "fields": { "command": "config.explain", "exit_code": 0 },
            })]
        )
    );
}

#[rstest]
#[case::unknown_command(&["frobnicate", "--now"], "frobnicate --now")]
#[case::command_group_alone(&["browser"], "browser")]
fn arguments_that_match_no_command_feature_are_recorded_as_unmatched(
    #[case] arguments: &[&str],
    #[case] unmatched: &str,
) {
    let env = unrecorded();

    let output = cli(&env, collie()).args(arguments).run().unwrap();

    // 2 is clap's usage error.
    assert_eq!(
        (output.exit, json_lines(&env.folder().join(COMMAND_RECORD))),
        (
            Some(2),
            vec![json!({
                "target": "goodfetch::test::command",
                "fields": { "unmatched": unmatched, "exit_code": 2 },
            })]
        )
    );
}

#[test]
fn teardown_copies_the_command_record_into_the_tests_exercise_record() {
    let env = TestEnv::new();
    let records = env.folder().join("records");
    let mut command = probe(&env, "probe::records_a_command_then_drops_its_test_env");
    command
        .env(RECORDS_VARIABLE, &records)
        .env("NEXTEST_BINARY_ID", "sample::it")
        .env("NEXTEST_TEST_NAME", "installs");

    let status = command.status().unwrap();

    assert!(status.success());
    assert_eq!(
        json_lines(&records.join("sample%3A%3Ait%20installs.jsonl")),
        vec![
            json!({ "test": "sample::it installs" }),
            json!({
                "target": "goodfetch::test::command",
                "fields": { "command": "browser.install", "exit_code": 0 },
            }),
        ]
    );
}

#[test]
fn a_locale_case_sets_lang_and_tz_only_for_the_child_it_runs() {
    let env = unrecorded();
    let own = (std::env::var_os("LANG"), std::env::var_os("TZ"));

    let output = probe_cli(&env, "probe::prints_its_environment")
        .with_locale("tr-TR")
        .with_time_zone("Asia/Kathmandu")
        .run()
        .unwrap();

    let child: BTreeMap<String, String> = output
        .stdout
        .lines()
        .filter_map(|line| line.split_once("env:")?.1.split_once('='))
        .map(|(name, value)| (name.to_owned(), value.to_owned()))
        .collect();
    let mut expected = env.env().clone();
    expected.extend(
        [
            ("NO_COLOR", "1"),
            ("COLUMNS", "100"),
            ("TERM", "dumb"),
            ("LANG", "tr_TR.UTF-8"),
            ("TZ", "Asia/Kathmandu"),
        ]
        .map(|(name, value)| (name.to_owned(), value.to_owned())),
    );
    assert_eq!(
        (
            child,
            env.env()["LANG"].as_str(),
            env.env()["TZ"].as_str(),
            (std::env::var_os("LANG"), std::env::var_os("TZ")),
        ),
        (expected, "C.UTF-8", "UTC", own)
    );
}

#[test]
fn a_variable_the_setup_adds_reaches_the_child_over_the_maps_own() {
    let env = unrecorded().with_variable("TZ", "Europe/Paris");

    let output = probe_cli(&env, "probe::prints_its_environment")
        .run()
        .unwrap();

    assert!(
        output
            .stdout
            .lines()
            .any(|line| line.ends_with("env:TZ=Europe/Paris")),
        "{output}"
    );
}

#[test]
fn a_locale_outside_the_shared_table_fails_the_run_before_it_starts() {
    let env = unrecorded();

    // The table names locales by their BCP 47 tag, `tr-TR`.
    let run = cli(&env, collie())
        .args(["config", "explain", "timeout"])
        .with_locale("tr_TR.UTF-8")
        .run();

    let error = run.unwrap_err();
    assert!(
        matches!(error, CliError::LocaleUnknown { .. })
            && !env.folder().join(COMMAND_RECORD).exists(),
        "{error}"
    );
}

#[test]
fn a_time_zone_outside_the_shared_table_fails_the_run_before_it_starts() {
    let env = unrecorded();

    let run = cli(&env, collie())
        .args(["config", "explain", "timeout"])
        .with_time_zone("Europe/Berlin")
        .run();

    let error = run.unwrap_err();
    assert!(
        matches!(error, CliError::TimeZoneUnknown { .. })
            && !env.folder().join(COMMAND_RECORD).exists(),
        "{error}"
    );
}
