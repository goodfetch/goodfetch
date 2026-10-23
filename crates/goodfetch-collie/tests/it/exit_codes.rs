//! ADR 0069's exit-code table through the binary: one case per row. A
//! command that succeeds exits `0` and a command line it cannot read `2`;
//! a call that fails exits with its Error code's own row, else its group's,
//! the code and group coming from the Scripted engine run as a Daemon, which
//! answers `config explain --daemon` with them; a Run that ended `stopped`
//! exits `20` and resumes by its Run ID; and `SIGTERM` exits `143` once the
//! call it interrupted is cancelled.
#![expect(
    clippy::disallowed_methods,
    reason = "a test writes the script its Scripted engine plays itself"
)]
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use std::process::Stdio;

use goodfetch_test_support::{Process, TestEnv, cli, engine_script, reported_address, tools};
use rstest::rstest;
use serde_json::{Value, json};

const COLLIE: &str = env!("CARGO_BIN_EXE_goodfetch-collie");

/// The corpus's script whose Daemon answers `config.explain` with an Error
/// code, which each case replaces.
const CALL_FAILED: &str = "daemon/cli-call-failed.json";

/// Starts the Scripted engine as the Daemon of the test's home, playing
/// [`CALL_FAILED`] with `code` of the group whose JSON-RPC class is `class`
/// in its reply.
fn daemon_failing_with(env: &TestEnv, class: i64, code: &str) -> Process {
    let mut script: Value =
        serde_json::from_slice(&std::fs::read(engine_script(CALL_FAILED)).unwrap()).unwrap();
    let error = script["steps"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find_map(|step| {
            step.get_mut("reply")
                .and_then(|reply| reply.get_mut("error"))
        })
        .unwrap();
    error["code"] = json!(class);
    error["data"]["code"] = json!(code);
    let path = env.folder().join("call-failed.json");
    std::fs::write(&path, serde_json::to_vec(&script).unwrap()).unwrap();
    let scripted = tools()
        .unwrap()
        .path(goodfetch_test_support::Tool::ScriptedEngine)
        .unwrap()
        .to_owned();
    let mut command = env.process_command(scripted);
    command
        .arg("--script")
        .arg(&path)
        .args(["--mode", "socket"])
        .stderr(Stdio::null());
    let process = env.spawn(&mut command).unwrap();
    reported_address(&mut process.child()).unwrap();
    process
}

/// A call that fails exits with its Error code's own row of the table, else
/// with its group's; `person_required` is a refusal, `4`, and a code this
/// executable does not know, from a newer engine, takes its group's.
///
/// Covers: story:11-205, command:config.explain
#[rstest]
/// Covers: exit:internal
#[case::internal(-32603, "internal_error", 1)]
/// Covers: exit:program
#[case::program(-32001, "run_not_found", 3)]
/// Covers: exit:refused
#[case::refused(-32002, "person_required", 4)]
/// Covers: exit:page
#[case::page(-32003, "navigation_failed", 5)]
/// Covers: exit:model
#[case::model(-32004, "model_answer_unusable", 6)]
/// Covers: exit:deadline_exceeded
#[case::deadline_exceeded(-32005, "deadline_exceeded", 7)]
/// Covers: exit:unavailable
#[case::unavailable(-32006, "run_state_unavailable", 8)]
/// Covers: exit:not_in_product
#[case::not_in_product(-32006, "not_in_product", 10)]
/// Covers: exit:protocol_version_unsupported
#[case::protocol_version_unsupported(-32006, "protocol_version_unsupported", 11)]
/// Covers: exit:browser_host_outdated
#[case::browser_host_outdated(-32006, "browser_host_outdated", 12)]
/// Covers: exit:path_not_writable
#[case::path_not_writable(-32001, "path_not_writable", 13)]
/// Covers: exit:file_write_failed
#[case::file_write_failed(-32006, "file_write_failed", 14)]
/// Covers: exit:resources_exhausted
#[case::resources_exhausted(-32006, "resources_exhausted", 15)]
/// Covers: exit:limit_reached
#[case::limit_reached(-32002, "limit_reached", 16)]
fn a_failed_call_exits_with_its_codes_row_else_its_groups(
    #[case] class: i64,
    #[case] code: &str,
    #[case] exit: i32,
) {
    let env = TestEnv::new();
    let scripted = daemon_failing_with(&env, class, code);

    let output = cli(&env, COLLIE)
        .args(["config", "explain", "daemon", "--daemon"])
        .run()
        .unwrap();
    scripted.child().wait().unwrap();

    assert_eq!(
        (output.exit, output.stdout.as_str()),
        (Some(exit), ""),
        "{}",
        output.stderr
    );
}

/// Covers: story:11-205, command:engine.info, exit:success
#[test]
fn a_command_that_did_what_it_was_asked_exits_0() {
    let env = TestEnv::new();

    let output = cli(&env, COLLIE)
        .args(["engine", "info", "--json"])
        .run()
        .unwrap();

    assert_eq!(output.exit, Some(0), "{}", output.stderr);
}

/// Covers: story:11-205, command:engine.info, exit:usage
#[test]
fn a_command_line_with_an_unknown_flag_exits_2_before_any_engine_call() {
    let env = TestEnv::new();

    let output = cli(&env, COLLIE)
        .args(["engine", "info", "--no-such-flag"])
        .run()
        .unwrap();

    assert_eq!(
        (output.exit, output.stdout.as_str()),
        (Some(2), ""),
        "{}",
        output.stderr
    );
}

/// The Run a command started stops with its Attached engine; the command
/// that answers its report exits `20`.
///
/// Covers: story:11-206, command:run.start, command:run.finish, exit:run_stopped
#[test]
fn a_run_that_ended_stopped_exits_20() {
    let env = TestEnv::new();
    let started = cli(&env, COLLIE)
        .args(["run", "start", "--run-id", "first"])
        .run()
        .unwrap();
    assert_eq!(started.exit, Some(0), "{}", started.stderr);

    let finished = cli(&env, COLLIE)
        .args(["run", "finish", "first", "--json"])
        .json("RunReport")
        .unwrap();

    assert_eq!(
        (
            finished.output.exit,
            finished.documents[0]["lifecycle"].clone()
        ),
        (Some(20), json!("stopped"))
    );
}

/// A Run that ended `stopped` goes on by its Run ID.
///
/// Covers: story:11-206, command:run.start, command:run.resume
#[test]
fn a_run_that_ended_stopped_resumes_by_its_run_id() {
    let env = TestEnv::new();
    let started = cli(&env, COLLIE)
        .args(["run", "start", "--run-id", "first"])
        .run()
        .unwrap();
    assert_eq!(started.exit, Some(0), "{}", started.stderr);

    let resumed = cli(&env, COLLIE)
        .args(["run", "resume", "first", "--json"])
        .json("RunStarted")
        .unwrap();

    assert_eq!(
        (
            resumed.output.exit,
            resumed.documents[0]["started_as"].clone()
        ),
        (Some(0), json!("resumed"))
    );
}

/// `--require-verified` on a Run that ended `stopped` keeps `20`: its
/// Verdict is not its last, and a resume goes on with it.
///
/// Covers: story:11-206, command:run.start, command:run.finish, exit:run_stopped
#[test]
fn a_stopped_run_exits_20_under_require_verified() {
    let env = TestEnv::new();
    let started = cli(&env, COLLIE)
        .args(["run", "start", "--run-id", "first"])
        .run()
        .unwrap();
    assert_eq!(started.exit, Some(0), "{}", started.stderr);

    let finished = cli(&env, COLLIE)
        .args(["run", "finish", "first", "--require-verified"])
        .run()
        .unwrap();

    assert_eq!(finished.exit, Some(20), "{}", finished.stderr);
}

#[cfg(unix)]
const SUBSCRIBE: [&str; 6] = [
    "events",
    "subscribe",
    "--after",
    r#"{"engine_instance":"another-engine","position":100}"#,
    "--follow",
    "--json",
];

/// `SIGTERM` cancels the call in flight, as `SIGINT` does, and the command
/// exits `143`.
///
/// Covers: story:11-206, command:events.subscribe, exit:terminated
#[cfg(unix)]
#[test]
fn sigterm_cancels_the_call_and_exits_143() {
    use std::io::{BufRead as _, BufReader, Read as _};

    use goodfetch_test_support::record_command;

    let env = TestEnv::new();
    env.log_level("trace");
    let mut command = env.process_command(COLLIE);
    command
        .args(SUBSCRIBE)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let process = env.spawn(&mut command).unwrap();
    let mut stdout = BufReader::new(process.child().stdout.take().unwrap());
    // A position another engine instance gave starts with `events.resync`:
    // the subscription is in flight once it is printed.
    let mut resync = String::new();
    stdout.read_line(&mut resync).unwrap();

    process.terminate().unwrap();

    let status = process.child().wait().unwrap();
    record_command(&env, &SUBSCRIBE, status.code()).unwrap();
    let mut stderr = String::new();
    let mut pipe = process.child().stderr.take().unwrap();
    pipe.read_to_string(&mut stderr).unwrap();
    let resync: Value = serde_json::from_str(&resync).unwrap();
    let cancelled = stderr.lines().any(|line| {
        line.contains(r#"request failed method="events.subscribe" error_code="call_cancelled""#)
    });
    assert_eq!(
        (resync["event"].clone(), cancelled, status.code()),
        (json!("events.resync"), true, Some(143)),
        "{stderr}"
    );
}
