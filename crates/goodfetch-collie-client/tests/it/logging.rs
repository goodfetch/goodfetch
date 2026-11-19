//! The engine's logs through the client crate (ADR 0078), with the
//! Scripted engine as the Daemon playing the corpus's scripts of the
//! `logging` client conformance cases, which the TypeScript and Python
//! SDKs play too: `hello` carries the level the program's subscriber
//! records for `goodfetch::engine`, a start tells the engine that level
//! again only when it changed, and each `$/log` event reaches the
//! program's subscriber under `goodfetch::engine::…`. The Scripted engine
//! fails on a message it does not expect and on a step left unused, so
//! its clean exit says the crate sent exactly what the script holds.
//! These are Rust's `daemon` mode plays; `goodfetch-collie`'s
//! `sdk::logging` tests play the cases `linked`.

use std::time::Duration;

use goodfetch_collie_client::logging::{self, engine_log_level};
use goodfetch_collie_client::transport::{Endpoint, Network};
use goodfetch_collie_client::{ClientConnection, ConnectionRuntime};
use goodfetch_protocol::connection::{DaemonEndpoint, HelloParams};
use goodfetch_protocol::log::LogLevel;
use goodfetch_protocol::run::RunId;
use goodfetch_test_support::{CapturedLog, Process, TestEnv, capturing};
use tracing::Level;
use tracing::subscriber::with_default;

use crate::scripted_daemon::{daemon_hello, scripted_daemon_hearing};

/// This crate's `hello` to the Scripted engine's Daemon, carrying the
/// level the program's subscriber records for the engine.
fn hello_at_the_recorded_level() -> HelloParams {
    HelloParams {
        log_level: Some(engine_log_level()),
        ..daemon_hello()
    }
}

/// A Connection to the Scripted engine's Daemon at `endpoint`, on the
/// test's runtime, saying `hello`.
async fn connect(endpoint: DaemonEndpoint, hello: HelloParams) -> ClientConnection {
    let (connection, _said) = ClientConnection::open(
        ConnectionRuntime::driven(tokio::runtime::Handle::current()),
        &Network::Os,
        &Endpoint::Daemon(endpoint),
        hello,
    )
    .await
    .unwrap();
    connection
}

/// Starts the Run `run_id` on `connection` and finishes it.
async fn start_and_finish(connection: &ClientConnection, run_id: &str) {
    let run_id = RunId::parse(run_id).unwrap();
    let run = connection.start().run_id(&run_id).await.unwrap();
    run.finish().await.unwrap();
}

/// Waits for the Scripted engine to exit and checks it played its script
/// whole and was sent nothing else.
fn assert_played_whole(process: &Process) {
    let status = process.child().wait().unwrap();
    assert!(
        status.success(),
        "the Scripted engine played its script whole"
    );
}

/// A subscriber recording `goodfetch` at `level`, with what it writes.
fn recording(level: Level) -> (impl tracing::Subscriber + Send + Sync + use<>, CapturedLog) {
    capturing(&[("goodfetch", level)])
}

/// The `log-level-follows-the-logger` client conformance case, played by
/// the Scripted engine's Daemon: `hello` says the `debug` the program's
/// subscriber records, two starts at that level send no
/// `connection.set_log_level`, and the start after the program's
/// subscriber changed to `warn` sends it once, before its `run.start`.
/// Covers: story:11-26
#[test]
fn a_start_tells_the_engine_the_level_the_subscriber_records_only_when_it_changed() {
    let env = TestEnv::new();
    let runtime = goodfetch_test_support::real_time_runtime();
    let (at_debug, _) = recording(Level::DEBUG);
    let (at_warn, _) = recording(Level::WARN);

    let process = with_default(at_debug, || {
        let hello = hello_at_the_recorded_level();
        assert_eq!(hello.log_level, Some(LogLevel::Debug));
        let (process, endpoint) =
            scripted_daemon_hearing(&env, "run/log-level-follows-the-logger.json", &hello);
        let connection = runtime.block_on(connect(endpoint, hello));
        runtime.block_on(async {
            start_and_finish(&connection, "first").await;
            start_and_finish(&connection, "second").await;
        });
        with_default(at_warn, || {
            runtime.block_on(start_and_finish(&connection, "third"));
        });
        runtime.block_on(connection.close()).unwrap();
        process
    });

    assert_played_whole(&process);
}

/// The `log-level-follows-the-logger` client conformance case's refusal,
/// played by the Scripted engine's Daemon: the engine refuses the `warn`
/// a start tells it, the start goes on and the crate warns under
/// `goodfetch::client`, and the next start tells the engine again, which
/// takes it.
/// Covers: story:11-26
#[test]
fn a_refused_level_lets_the_start_go_on_and_is_told_again_at_the_next_start() {
    let env = TestEnv::new();
    let runtime = goodfetch_test_support::real_time_runtime();
    let (at_debug, _) = recording(Level::DEBUG);
    let (at_warn, warned) = recording(Level::WARN);

    let process = with_default(at_debug, || {
        let hello = hello_at_the_recorded_level();
        let (process, endpoint) =
            scripted_daemon_hearing(&env, "run/log-level-refused.json", &hello);
        let connection = runtime.block_on(connect(endpoint, hello));
        with_default(at_warn, || {
            runtime.block_on(async {
                start_and_finish(&connection, "first").await;
                start_and_finish(&connection, "second").await;
            });
        });
        runtime.block_on(connection.close()).unwrap();
        process
    });

    let warned = String::from_utf8(warned.written()).unwrap();
    let refusals: Vec<&str> = warned
        .lines()
        .filter(|line| line.contains(&format!(" {}:", logging::CLIENT)))
        .collect();
    assert_eq!(refusals.len(), 1, "{warned}");
    assert!(refusals[0].contains("WARN"), "{warned}");
    assert!(refusals[0].contains(r#"log_level="warn""#), "{warned}");
    assert_played_whole(&process);
}

/// The `engine-logs-reach-the-logger` client conformance case, played by
/// the Scripted engine's Daemon: `hello` says the `trace` the program's
/// subscriber records, and each `$/log` event the engine sends reaches
/// the subscriber at its level, under `goodfetch::engine` followed by its
/// target past `goodfetch`, with the engine's target, its fields and how
/// many events the engine dropped before it.
/// Covers: story:11-25
#[test]
fn each_engine_log_event_reaches_the_subscriber_under_goodfetch_engine() {
    let env = TestEnv::new();
    let runtime = goodfetch_test_support::real_time_runtime();
    let (at_trace, log) = recording(Level::TRACE);

    let process = with_default(at_trace, || {
        let hello = hello_at_the_recorded_level();
        assert_eq!(hello.log_level, Some(LogLevel::Trace));
        let (process, endpoint) =
            scripted_daemon_hearing(&env, "connect/engine-logs-reach-the-logger.json", &hello);
        runtime.block_on(async {
            let connection = connect(endpoint, hello).await;
            let mut waited = 0;
            while relayed(&log).len() < 5 && waited < 1000 {
                tokio::time::sleep(Duration::from_millis(10)).await;
                waited += 1;
            }
            connection.close().await.unwrap();
        });
        process
    });

    let relayed = relayed(&log);
    let expected = [
        (
            "ERROR",
            "goodfetch::engine::rpc::dispatch",
            "goodfetch::rpc::dispatch",
            r#"fields={"operation":"run.start"}"#,
        ),
        (
            "WARN",
            "goodfetch::engine::page",
            "goodfetch::page",
            "dropped=3",
        ),
        ("INFO", "goodfetch::engine::run", "goodfetch::run", ""),
        (
            "DEBUG",
            "goodfetch::engine::rpc::dispatch",
            "goodfetch::rpc::dispatch",
            r#"fields={"id":7,"method":"run.start"}"#,
        ),
        (
            "TRACE",
            "goodfetch::engine::page::cdp",
            "goodfetch::page::cdp",
            "",
        ),
    ];
    assert_eq!(relayed.len(), expected.len(), "{relayed:#?}");
    for (line, (level, target, engine_target, detail)) in relayed.iter().zip(expected) {
        assert!(line.contains(&format!("{level} {target}:")), "{line}");
        assert!(
            line.contains(&format!(r#"target="{engine_target}""#)),
            "{line}"
        );
        assert!(line.contains(detail), "{line}");
    }
    assert!(!relayed[0].contains("dropped="), "{}", relayed[0]);
    assert!(!relayed[2].contains("fields="), "{}", relayed[2]);
    assert_played_whole(&process);
}

/// The lines `log` holds under `goodfetch::engine`, in order.
fn relayed(log: &CapturedLog) -> Vec<String> {
    String::from_utf8(log.written())
        .unwrap()
        .lines()
        .filter(|line| line.contains(&format!(" {}", logging::ENGINE)))
        .map(str::to_owned)
        .collect()
}
