//! Cancellation (ADR 0080), through the client crate: the corpus's script
//! of the `cancellation-reaches-the-engine` client conformance case, with
//! the Scripted engine as the Daemon. It fails on a message it does not
//! expect and on a step left unused, so its clean exit says each cancelled
//! click reached the engine as one `$/cancel` naming it, and that no click
//! was sent again or undone. This is Rust's `daemon` mode play.

use std::time::Duration;

use goodfetch_collie_client::transport::{Endpoint, Network};
use goodfetch_collie_client::{ClientConnection, ConnectionRuntime, Error, ErrorCode};
use goodfetch_conformance::{ClientCaseRun, RustMode};
use goodfetch_protocol::connection::DaemonEndpoint;
use goodfetch_test_support::{TestEnv, case_logs};

use crate::scripted_daemon::{daemon_hello, scripted_daemon};

/// How long a click waits before the program drops it or its timer ends.
const SOON: Duration = Duration::from_millis(50);

/// Longer than the test runs: the end that never comes first.
const NEVER: Duration = Duration::from_secs(600);

/// Plays the script's calls on a Connection to the Daemon at `endpoint`,
/// and answers the error of the click whose timer ended.
async fn cancelled_clicks(endpoint: DaemonEndpoint) -> Error {
    let (connection, _said) = ClientConnection::open(
        ConnectionRuntime::driven(tokio::runtime::Handle::current()),
        &Network::Os,
        &Endpoint::Daemon(endpoint),
        daemon_hello(),
    )
    .await
    .unwrap();
    let run = connection.start().await.unwrap();
    let page = run.new_page().await.unwrap();
    let buy = page.locator("#buy");
    // The program drops the first click once it is sent.
    tokio::select! {
        clicked = buy.click().no_timeout() => panic!("the engine answers only its cancel: {clicked:?}"),
        () = tokio::time::sleep(SOON) => {}
    }
    // The second click's timer ends before the program would drop it.
    let timed_out = tokio::select! {
        clicked = buy.click().timeout(SOON) => clicked.unwrap_err(),
        () = tokio::time::sleep(NEVER) => panic!("the click's timer ends first"),
    };
    // The program drops the third click before its timer ends.
    tokio::select! {
        clicked = buy.click().timeout(NEVER) => panic!("the engine answers only its cancel: {clicked:?}"),
        () = tokio::time::sleep(SOON) => {}
    }
    run.finish().await.unwrap();
    connection.close().await.unwrap();
    timed_out
}

/// The `cancellation-reaches-the-engine` client conformance case, played
/// by the Scripted engine's Daemon: a click whose future the program drops
/// once it is sent is cancelled with `$/cancel` naming it, without waiting,
/// and the engine's work for it ends with `call_cancelled`, which the
/// program never sees; with a timeout and a drop both set, whichever comes
/// first ends the click, and a timer of the client's own that ends first
/// fails it with `deadline_exceeded` and cancels it too. The clicks already
/// sent are neither sent again nor undone, and the Run's finish follows.
/// Covers: story:11-47
#[test]
fn a_dropped_or_timed_out_click_is_cancelled_once_by_its_id_and_never_sent_again() {
    let _case = ClientCaseRun::start(
        case_logs().as_deref(),
        "cancellation-reaches-the-engine",
        RustMode::Daemon,
    )
    .unwrap();
    let env = TestEnv::new();
    let (process, endpoint) =
        scripted_daemon(&env, "sdk-rust/run/cancellation-reaches-the-engine.json");
    let timed_out =
        goodfetch_test_support::real_time_runtime().block_on(cancelled_clicks(endpoint));

    assert_eq!(
        timed_out.code(),
        ErrorCode::DEADLINE_EXCEEDED,
        "{timed_out:?}"
    );
    assert!(
        process.child().wait().unwrap().success(),
        "the Scripted engine played its script whole"
    );
}
