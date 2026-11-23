//! A program's Sessions through the client crate (ADRs 0092 and 0078): the
//! corpus's script of the `sessions-open-from-their-source` client
//! conformance case, which the TypeScript and Python SDKs play too, with
//! the Scripted engine as the Daemon. `run.new_session` sends its `source`
//! beside its `settings`, each Session's pages are its own, and closing one
//! closes its pages while the others open pages still. The script fails on
//! a message it does not expect and on a step left unused, so its clean
//! exit says the crate sent exactly what it holds. This is Rust's `daemon`
//! mode play.

use goodfetch_collie_client::settings::{Settings, browser};
use goodfetch_collie_client::transport::{Endpoint, Network};
use goodfetch_collie_client::{ClientConnection, ConnectionRuntime, Error, ErrorCode};
use goodfetch_conformance::{ClientCaseRun, RustMode};
use goodfetch_protocol::catalog::ErrorGroup;
use goodfetch_protocol::connection::DaemonEndpoint;
use goodfetch_protocol::settings::session_source::SessionSource;
use goodfetch_protocol::settings::{SettingInvalidDetails, SettingsLayer};
use goodfetch_test_support::{TestEnv, case_logs};

use crate::scripted_daemon::{daemon_hello, scripted_daemon};
use crate::start::failure;

const SHOP_URL: &str = "https://shop.test/";

/// What the script's refused calls failed with, in its order.
struct Refused {
    /// The closed Session's page's `goto`, then the Session's `new_page`.
    after_close: [Error; 2],
    /// A Session whose `cdp_url` source names `www.example.com` in
    /// `domains`.
    not_registrable: Error,
}

/// Plays the script's calls on a Connection to the Daemon at `endpoint`.
async fn play(endpoint: DaemonEndpoint) -> Refused {
    let (connection, _said) = ClientConnection::open(
        ConnectionRuntime::driven(tokio::runtime::Handle::current()),
        &Network::Os,
        &Endpoint::Daemon(endpoint),
        daemon_hello(),
    )
    .await
    .unwrap();
    let run = connection.start().await.unwrap();
    let saved = run
        .new_session()
        .source(Some(SessionSource::SavedIdentity {
            name: "shop".to_owned(),
        }))
        .settings(Settings::new().set(browser::ACCESS_MODE, browser::AccessMode::Standard))
        .await
        .unwrap();
    let saved_page = saved.new_page().await.unwrap();
    let fresh = run.new_session().source(None).await.unwrap();
    // Each page is held to the Connection's close: dropping an unclosed
    // value would send its close (ADR 0078).
    let fresh_page = fresh.new_page().await.unwrap();

    saved.close().await.unwrap();
    let after_close = [
        failure(saved_page.goto(SHOP_URL).await),
        failure(saved.new_page().await),
    ];
    let fresh_second_page = fresh.new_page().await.unwrap();
    let run_page = run.new_page().await.unwrap();
    let not_registrable = failure(
        run.new_session()
            .source(Some(SessionSource::CdpUrl {
                url: "http://127.0.0.1:9222".to_owned(),
                headers: None,
                domains: Some(vec!["www.example.com".to_owned()]),
                exclusive: None,
            }))
            .await,
    );
    run.finish().await.unwrap();
    connection.close().await.unwrap();
    drop((fresh_page, fresh_second_page, run_page));
    Refused {
        after_close,
        not_registrable,
    }
}

/// The `sessions-open-from-their-source` client conformance case, played
/// by the Scripted engine's Daemon: a Session opens from a Saved identity
/// under its own Access mode and another fresh, each opening its own page;
/// the closed one's page and the Session itself refuse with `closed`, while
/// the other and the Run's default Session open pages still; a `cdp_url`
/// source whose `domains` names a host, not its registrable domain, fails
/// with `setting_invalid` whose fix names the registrable domain.
/// Covers: story:11-92
#[test]
fn sessions_open_from_their_source_and_close_with_their_pages() {
    let _case = ClientCaseRun::start(
        case_logs().as_deref(),
        "sessions-open-from-their-source",
        RustMode::Daemon,
    )
    .unwrap();
    let env = TestEnv::new();
    let (process, endpoint) = scripted_daemon(&env, "run/sessions-open-from-their-source.json");
    let refused = goodfetch_test_support::real_time_runtime().block_on(play(endpoint));

    for error in &refused.after_close {
        assert_eq!(
            (error.code(), error.group()),
            (ErrorCode::CLOSED, ErrorGroup::Program),
            "{error:?}"
        );
    }
    let error = &refused.not_registrable;
    assert_eq!(
        (error.code(), error.group(), error.retryable()),
        (ErrorCode::SETTING_INVALID, ErrorGroup::Program, false)
    );
    assert_eq!(error.fix(), Some("Give `example.com` in `domains`"));
    let details = error.details::<SettingInvalidDetails>().unwrap();
    assert_eq!(
        (details.key.as_str(), details.layer, details.reason.as_str()),
        (
            "session.source",
            Some(SettingsLayer::Run),
            "`www.example.com` in `domains` is no registrable domain, whose is `example.com`",
        )
    );
    assert!(
        process.child().wait().unwrap().success(),
        "the Scripted engine played its script whole"
    );
}
