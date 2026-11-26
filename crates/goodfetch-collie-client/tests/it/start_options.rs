//! A start's options through the client crate (ADRs 0033, 0080, 0091,
//! 0102 and 0105), with the Scripted engine as the Daemon playing the
//! crate's copy of the corpus's script of the `start-takes-every-option`
//! client conformance case: a start sends its settings, its `timeout`, its
//! Plan file, its Response recording and each Plugin change it accepts, and
//! a resume honours them; `.no_timeout()` sends `null`; a zero `timeout` is
//! refused before anything is sent; and a start the program drops once it
//! is sent is given up with no `$/cancel`, since the catalog marks
//! `run.start` not cancellable, where TypeScript's and Python's copies send
//! one the engine may ignore. The Daemon checks every message the crate
//! sends and fails on
//! a step left unused, so its clean exit says each start carried exactly
//! what the script holds.
//! Covers: story:11-50, story:11-132

use std::time::Duration;

use goodfetch_collie_client::settings::{Settings, timeout};
use goodfetch_collie_client::transport::{Endpoint, Network};
use goodfetch_collie_client::{ClientConnection, ConnectionRuntime, Error, ErrorCode};
use goodfetch_conformance::{ClientCaseRun, RustMode};
use goodfetch_protocol::connection::DaemonEndpoint;
use goodfetch_protocol::run::{Recording, RecordingMode, RunId, StartedAs};
use goodfetch_test_support::{TestEnv, case_logs, real_time_runtime};
use serde_json::json;

use crate::scripted_daemon::scripted_daemon;
use crate::start::failure;

/// The crate's copy of the corpus's script of the case.
const SCRIPT: &str = "sdk-rust/run/start-takes-every-option.json";

/// How long the program waits before it drops the start it sent.
const SOON: Duration = Duration::from_millis(50);

/// What each start answered.
struct Played {
    started_as: [StartedAs; 2],
    zero: Error,
}

/// Plays the script's starts on a Connection to the Daemon at `endpoint`.
async fn play(endpoint: DaemonEndpoint) -> Played {
    let id = |text: &str| RunId::parse(text).unwrap();
    let (connection, _said) = ClientConnection::open(
        ConnectionRuntime::driven(tokio::runtime::Handle::current()),
        &Network::Os,
        &Endpoint::Daemon(endpoint),
        crate::scripted_daemon::daemon_hello(),
    )
    .await
    .unwrap();

    let resumed = connection
        .start()
        .run_id(&id("nightly"))
        .settings(Settings::new().set(timeout::WAIT, Duration::from_secs(5)))
        .timeout(Duration::from_secs(3600))
        .plan("plans/nightly.json")
        .recording(Recording {
            mode: RecordingMode::Replay,
            path: "recordings/nightly.warc".to_owned(),
        })
        .accept_plugin_change("shop-prices")
        .await
        .unwrap();
    resumed.close().await.unwrap();
    let ended = connection
        .start()
        .run_id(&id("done"))
        .no_timeout()
        .await
        .unwrap();
    ended.close().await.unwrap();
    let zero = failure(
        connection
            .start()
            .run_id(&id("never"))
            .timeout(Duration::ZERO)
            .await,
    );
    // The program drops the start once it is sent.
    tokio::select! {
        started = connection.start().run_id(&id("slow")) => panic!("the engine never answers it: {started:?}"),
        () = tokio::time::sleep(SOON) => {}
    }
    connection.close().await.unwrap();
    Played {
        started_as: [resumed.started_as(), ended.started_as()],
        zero,
    }
}

/// Each start sends every option it is given, `null` for no timeout; a
/// zero timeout sends nothing; and a dropped start is given up with no
/// `$/cancel`.
#[test]
fn a_start_sends_every_option_refuses_a_zero_timeout_and_gives_up_when_dropped() {
    let mut case = ClientCaseRun::start(
        case_logs().as_deref(),
        "start-takes-every-option",
        RustMode::Daemon,
    )
    .unwrap();
    let env = TestEnv::new();
    let (process, endpoint) = scripted_daemon(&env, SCRIPT);
    let played = real_time_runtime().block_on(play(endpoint));

    case.check("a resume honours the start's options", || {
        assert_eq!(played.started_as, [StartedAs::Resumed, StartedAs::Ended]);
    });
    case.check(
        "a zero timeout is setting_invalid from the argument, naming the null form",
        || {
            assert_eq!(
                (
                    played.zero.code(),
                    serde_json::to_value(played.zero.limit()).unwrap(),
                    played
                        .zero
                        .fix()
                        .is_some_and(|fix| fix.contains("`.no_timeout()`")),
                ),
                (
                    ErrorCode::SETTING_INVALID,
                    json!({ "key": "timeout.run", "value": 0, "layer": "run", "origin": "argument" }),
                    true,
                )
            );
        },
    );
    case.check(
        "the Daemon heard every option and no cancel, so it played its script whole",
        || assert!(process.child().wait().unwrap().success()),
    );
}
