//! The engine's logs through the Rust SDK (ADR 0078), the `logging` client
//! conformance cases in `linked` mode: the linked engine's own events reach
//! the program's subscriber directly, under their own targets, and a start
//! tells the engine the level the program's subscriber records for
//! `goodfetch::engine` only when it changed. Each test installs the
//! process's global subscriber, which the linked engine's threads log to,
//! so each runs in a process of its own, as `just test` runs every test.

use goodfetch_collie::{Collie, RunId};
use goodfetch_conformance::{ClientCaseRun, RustMode};
use goodfetch_test_support::{CapturedLog, TestEnv, capturing, case_logs, real_time_runtime};
use tracing::Level;
use tracing::subscriber::with_default;

use super::options;

/// Installs, as the process's global subscriber, one that records the
/// engine's own events, the SDK's and those under `goodfetch::engine` at
/// every level, and returns what it writes.
fn global_log() -> CapturedLog {
    let (subscriber, log) = capturing(&[
        ("goodfetch::rpc", Level::TRACE),
        ("goodfetch::client", Level::TRACE),
        ("goodfetch::engine", Level::TRACE),
    ]);
    tracing::subscriber::set_global_default(subscriber).unwrap();
    log
}

/// The lines of `log` that hold `text`.
fn lines_with(log: &CapturedLog, text: &str) -> Vec<String> {
    String::from_utf8(log.written())
        .unwrap()
        .lines()
        .filter(|line| line.contains(text))
        .map(str::to_owned)
        .collect()
}

/// The `engine-logs-reach-the-logger` client conformance case in `linked`
/// mode: the linked engine's events reach the program's subscriber under
/// their own targets, and none is re-emitted under `goodfetch::engine`,
/// though the subscriber records it at every level.
/// Covers: story:11-25
#[test]
fn the_linked_engines_own_events_reach_the_programs_subscriber_once() {
    let mut case = ClientCaseRun::start(
        case_logs().as_deref(),
        "engine-logs-reach-the-logger",
        RustMode::Linked,
    )
    .unwrap();
    let env = TestEnv::new();
    let log = global_log();
    let first = RunId::parse("first").unwrap();

    real_time_runtime().block_on(async {
        let collie = Collie::connect(options(&env)).await.unwrap();
        collie
            .start()
            .run_id(&first)
            .await
            .unwrap()
            .finish()
            .await
            .unwrap();
        collie.close().await.unwrap();
    });

    let answered = lines_with(&log, "goodfetch::rpc::dispatch: request answered");
    case.check("the engine's events reach the subscriber", || {
        assert!(
            answered
                .iter()
                .any(|line| line.contains("method=\"run.start\"")),
            "{answered:#?}"
        );
    });
    case.check("none is re-emitted under goodfetch::engine", || {
        assert_eq!(lines_with(&log, " goodfetch::engine"), Vec::<String>::new());
    });
}

/// The `log-level-follows-the-logger` client conformance case in `linked`
/// mode: the program's subscriber records `goodfetch::engine` at `debug`
/// while it connects and starts two Runs, which tell the linked engine
/// nothing more, then at `warn`, which the third start tells it once.
/// Covers: story:11-26
#[test]
fn a_start_tells_the_linked_engine_the_level_only_when_it_changed() {
    let mut case = ClientCaseRun::start(
        case_logs().as_deref(),
        "log-level-follows-the-logger",
        RustMode::Linked,
    )
    .unwrap();
    let env = TestEnv::new();
    let log = global_log();
    let runtime = real_time_runtime();
    let (at_debug, _) = capturing(&[("goodfetch::engine", Level::DEBUG)]);
    let (at_warn, _) = capturing(&[("goodfetch::engine", Level::WARN)]);
    let start_and_finish = async |collie: &Collie, run_id: &str| {
        let run_id = RunId::parse(run_id).unwrap();
        collie
            .start()
            .run_id(&run_id)
            .await
            .unwrap()
            .finish()
            .await
            .unwrap();
    };

    with_default(at_debug, || {
        let collie = runtime.block_on(Collie::connect(options(&env))).unwrap();
        runtime.block_on(async {
            start_and_finish(&collie, "first").await;
            start_and_finish(&collie, "second").await;
        });
        with_default(at_warn, || {
            runtime.block_on(start_and_finish(&collie, "third"));
        });
        runtime.block_on(collie.close()).unwrap();
    });

    let answered: Vec<String> = lines_with(&log, "goodfetch::rpc::dispatch: request answered")
        .into_iter()
        .filter(|line| line.contains("connection.set_log_level") || line.contains("run.start"))
        .collect();
    let methods: Vec<&str> = answered
        .iter()
        .map(|line| {
            if line.contains("connection.set_log_level") {
                "connection.set_log_level"
            } else {
                "run.start"
            }
        })
        .collect();
    case.check("only the start after the level changed tells it", || {
        assert_eq!(
            methods,
            [
                "run.start",
                "run.start",
                "connection.set_log_level",
                "run.start"
            ],
            "{answered:#?}"
        );
    });
}

/// The SDK installs no subscriber: after a program with none connects,
/// starts a Run and closes, none was ever set in the process, so what it
/// and the linked engine log goes nowhere, not to stdout or stderr.
/// Covers: story:11-25
#[test]
fn the_sdk_installs_no_subscriber() {
    let env = TestEnv::new();
    let first = RunId::parse("first").unwrap();

    real_time_runtime().block_on(async {
        let collie = Collie::connect(options(&env)).await.unwrap();
        collie
            .start()
            .run_id(&first)
            .await
            .unwrap()
            .finish()
            .await
            .unwrap();
        collie.close().await.unwrap();
    });

    assert!(!tracing::dispatcher::has_been_set());
}
