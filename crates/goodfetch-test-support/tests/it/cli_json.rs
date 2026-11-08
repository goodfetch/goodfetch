//! The CLI helper's `--json` checks and modes (ADR 0069): `json` validates
//! stdout against a schema of the committed bundle and refuses anything
//! else on stdout, `json_lines` does the same for each line of an event
//! stream, and `Mode::Daemon` runs a command against a Daemon of the test's
//! home, which it stops afterwards, so one rstest case per mode shares one
//! snapshot.

use std::path::Path;

use goodfetch_test_support::{
    Cli, CliError, DISPATCH_TARGET, Mode, RUNTIME_FILE, TestEnv, Tool, assert_cli_snapshot, cli,
    tools,
};
use rstest::rstest;
use serde_json::{Value, json};

use crate::cli::{collie, unrecorded};
#[cfg(unix)]
use crate::test_env_daemon::{LEAVES_ITS_RUNTIME_FILE, stand_in};

/// An explanation of no setting, valid against `SettingsExplanation`.
const EXPLANATION: &str = r#"{"files":[],"settings":[]}"#;

/// The CLI helper running the Scripted engine as a fixture command whose
/// stdout is `stdout`, written as it is, and whose exit code is 0.
fn printing<'env>(env: &'env TestEnv, stdout: &str) -> Cli<'env> {
    let script = env.folder().join(env.unique_name("prints") + ".json");
    let steps = json!({
        "description": "a fixture command's stdout",
        "steps": [{ "raw": { "text": stdout } }],
    });
    std::fs::write(&script, steps.to_string()).unwrap();
    let engine = tools()
        .unwrap()
        .path(Tool::ScriptedEngine)
        .unwrap()
        .to_owned();
    cli(env, engine).args(["--script", script.to_str().unwrap(), "--mode", "stdio"])
}

#[test]
fn json_accepts_a_commands_document_valid_against_its_schema() {
    let env = unrecorded();

    let checked = cli(&env, collie())
        .args(["config", "explain", "timeout.navigation", "--json"])
        .json("SettingsExplanation")
        .unwrap();

    assert_eq!(
        (
            checked.output.exit,
            checked.documents.len(),
            checked.documents[0]["settings"][0]["key"].clone(),
        ),
        (Some(0), 1, json!("timeout.navigation"))
    );
}

#[test]
fn json_snapshots_the_document_pretty_printed_with_the_exit_code_and_stderr() {
    let env = unrecorded();

    let checked = printing(&env, &format!("{EXPLANATION}\n"))
        .json("SettingsExplanation")
        .unwrap();

    assert_cli_snapshot!(checked.output, @r#"
    exit: 0
    ----- stdout -----
    {
      "files": [],
      "settings": []
    }
    ----- stderr -----
    "#);
}

#[rstest]
#[case::violating_its_schema(r#"{"files":[],"settings":"every key"}"#)]
#[case::missing_a_required_field(r#"{"settings":[]}"#)]
fn json_rejects_a_document_violating_its_schema(#[case] stdout: &str) {
    let env = unrecorded();

    let checked = printing(&env, &format!("{stdout}\n")).json("SettingsExplanation");

    let error = checked.unwrap_err();
    assert!(
        matches!(&error, CliError::SchemaViolated { schema, errors, .. }
            if schema == "SettingsExplanation" && !errors.is_empty()),
        "{error}"
    );
}

#[rstest]
#[case::on_its_own_line(format!("{EXPLANATION}\n{EXPLANATION}\n"))]
#[case::on_the_same_line(format!("{EXPLANATION} {EXPLANATION}\n"))]
fn json_rejects_a_second_document_on_stdout(#[case] stdout: String) {
    let env = unrecorded();

    let checked = printing(&env, &stdout).json("SettingsExplanation");

    let error = checked.unwrap_err();
    assert!(matches!(error, CliError::SecondDocument { .. }), "{error}");
}

#[rstest]
#[case::after_the_document(format!("{EXPLANATION}\nDone.\n"))]
#[case::before_the_document(format!("Resolving settings…\n{EXPLANATION}\n"))]
#[case::alone("Done.\n".to_owned())]
#[case::nothing(String::new())]
fn json_rejects_a_stray_line_or_no_document_on_stdout(#[case] stdout: String) {
    let env = unrecorded();

    let checked = printing(&env, &stdout).json("SettingsExplanation");

    let error = checked.unwrap_err();
    assert!(matches!(error, CliError::NotJson { .. }), "{error}");
}

#[test]
fn json_refuses_a_schema_the_bundle_does_not_hold() {
    let env = unrecorded();

    let checked = printing(&env, &format!("{EXPLANATION}\n")).json("NoSuchOutput");

    let error = checked.unwrap_err();
    assert!(
        matches!(&error, CliError::SchemaUnknown { schema } if schema == "NoSuchOutput"),
        "{error}"
    );
}

/// Two events of a stream, valid against `EventEnvelope`.
const EVENTS: &str = concat!(
    r#"{"position":1,"event":"run.started","data":{}}"#,
    "\n",
    r#"{"position":2,"event":"run.finished","data":{"pages":3}}"#,
    "\n",
);

#[test]
fn json_lines_accepts_lines_each_valid_against_the_event_type() {
    let env = unrecorded();

    let checked = printing(&env, EVENTS).json_lines("EventEnvelope").unwrap();

    assert_eq!(
        checked
            .documents
            .iter()
            .map(|event| event["position"].clone())
            .collect::<Vec<Value>>(),
        vec![json!(1), json!(2)]
    );
}

#[test]
fn json_lines_rejects_a_line_of_another_type() {
    let env = unrecorded();

    let checked = printing(&env, &format!("{EVENTS}{EXPLANATION}\n")).json_lines("EventEnvelope");

    let error = checked.unwrap_err();
    assert!(
        matches!(&error, CliError::SchemaViolated { line: Some(3), .. }),
        "{error}"
    );
}

/// The two events, as one JSON array: what a command of a sequence that
/// ended prints under `--json`.
const EVENT_ARRAY: &str = concat!(
    r#"[{"position":1,"event":"run.started","data":{}},"#,
    r#"{"position":2,"event":"run.finished","data":{"pages":3}}]"#,
    "\n",
);

#[test]
fn json_array_accepts_one_array_whose_items_are_each_valid_against_the_type() {
    let env = unrecorded();

    let checked = printing(&env, EVENT_ARRAY)
        .json_array("EventEnvelope")
        .unwrap();

    assert_eq!(
        checked.documents,
        [json!([
            { "position": 1, "event": "run.started", "data": {} },
            { "position": 2, "event": "run.finished", "data": { "pages": 3 } },
        ])]
    );
}

#[rstest]
#[case::an_item_of_another_type(format!("[{}]\n", EXPLANATION))]
#[case::one_item_not_in_an_array(EVENTS.lines().next().unwrap().to_owned() + "\n")]
fn json_array_rejects_anything_but_an_array_of_the_type(#[case] stdout: String) {
    let env = unrecorded();

    let checked = printing(&env, &stdout).json_array("EventEnvelope");

    let error = checked.unwrap_err();
    assert!(
        matches!(&error, CliError::SchemaViolated { line: None, .. }),
        "{error}"
    );
}

#[test]
fn json_lines_rejects_a_stdout_with_no_line() {
    let env = unrecorded();

    let checked = printing(&env, "").json_lines("EventEnvelope");

    let error = checked.unwrap_err();
    assert!(
        matches!(&error, CliError::NotJson { line: None, .. }),
        "{error}"
    );
}

#[rstest]
#[case::a_stray_line(format!("{EVENTS}Done.\n"))]
#[case::two_documents_on_one_line(EVENTS.replace('\n', " ") + "\n")]
#[case::a_blank_line(EVENTS.replace('\n', "\n\n"))]
fn json_lines_rejects_a_line_that_is_not_one_document(#[case] stdout: String) {
    let env = unrecorded();

    let checked = printing(&env, &stdout).json_lines("EventEnvelope");

    let error = checked.unwrap_err();
    assert!(matches!(error, CliError::NotJson { .. }), "{error}");
}

/// Whether the home holds a Daemon's runtime file of any version, so the
/// check holds whatever the workspace's version is.
fn any_runtime_file(home: &Path) -> bool {
    std::fs::read_dir(home).is_ok_and(|majors| {
        majors
            .flatten()
            .any(|major| major.path().join(RUNTIME_FILE).exists())
    })
}

/// Whether one of the home's log files records the engine dispatching
/// `method`, as only a Daemon does for `config explain`.
fn dispatched(home: &Path, method: &str) -> bool {
    let Ok(logs) = std::fs::read_dir(home.join("logs")) else {
        return false;
    };
    logs.flatten().any(|log| {
        std::fs::read_to_string(log.path())
            .unwrap()
            .lines()
            .filter_map(|line| serde_json::from_str::<Value>(line).ok())
            .any(|line| line["target"] == DISPATCH_TARGET && line["fields"]["method"] == method)
    })
}

#[test]
fn a_command_prints_the_same_bytes_in_each_mode() {
    let env = unrecorded();
    let printed = |mode| {
        cli(&env, collie())
            .args(["config", "explain", "timeout.navigation", "--json"])
            .mode(mode)
            .json("SettingsExplanation")
            .unwrap()
            .printed
    };

    let (attached, daemon) = (printed(Mode::Attached), printed(Mode::Daemon));

    assert_eq!(attached, daemon);
}

#[rstest]
#[case::attached(Mode::Attached)]
#[case::daemon(Mode::Daemon)]
fn a_command_run_in_each_mode_matches_one_snapshot_and_leaves_no_daemon(#[case] mode: Mode) {
    let env = unrecorded();

    let checked = cli(&env, collie())
        .args(["config", "explain", "timeout.navigation", "--json"])
        .mode(mode)
        .json("SettingsExplanation")
        .unwrap();

    assert_cli_snapshot!("config_explain_timeout_navigation_json", checked.output);
    assert_eq!(
        (
            dispatched(env.home(), "config.explain"),
            any_runtime_file(env.home()),
        ),
        (mode == Mode::Daemon, false)
    );
}

#[cfg(unix)]
#[test]
fn a_daemon_mode_run_whose_daemon_leaves_its_runtime_file_fails() {
    let env = unrecorded();
    let program = stand_in(&env, "leaving-daemon", LEAVES_ITS_RUNTIME_FILE, "");

    let run = cli(&env, &program)
        .args(["config", "explain"])
        .mode(Mode::Daemon)
        .run();

    let runtime_file = goodfetch_test_support::RuntimeFile::path(env.home(), "0.0.0");
    assert!(
        matches!(&run, Err(CliError::Daemon(goodfetch_test_support::DaemonError::RuntimeFileLeft(path))) if *path == runtime_file),
        "{run:?}"
    );
}
