//! A start reaching the Run its Run ID names through the client crate
//! (ADRs 0078 and 0102), with the Scripted engine as the Daemon playing the
//! corpus's script of the `start-reaches-its-run` client conformance case,
//! which the TypeScript SDK plays too: a fresh Run ID begins the Run, an
//! interrupted Run's resumes it, logged at info, and a finished Run's
//! returns it ended, logged at warning, where a call that would give it
//! work fails with `run_ended`; each start the engine refuses fails with
//! its code and details. The script checks every message the crate sends,
//! so `Run::id` sends none.
//! Covers: story:11-126, story:11-127

use goodfetch_collie_client::transport::{Endpoint, Network};
use goodfetch_collie_client::{ClientConnection, ConnectionRuntime, Error};
use goodfetch_protocol::connection::{DaemonEndpoint, HelloParams};
use goodfetch_protocol::log::LogLevel;
use goodfetch_protocol::run::{RunConflictDetails, RunDetails, RunId, StartInput, StartedAs, Tags};
use goodfetch_test_support::{
    RuntimeFile, SCRIPTED_DAEMON_TOKEN, TestEnv, Tool, engine_script, logged, reported_address,
    tools,
};
use serde_json::{Value, json};

use crate::connection::hello;

/// The corpus's script of the case.
const SCRIPT: &str = "run/start-reaches-its-run.json";

/// This crate's `hello` to a Daemon, carrying the Scripted engine's token
/// and the `trace` that `logged` records, so a start tells the engine no
/// other level.
fn daemon_hello() -> HelloParams {
    HelloParams {
        token: Some(SCRIPTED_DAEMON_TOKEN.to_owned()),
        log_level: Some(LogLevel::Trace),
        ..hello()
    }
}

/// Starts the Scripted engine as the Daemon of `env`'s home, playing the
/// case's script with this crate's `hello` in place of the TypeScript
/// SDK's, and returns it with where it listens.
#[expect(
    clippy::disallowed_methods,
    reason = "the test writes the Scripted engine's script and reads its runtime file before any async worker runs"
)]
fn daemon(env: &TestEnv) -> (goodfetch_test_support::Process, DaemonEndpoint) {
    let mut script: Value =
        serde_json::from_slice(&std::fs::read(engine_script(SCRIPT)).unwrap()).unwrap();
    script["steps"][0]["expect"]["params"] = serde_json::to_value(daemon_hello()).unwrap();
    let path = env.folder().join("start-reaches-its-run.json");
    std::fs::write(&path, serde_json::to_vec(&script).unwrap()).unwrap();
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

/// The failure `result` holds, as its code, whether it may be retried, and
/// its details.
fn failure<T: std::fmt::Debug>(result: Result<T, Error>) -> (String, bool, Value) {
    let Err(Error::Failed(failure)) = result else {
        panic!("{result:?}");
    };
    (
        failure.data.code,
        failure.data.retryable,
        Value::Object(failure.data.details),
    )
}

/// Reads `details` as the code's own details type.
fn details<T: serde::de::DeserializeOwned>(details: &Value) -> T {
    serde_json::from_value(details.clone()).unwrap()
}

/// What each step of the script answered.
struct Played {
    started_as: [StartedAs; 3],
    named: RunId,
    no_work: (String, bool, Value),
    in_use: (String, bool, Value),
    conflict: (String, bool, Value),
    reached: (String, bool, Value),
}

/// Plays the script's steps on a Connection to the Daemon at `endpoint`.
async fn play(endpoint: DaemonEndpoint) -> Played {
    let id = |text: &str| RunId::parse(text).unwrap();
    let (connection, _said) = ClientConnection::open(
        ConnectionRuntime::driven(tokio::runtime::Handle::current()),
        &Network::Os,
        &Endpoint::Daemon(endpoint),
        daemon_hello(),
    )
    .await
    .unwrap();

    let begun = connection
        .start()
        .run_id(&id("nightly"))
        .tags(Tags::parse(["shop", "eu"]).unwrap())
        .await
        .unwrap();
    let named = begun.id().clone();
    begun.close().await.unwrap();
    let resumed = connection.start().run_id(&id("nightly")).await.unwrap();
    resumed.finish().await.unwrap();
    let ended = connection.start().run_id(&id("nightly")).await.unwrap();
    let no_work = failure(ended.new_page().await);
    let in_use = failure(connection.start().run_id(&id("busy")).await);
    let conflict = failure(connection.start().run_id(&id("crawled")).await);
    let reached = failure(connection.start().run_id(&id("capped")).await);
    connection.close().await.unwrap();
    Played {
        started_as: [begun.started_as(), resumed.started_as(), ended.started_as()],
        named,
        no_work,
        in_use,
        conflict,
        reached,
    }
}

/// Checks that `log` holds the resume at info and the ended Run at warning,
/// with their fields, on the start's target, and nothing else there.
fn assert_start_lines(log: &str) {
    let start_lines: Vec<&str> = log
        .lines()
        .filter(|line| line.contains("goodfetch::client::run"))
        .collect();
    assert_eq!(start_lines.len(), 2, "{log}");
    for (line, (level, started_as)) in start_lines
        .iter()
        .zip([("INFO", "resumed"), ("WARN", "ended")])
    {
        assert!(
            line.contains(level)
                && line.contains("run_id=nightly")
                && line.contains(&format!("started_as=\"{started_as}\"")),
            "{log}"
        );
    }
}

#[test]
fn a_start_reaches_the_run_its_run_id_names_logs_a_resume_and_an_end_and_fails_each_refusal() {
    let env = TestEnv::new();
    let (process, endpoint) = daemon(&env);
    let id = |text: &str| RunId::parse(text).unwrap();
    let runtime = goodfetch_test_support::real_time_runtime();
    let (played, log) = logged(|| runtime.block_on(play(endpoint)));
    let Played {
        started_as,
        named,
        no_work,
        in_use,
        conflict,
        reached,
    } = played;

    assert_eq!(
        (started_as, named.as_str()),
        (
            [StartedAs::New, StartedAs::Resumed, StartedAs::Ended],
            "nightly"
        )
    );
    assert_start_lines(&String::from_utf8(log).unwrap());
    assert_eq!(
        (
            no_work.0.as_str(),
            no_work.1,
            details::<RunDetails>(&no_work.2).run_id
        ),
        ("run_ended", false, id("nightly"))
    );
    assert_eq!(
        (
            in_use.0.as_str(),
            in_use.1,
            details::<RunDetails>(&in_use.2).run_id
        ),
        ("run_in_use", true, id("busy"))
    );
    assert_eq!(
        (
            conflict.0.as_str(),
            details::<RunConflictDetails>(&conflict.2)
        ),
        (
            "run_conflict",
            RunConflictDetails {
                run_id: id("crawled"),
                differences: vec![
                    StartInput::Product,
                    StartInput::Kind,
                    StartInput::Caller,
                    StartInput::Seeds
                ],
            }
        )
    );
    assert_eq!(
        (
            reached.0.as_str(),
            &reached.2["scope"],
            &reached.2["usage_kind"],
            &reached.2["resets_at"],
        ),
        (
            "limit_reached",
            &json!({ "kind": "run", "id": "capped" }),
            &json!("requests"),
            &json!("2026-02-01T00:00:00Z"),
        )
    );
    let status = process.child().wait().unwrap();
    assert!(
        status.success(),
        "the Scripted engine played its script whole"
    );
}
