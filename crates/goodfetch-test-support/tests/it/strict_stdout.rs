//! `StrictStdout` reading a real child's stdout, started through
//! `env.spawn` or `command(&env, …)`: a child that writes only JSON-RPC
//! messages passes, one that writes anything else fails naming the line,
//! and its stderr is kept apart and shown on failure (ADRs 0069, 0070).

use std::process::Stdio;
use std::time::Duration;

use goodfetch_test_support::{
    Line, Process, StrictStdout, StrictStdoutErrorKind, TestEnv, command,
};
use rstest::rstest;
use serde_json::json;

/// What the fixture child writes on its stderr: per-message logs at
/// `trace`, before its stdout.
const TRACE: &str = "2026-01-05T09:00:00Z TRACE goodfetch::rpc: sent hello\n\
                     2026-01-05T09:00:00Z TRACE goodfetch::rpc: sent run.progress\n";

/// A request, a notification and a reply, one per line.
const MESSAGES: &[u8] = b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"hello\"}\n\
    {\"jsonrpc\":\"2.0\",\"method\":\"run.progress\",\"params\":{\"done\":1}}\n\
    {\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{\"ok\":true}}\n";

/// The command of a child that writes `TRACE` on its stderr, then `stdout`
/// byte for byte on its stdout, and exits: the OS's own shell copying two
/// files of the test's folder.
fn fixture_child(env: &TestEnv, stdout: &[u8]) -> std::process::Command {
    std::fs::write(env.folder().join("stdout.bin"), stdout).unwrap();
    std::fs::write(env.folder().join("stderr.txt"), TRACE).unwrap();
    let mut command = if cfg!(windows) {
        let mut command = env.process_command(&env.env()["ComSpec"]);
        command.args(["/C", "type stderr.txt 1>&2 & type stdout.bin"]);
        command
    } else {
        let mut command = env.process_command("/bin/sh");
        command.args(["-c", "cat stderr.txt >&2; cat stdout.bin"]);
        command
    };
    command.stderr(Stdio::piped());
    command
}

/// Starts the fixture child through `env.spawn`.
fn spawn_fixture_child(env: &TestEnv, stdout: &[u8]) -> Process {
    env.spawn(&mut fixture_child(env, stdout)).unwrap()
}

/// `MESSAGES`' first line, then `line`.
fn after_a_request(line: &[u8]) -> Vec<u8> {
    let mut stdout = MESSAGES[..=MESSAGES.iter().position(|byte| *byte == b'\n').unwrap()].to_vec();
    stdout.extend_from_slice(line);
    stdout
}

#[test]
fn a_child_writing_a_request_a_notification_and_a_reply_passes() {
    let env = TestEnv::new();
    let process = spawn_fixture_child(&env, MESSAGES);
    let stdout = StrictStdout::of(&process).unwrap();

    let messages: Vec<serde_json::Value> = stdout.collect();

    process.close_stdin_and_wait().unwrap();
    assert_eq!(
        messages,
        [
            json!({"jsonrpc":"2.0","id":1,"method":"hello"}),
            json!({"jsonrpc":"2.0","method":"run.progress","params":{"done":1}}),
            json!({"jsonrpc":"2.0","id":1,"result":{"ok":true}}),
        ]
    );
}

#[rstest]
#[case::plain_log_line(b"2026-01-05T09:00:00Z INFO goodfetch: engine ready")]
fn a_child_writing_anything_else_fails_naming_the_line_and_showing_its_stderr(#[case] line: &[u8]) {
    let env = TestEnv::new();
    let mut written = after_a_request(line);
    written.push(b'\n');
    let process = spawn_fixture_child(&env, &written);
    let mut stdout = StrictStdout::of(&process).unwrap();
    stdout.try_next().unwrap();

    let error = stdout.try_next().unwrap_err();

    process.close_stdin_and_wait().unwrap();
    assert!(
        matches!(error.kind, StrictStdoutErrorKind::NotJson(_)),
        "{error}"
    );
    assert_eq!(
        error.kind.line(),
        Some(&Line {
            number: 2,
            bytes: line.to_vec()
        })
    );
    assert_eq!(error.stderr, TRACE);
}

#[test]
fn a_child_run_by_command_is_read_from_its_output() {
    let env = TestEnv::new();
    let child = fixture_child(&env, MESSAGES);
    let output = command(&env, child.get_program())
        .args(child.get_args())
        .output()
        .unwrap();

    let messages: Vec<serde_json::Value> = StrictStdout::from_output(&output).collect();

    assert_eq!(
        messages,
        [
            json!({"jsonrpc":"2.0","id":1,"method":"hello"}),
            json!({"jsonrpc":"2.0","method":"run.progress","params":{"done":1}}),
            json!({"jsonrpc":"2.0","id":1,"result":{"ok":true}}),
        ]
    );
}

#[test]
fn a_child_run_by_command_that_writes_a_log_line_fails_showing_its_stderr() {
    let env = TestEnv::new();
    let child = fixture_child(&env, &after_a_request(b"engine ready\n"));
    let output = command(&env, child.get_program())
        .args(child.get_args())
        .output()
        .unwrap();
    let mut stdout = StrictStdout::from_output(&output);
    stdout.try_next().unwrap();

    let error = stdout.try_next().unwrap_err();

    assert_eq!(
        error.kind.line(),
        Some(&Line {
            number: 2,
            bytes: b"engine ready".to_vec()
        })
    );
    assert_eq!(error.stderr, TRACE);
}

#[test]
fn a_child_that_writes_nothing_fails_at_the_deadline() {
    let env = TestEnv::new();
    let mut command = if cfg!(windows) {
        let mut command = env.process_command(&env.env()["ComSpec"]);
        command.args(["/C", "set /p line="]);
        command
    } else {
        let mut command = env.process_command("/bin/sh");
        command.args(["-c", "read line"]);
        command
    };
    command.stderr(Stdio::piped());
    let process = env.spawn(&mut command).unwrap();
    let mut stdout = StrictStdout::of(&process)
        .unwrap()
        .within(Duration::from_millis(200));

    let error = stdout.try_next().unwrap_err();

    process.close_stdin_and_wait().unwrap();
    assert!(
        matches!(error.kind, StrictStdoutErrorKind::Deadline(_)),
        "{error}"
    );
}

#[test]
fn a_child_whose_stderr_is_not_piped_is_refused() {
    let env = TestEnv::new();
    let mut command = fixture_child(&env, MESSAGES);
    command.stderr(Stdio::null());
    let process = env.spawn(&mut command).unwrap();

    let stdout = StrictStdout::of(&process);

    process.close_stdin_and_wait().unwrap();
    assert!(matches!(
        stdout.unwrap_err().kind,
        StrictStdoutErrorKind::StderrNotPiped
    ));
}
