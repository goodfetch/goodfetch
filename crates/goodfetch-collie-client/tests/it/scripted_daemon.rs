//! The Scripted engine as the Daemon of a test's home, playing a script of
//! the corpus with this crate's `hello` in place of the TypeScript SDK's,
//! for the cases that reach a Daemon through the client crate, and the
//! lines a start the Daemon answers logs.

use goodfetch_protocol::connection::{DaemonEndpoint, HelloParams};
use goodfetch_protocol::log::LogLevel;
use goodfetch_test_support::{
    RuntimeFile, SCRIPTED_DAEMON_TOKEN, TestEnv, Tool, engine_script, reported_address, tools,
};
use serde_json::Value;

use crate::connection::hello;

/// This crate's `hello` to a Daemon, carrying the Scripted engine's token.
pub(crate) fn daemon_hello() -> HelloParams {
    HelloParams {
        token: Some(SCRIPTED_DAEMON_TOKEN.to_owned()),
        ..hello()
    }
}

/// [`daemon_hello`] at the `trace` that `logged` records, so a start
/// tells the engine no other level.
pub(crate) fn traced_daemon_hello() -> HelloParams {
    HelloParams {
        log_level: Some(LogLevel::Trace),
        ..daemon_hello()
    }
}

/// Checks that `log` holds the resume of `resumed` at info and the ended
/// `ended` at warning, with their fields, on the start's target, and
/// nothing else there.
pub(crate) fn assert_start_lines(log: &str, resumed: &str, ended: &str) {
    let start_lines: Vec<&str> = log
        .lines()
        .filter(|line| line.contains("goodfetch::client::run"))
        .collect();
    assert_eq!(start_lines.len(), 2, "{log}");
    for (line, (level, run_id, started_as)) in start_lines
        .iter()
        .zip([("INFO", resumed, "resumed"), ("WARN", ended, "ended")])
    {
        assert!(
            line.contains(level)
                && line.contains(&format!("run_id={run_id}"))
                && line.contains(&format!("started_as=\"{started_as}\"")),
            "{log}"
        );
    }
}

/// Starts the Scripted engine as the Daemon of `env`'s home, playing
/// `script` of the corpus with this crate's `hello` in place of the
/// TypeScript SDK's, and returns it with where it listens.
pub(crate) fn scripted_daemon(
    env: &TestEnv,
    script: &str,
) -> (goodfetch_test_support::Process, DaemonEndpoint) {
    scripted_daemon_hearing(env, script, &daemon_hello())
}

/// Starts the Scripted engine as [`scripted_daemon`] does, expecting
/// `hello` in place of the TypeScript SDK's.
#[expect(
    clippy::disallowed_methods,
    reason = "the test writes the Scripted engine's script and reads its runtime file before any async worker runs"
)]
pub(crate) fn scripted_daemon_hearing(
    env: &TestEnv,
    script: &str,
    hello: &HelloParams,
) -> (goodfetch_test_support::Process, DaemonEndpoint) {
    let mut played: Value =
        serde_json::from_slice(&std::fs::read(engine_script(script)).unwrap()).unwrap();
    played["steps"][0]["expect"]["params"] = serde_json::to_value(hello).unwrap();
    let path = env.folder().join("script.json");
    std::fs::write(&path, serde_json::to_vec(&played).unwrap()).unwrap();
    let scripted = tools()
        .unwrap()
        .path(Tool::ScriptedEngine)
        .unwrap()
        .to_owned();
    let mut command = env.process_command(scripted);
    command
        .arg("--script")
        .arg(&path)
        .args(["--mode", "socket"])
        .stderr(std::process::Stdio::null());
    let process = env.spawn(&mut command).unwrap();
    let reported = reported_address(&mut process.child()).unwrap();
    let runtime_file = reported.line["runtime_file"].as_str().unwrap();
    let found = RuntimeFile::read(runtime_file.as_ref()).unwrap();
    let endpoint = serde_json::from_value(serde_json::to_value(found.endpoint).unwrap()).unwrap();
    (process, endpoint)
}
