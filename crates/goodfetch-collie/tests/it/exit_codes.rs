//! ADR 0069's exit-code table through the binary: one case per row. A
//! command that succeeds exits `0` and a command line it cannot read `2`;
//! a call that fails exits with its Error code's own row, else its group's,
//! the code and group coming from the Scripted engine run as a Daemon, which
//! answers `config explain --daemon` with them; a Run that ended `stopped`
//! exits `20` and resumes by its Run ID; and `SIGTERM` exits `143` once the
//! call it interrupted is cancelled. A failure before any call, as a
//! Daemon nothing answers for or a deployment variable the executable
//! cannot take, prints as every failure does, `{ "error" }` under
//! `--json`, and exits by its code's row.
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

use crate::daemon::scripted_daemon;

const COLLIE: &str = env!("CARGO_BIN_EXE_goodfetch-collie");

/// A token for a Daemon that never reads it: no real token's shape.
const UNREAD_TOKEN: &str = "not-a-real-token";

/// The Error code and the exit of a failure printed as `{ "error" }` on
/// stdout, as `--json` prints it, or what stdout held instead.
fn failed_with(output: &goodfetch_test_support::CliOutput) -> (Option<i32>, Value) {
    let printed: Value = serde_json::from_str(&output.stdout).unwrap_or(Value::Null);
    (output.exit, printed["error"]["code"].clone())
}

/// A command whose Daemon nothing answers for, the remote one
/// `GOODFETCH_URL` names at a loopback port nobody listens on, fails with
/// `daemon_unreachable`, an `UnavailableError` whose `fix` says what to
/// do: `{ "error" }` under `--json`, exit `8`, never exit `1` with a log
/// line alone.
///
/// Covers: story:11-199, story:11-205, command:engine.info, error:daemon_unreachable, exit:unavailable
#[test]
fn a_command_that_reaches_no_daemon_prints_daemon_unreachable_and_exits_8() {
    let env = TestEnv::new();
    // A port the test took and let go, where nothing listens: a refused
    // connection, never a fixed port.
    #[expect(
        clippy::disallowed_types,
        reason = "the test finds a loopback port nobody listens on, outside the engine (ADR 0060)"
    )]
    let closed = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap();

    let output = cli(&env, COLLIE)
        .args(["engine", "info", "--json"])
        .env("GOODFETCH_URL", format!("ws://{closed}"))
        .env("GOODFETCH_TOKEN", UNREAD_TOKEN)
        .run()
        .unwrap();
    let printed: Value = serde_json::from_str(&output.stdout).unwrap_or(Value::Null);

    assert_eq!(
        (
            failed_with(&output),
            printed["error"]["retryable"].clone(),
            printed["error"]["fix"].is_string(),
        ),
        ((Some(8), json!("daemon_unreachable")), json!(true), true),
        "{output}"
    );
}

/// A `GOODFETCH_URL` that is not loopback is refused with `setting_invalid`
/// before any Connection, exit `3`, as the Daemon refuses such an address
/// until it terminates TLS: its token would travel in clear (608-O2).
///
/// Covers: story:11-199, command:engine.info, error:setting_invalid, exit:program
#[test]
fn a_goodfetch_url_that_is_not_loopback_is_refused_with_setting_invalid() {
    let env = TestEnv::new();

    let output = cli(&env, COLLIE)
        .args(["engine", "info", "--json"])
        .env("GOODFETCH_URL", "ws://192.0.2.1:7900")
        .env("GOODFETCH_TOKEN", UNREAD_TOKEN)
        .run()
        .unwrap();
    let printed: Value = serde_json::from_str(&output.stdout).unwrap_or(Value::Null);

    assert_eq!(
        (failed_with(&output), printed["error"]["key"].clone()),
        ((Some(3), json!("setting_invalid")), json!("GOODFETCH_URL")),
        "{output}"
    );
}

/// A deployment variable the executable cannot take fails the command
/// before any engine starts as every failure prints, `{ "error" }` under
/// `--json`, with no colour under `--color never`, and exits by its code's
/// row: whether clap read the command line, or refused what only Foxhound
/// carries.
///
/// Covers: story:11-205, error:setting_invalid, exit:program
#[rstest]
/// Covers: command:engine.info
#[case::a_command_it_reads(&["engine", "info", "--json", "--color", "never"])]
#[case::what_only_foxhound_carries(&["crawl", "start", "https://example.com/", "--json", "--color=never"])]
fn a_bad_deployment_variable_fails_as_every_failure_prints(#[case] arguments: &[&str]) {
    let env = TestEnv::new();

    let output = cli(&env, COLLIE)
        .args(arguments)
        .env("GOODFETCH_CHANNEL", "nowhere")
        .run()
        .unwrap();

    assert_eq!(
        failed_with(&output),
        (Some(3), json!("setting_invalid")),
        "{output}"
    );
}

/// Under `--daemon`, a Daemon that refuses `hello` by protocol version and
/// cannot hand over fails `audit export` and `audit verify` as every
/// command's refusal prints, its `fix` naming this executable's `daemon
/// restart`, exit `11`.
///
/// Covers: story:11-199, error:protocol_version_unsupported, exit:protocol_version_unsupported
#[rstest]
/// Covers: command:audit.export
#[case::export("export")]
/// Covers: command:audit.verify
#[case::verify("verify")]
fn a_refused_audit_command_shows_the_refusals_fix(#[case] command: &str) {
    let env = TestEnv::new();
    let scripted = scripted_daemon(&env, "daemon/cli-hello-refused.json");

    let output = cli(&env, COLLIE)
        .args(["audit", command, "--daemon"])
        .run()
        .unwrap();
    scripted.child().wait().unwrap();

    assert_eq!(
        (
            output.exit,
            output.stdout.as_str(),
            output
                .stderr
                .lines()
                .any(|line| line.starts_with("fix:") && line.contains("daemon restart")),
        ),
        (Some(11), "", true),
        "{output}"
    );
}

/// The person-only commands, at a terminal, print the same refusal: `usage
/// raise` once the person answered yes, `certs accept` and `certs forget`
/// on the call that reads the pin, each with its `fix` line and exit `11`.
///
/// Covers: story:11-199, error:protocol_version_unsupported, exit:protocol_version_unsupported
#[rstest]
/// Covers: command:usage.raise
#[case::usage_raise(&["usage", "raise", "--scope", "install", "--usage-kind", "requests", "--amount", "1000", "--daemon"], Some("y"))]
/// Covers: command:certs.accept
#[case::certs_accept(&["certs", "accept", "example.com", "--daemon"], None)]
/// Covers: command:certs.forget
#[case::certs_forget(&["certs", "forget", "example.com", "--daemon"], None)]
fn a_refused_person_command_shows_the_refusals_fix(
    #[case] arguments: &[&str],
    #[case] answer: Option<&str>,
) {
    let env = TestEnv::new();
    let scripted = scripted_daemon(&env, "daemon/cli-hello-refused.json");

    let mut asked = goodfetch_test_support::pty(&env, COLLIE)
        .args(arguments)
        .spawn()
        .unwrap();
    if let Some(answer) = answer {
        asked.expect("[y/N]").unwrap();
        asked.send_line(answer).unwrap();
    }
    let output = asked.finish().unwrap();
    scripted.child().wait().unwrap();

    let shown = output.stripped();
    assert_eq!(
        (
            output.exit,
            shown
                .lines()
                .any(|line| line.starts_with("fix:") && line.contains("daemon restart")),
        ),
        (Some(11), true),
        "{shown}"
    );
}

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
        .args(["run", "start", "--run-id", "first", "--attached"])
        .run()
        .unwrap();
    assert_eq!(started.exit, Some(0), "{}", started.stderr);

    let finished = cli(&env, COLLIE)
        .args(["run", "finish", "first", "--json", "--attached"])
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
        .args(["run", "start", "--run-id", "first", "--attached"])
        .run()
        .unwrap();
    assert_eq!(started.exit, Some(0), "{}", started.stderr);

    let resumed = cli(&env, COLLIE)
        .args(["run", "resume", "first", "--json", "--attached"])
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
        .args(["run", "start", "--run-id", "first", "--attached"])
        .run()
        .unwrap();
    assert_eq!(started.exit, Some(0), "{}", started.stderr);

    let finished = cli(&env, COLLIE)
        .args(["run", "finish", "first", "--require-verified", "--attached"])
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
