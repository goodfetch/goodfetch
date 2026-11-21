//! A `cdp_url` the engine cannot use, one remedy per code (ADR 0115),
//! through the client crate: the corpus's script of the `cdp-url-errors`
//! client conformance case, which the TypeScript SDK plays too, with the
//! Scripted engine as the Daemon. Connecting and starting the Run succeed;
//! each opening of the default Session fails with its code. The script
//! fails on a message it does not expect and on a step left unused, so its
//! clean exit says the crate sent exactly what it holds. This is Rust's
//! `daemon` mode play.

use goodfetch_collie_client::transport::{Endpoint, Network};
use goodfetch_collie_client::{
    ClientConnection, ConnectionRuntime, Error, ErrorCode, ErrorDetails,
};
use goodfetch_protocol::catalog::ErrorGroup;
use goodfetch_protocol::connection::DaemonEndpoint;
use goodfetch_protocol::egress::CdpUrlNotAllowedDetails;
use goodfetch_protocol::page::cdp_url::{
    CdpUrlNotBrowserDetails, CdpUrlUnauthorizedDetails, CdpUrlUnreachableDetails,
};
use goodfetch_test_support::TestEnv;
use serde::Serialize;
use serde_json::{Value, json};

use crate::scripted_daemon::{daemon_hello, scripted_daemon};
use crate::start::failure;

/// What each opening of the Session failed with, in the script's order.
struct Refused {
    /// Nothing answered at the address.
    unreachable: Error,
    /// A page target's address answered, not a browser's.
    not_browser: Error,
    /// The browser's service answered 401.
    unauthorized: Error,
    /// A loopback address from a model's project configuration.
    not_allowed: Error,
}

/// Plays the script's calls on a Connection to the Daemon at `endpoint`.
async fn refused_pages(endpoint: DaemonEndpoint) -> Refused {
    let (connection, _said) = ClientConnection::open(
        ConnectionRuntime::driven(tokio::runtime::Handle::current()),
        &Network::Os,
        &Endpoint::Daemon(endpoint),
        daemon_hello(),
    )
    .await
    .unwrap();
    let run = connection.start().await.unwrap();
    let unreachable = failure(run.new_page().await);
    let not_browser = failure(run.new_page().await);
    let unauthorized = failure(run.new_page().await);
    let not_allowed = failure(run.new_page().await);
    run.finish().await.unwrap();
    connection.close().await.unwrap();
    Refused {
        unreachable,
        not_browser,
        unauthorized,
        not_allowed,
    }
}

/// Asserts `error`'s code, group, retryable flag and fix, and that its
/// details, read as `D`, are `details` whole.
fn assert_refused<D: ErrorDetails + Serialize>(
    error: &Error,
    (code, group, retryable): (ErrorCode, ErrorGroup, bool),
    details: &Value,
) {
    assert_eq!(
        (error.code(), error.group(), error.retryable()),
        (code, group, retryable)
    );
    assert_eq!(error.fix(), details["fix"].as_str());
    let read = error.details::<D>().unwrap();
    assert_eq!(&serde_json::to_value(read).unwrap(), details);
}

/// The `cdp-url-errors` client conformance case, played by the Scripted
/// engine's Daemon: `connect()` and the Run's start succeed, and each
/// opening of the default Session fails with its `cdp_url` code, group,
/// retryable flag, fix and details, naming the address and the settings
/// layer that gave it.
/// Covers: story:11-294
#[test]
fn each_cdp_url_code_fails_the_sessions_opening_with_its_group_fix_and_details() {
    let env = TestEnv::new();
    let (process, endpoint) = scripted_daemon(&env, "run/cdp-url-errors.json");
    let refused = goodfetch_test_support::real_time_runtime().block_on(refused_pages(endpoint));

    assert_refused::<CdpUrlUnreachableDetails>(
        &refused.unreachable,
        (
            ErrorCode::CDP_URL_UNREACHABLE,
            ErrorGroup::Unavailable,
            true,
        ),
        &json!({
            "url": "http://127.0.0.1:9222",
            "layer": "user_configuration",
            "failure": "connection_refused",
            "fix": "Start the browser with remote debugging on that port, or correct the address",
        }),
    );
    assert_refused::<CdpUrlNotBrowserDetails>(
        &refused.not_browser,
        (ErrorCode::CDP_URL_NOT_BROWSER, ErrorGroup::Program, false),
        &json!({
            "url": "ws://127.0.0.1:9222/devtools/page/8C5F0D1E",
            "layer": "user_configuration",
            "observed": "a CDP target of type `page`",
            "browser_url": "ws://127.0.0.1:9222/devtools/browser/1A2B3C4D",
            "fix": "Give the browser's endpoint, ws://127.0.0.1:9222/devtools/browser/1A2B3C4D, which this host's /json/version lists",
        }),
    );
    assert_refused::<CdpUrlUnauthorizedDetails>(
        &refused.unauthorized,
        (ErrorCode::CDP_URL_UNAUTHORIZED, ErrorGroup::Refused, false),
        &json!({
            "url": "https://browser.example.com/",
            "layer": "user_configuration",
            "status": 401,
            "headers": {"Authorization": "env:BROWSER_TOKEN"},
            "fix": "Check the Secret references in `headers`",
        }),
    );
    assert_refused::<CdpUrlNotAllowedDetails>(
        &refused.not_allowed,
        (ErrorCode::CDP_URL_NOT_ALLOWED, ErrorGroup::Refused, false),
        &json!({
            "url": "http://127.0.0.1:9222",
            "layer": "project_configuration",
            "rule": "loopback",
            "address": "127.0.0.1",
            "fix": "Set the address in Operator policy or in user configuration, or give a public address",
        }),
    );
    assert!(
        process.child().wait().unwrap().success(),
        "the Scripted engine played its script whole"
    );
}
