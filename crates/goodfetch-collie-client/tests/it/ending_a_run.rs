//! How a Run ends and what it does meanwhile (ADRs 0029 and 0102), through
//! the client crate: `finish` and `stop` answer the Run report, a failed
//! Run's error fails them and a call's timeout ends only its wait, while
//! `progress` follows the Run. The `finish-stop-and-progress` client
//! conformance case, which the TypeScript and Python SDKs play too, with
//! the Scripted engine as the Daemon: it fails on a message it does not
//! expect and on a step left unused, so its clean exit says the crate sent
//! exactly what the script holds, the read after a value included.

use std::time::Duration;

use goodfetch_collie_client::{Error, ErrorCode};
use goodfetch_conformance::{ClientCaseRun, RustMode};
use goodfetch_protocol::connection::DaemonEndpoint;
use goodfetch_protocol::run::{ProgressMessage, RunId, RunProgress};
use goodfetch_test_support::{TestEnv, case_logs};
use serde_json::{Value, json};

use crate::closing::{assert_played_whole, connect};
use crate::scripted_daemon::scripted_daemon;

/// Whether each of `pointers` in `value` holds what it pairs with.
fn assert_holds(value: &Value, pointers: &[(&str, Value)]) {
    for (pointer, expected) in pointers {
        assert_eq!(
            value.pointer(pointer),
            Some(expected),
            "{pointer} of {value}"
        );
    }
}

/// What the case's program saw, in the order it called.
struct Played {
    progress: [RunProgress; 2],
    set: RunProgress,
    waited: Result<(), Error>,
    finished: Value,
    stopped: Value,
    failed: [Result<(), Error>; 2],
}

/// Plays the case's program on a Connection to the Daemon at `endpoint`.
async fn play(endpoint: DaemonEndpoint, message: &ProgressMessage) -> Played {
    let connection = connect(endpoint).await;
    let started = async |id: &str| {
        connection
            .start()
            .run_id(&RunId::parse(id).unwrap())
            .await
            .unwrap()
    };
    let nightly = started("nightly").await;
    let mut following = nightly.progress();
    let first = following.next().await.unwrap().unwrap().into_inner();
    let set = nightly.set_progress_message(message.clone()).await.unwrap();
    let second = following.next().await.unwrap().unwrap().into_inner();
    drop(following);
    let waited = nightly.finish().timeout(Duration::from_secs(60)).await;
    let finished = nightly.finish().await.unwrap();
    let stopped = started("weekly").await.stop().await.unwrap();
    let broken = started("broken").await;
    let failed = [
        broken.finish().await.map(|_| ()),
        broken.stop().await.map(|_| ()),
    ];
    connection.close().await.unwrap();
    Played {
        progress: [first, second],
        set: set.into_inner(),
        waited: waited.map(|_| ()),
        finished: serde_json::to_value(&*finished).unwrap(),
        stopped: serde_json::to_value(&*stopped).unwrap(),
        failed,
    }
}

/// The `finish-stop-and-progress` client conformance case, played by the
/// Scripted engine's Daemon.
/// Covers: story:11-128, story:11-129
#[test]
fn finish_and_stop_answer_the_report_a_failed_run_fails_them_and_progress_follows() {
    let _case = ClientCaseRun::start(
        case_logs().as_deref(),
        "finish-stop-and-progress",
        RustMode::Daemon,
    )
    .unwrap();
    let env = TestEnv::new();
    let (process, endpoint) = scripted_daemon(&env, "run/finish-stop-and-progress.json");
    let message = ProgressMessage::parse("three pages left").unwrap();
    let Played {
        progress,
        set,
        waited,
        finished,
        stopped,
        failed,
    } = goodfetch_test_support::real_time_runtime().block_on(play(endpoint, &message));

    assert_eq!(
        progress
            .iter()
            .map(|value| (value.position, value.message.clone()))
            .collect::<Vec<_>>(),
        [(1, None), (2, Some(message.clone()))],
        "the progress answers at once, then the value after it carries the program's message"
    );
    assert_eq!((set.position, set.message.clone()), (2, Some(message)));
    assert!(
        matches!(&waited, Err(error) if error.code() == ErrorCode::DEADLINE_EXCEEDED),
        "a finish's timeout fails it with deadline_exceeded, ending only the wait: {waited:?}"
    );
    assert_holds(
        &finished,
        &[
            ("/run_id", json!("nightly")),
            ("/lifecycle", json!("finished")),
            ("/verdict", json!("needs_review")),
            ("/verdict_reasons/0/code", json!("uncertain_effect")),
            ("/verdict_reasons/0/count", json!(1)),
            ("/completeness", json!("complete")),
            ("/usage/requests", json!(3)),
            (
                "/weakened",
                json!([{"key": "consent.handling", "value": "off", "layer": "run"}]),
            ),
            ("/plan/status", json!("unchanged")),
            ("/settings/0/key", json!("timeout.close")),
            ("/settings/0/value", json!(30_000)),
            ("/plugins/0/name", json!("shop-login")),
            ("/plugins/0/network_level", json!("none")),
            ("/warnings/0/code", json!("storage_low")),
            ("/recording/file", json!("nightly.recording")),
            ("/nothing_done/reason", json!("nothing_pending")),
            ("/uncertain_effects/0/entry", json!("submit-order")),
            ("/uncertain_effects/0/since", json!("2026-01-01T00:00:30Z")),
            ("/ended_at", json!("2026-01-01T00:01:00Z")),
        ],
    );
    assert_holds(
        &stopped,
        &[
            ("/lifecycle", json!("stopped")),
            ("/stop_reason", json!({"kind": "abort", "by": "program"})),
            ("/completeness", json!("incomplete")),
            ("/checkpoint/run_id", json!("weekly")),
            ("/checkpoint/journal_position", json!(7)),
            ("/checkpoint/time", json!("2026-01-01T00:00:05Z")),
        ],
    );
    assert_eq!(
        failed
            .iter()
            .map(|ended| ended.as_ref().map_err(Error::code))
            .collect::<Vec<_>>(),
        [
            Err(ErrorCode::BROWSER_LAUNCH_FAILED),
            Err(ErrorCode::BROWSER_LAUNCH_FAILED)
        ],
        "a failed Run's finish and stop fail with the error that failed it"
    );
    assert_played_whole(&process);
}
