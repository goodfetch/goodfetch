//! Tests that other tests run as child processes, to watch what a test
//! using `TestEnv` shows when it hangs or fails, and to stand in for an
//! engine speaking its protocol. They are ignored, so they run only as those
//! children.

use std::time::Duration;

use goodfetch_test_support::{ObservationLog, Operation, Record, Redacted, TestEnv};

#[test]
#[ignore = "run as a child by seed::seed_line_is_printed_before_the_test_body_runs"]
fn hangs_after_creating_a_test_env() {
    let _env = TestEnv::new();

    // The parent kills it long before this ends.
    std::thread::sleep(Duration::from_secs(600));
}

#[test]
#[ignore = "run as a child by teardown::a_kept_folder_is_listed_on_failure"]
fn fails_with_a_test_env() {
    let _env = TestEnv::new();

    panic!("the probe fails on purpose");
}

#[test]
#[ignore = "run as a child by seeded_kill's tests"]
fn sends_protocol_events_then_hangs() {
    for n in 1..=3 {
        for method in ["delivery.begin", "delivery.commit"] {
            let message =
                serde_json::json!({ "jsonrpc": "2.0", "method": method, "params": { "n": n } });
            #[expect(
                clippy::print_stdout,
                reason = "the probe speaks newline-delimited JSON-RPC on its stdout, as an engine does"
            )]
            {
                println!("{message}");
            }
        }
    }

    // The parent kills it long before this ends.
    std::thread::sleep(Duration::from_secs(600));
}

#[test]
#[ignore = "run as a child by exercise_record::the_test_subscriber_writes_the_dispatch_records_into_the_exercise_record"]
fn dispatches_under_the_test_subscriber() {
    let start = TestEnv::new().start_time();
    let subscriber = goodfetch_test_support::subscriber(move || start);

    tracing::subscriber::with_default(subscriber, || {
        tracing::info!(target: "goodfetch::engine", "engine started");
        tracing::trace!(target: "goodfetch::rpc::dispatch", method = "engine.info", "request answered");
    });
}

/// A test's own operation, carrying a security value.
#[derive(serde::Serialize)]
struct SignIn {
    token: Redacted<String>,
}

#[test]
#[ignore = "run as a child by observation_log::a_failed_test_writes_its_observation_log_in_its_kept_folder"]
fn fails_with_an_observation_log() {
    let env = TestEnv::new();
    let mut log = ObservationLog::for_test(&env);
    log.push(Record::new("engine", Operation::Fence { epoch: 1 }));
    log.push(Record::new(
        "client",
        Operation::Other(SignIn {
            token: Redacted::new("gf_token_5f1e".to_owned()),
        }),
    ));

    panic!("the probe fails on purpose");
}
