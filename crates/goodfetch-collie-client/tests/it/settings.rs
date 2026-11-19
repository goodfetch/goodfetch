//! A scope's own settings through the client crate (ADR 0083), with the
//! Scripted engine as the Daemon playing the script the Python SDK plays
//! too: a Run's start, a Session's opening and a page's each send their
//! own typed `Settings`, nested by segment, a duration in whole
//! milliseconds and a cleared setting as null, and a key the scope does not
//! set fails the call before anything is sent, naming where it is set.

use std::time::Duration;

use goodfetch_collie_client::settings::{Settings, browser, checkpoint, content, timeout};
use goodfetch_collie_client::transport::{Endpoint, Network};
use goodfetch_collie_client::{ClientConnection, ConnectionRuntime, ErrorCode};
use goodfetch_protocol::settings::SettingNotApplicableDetails;
use goodfetch_test_support::TestEnv;

use crate::scripted_daemon::{daemon_hello, scripted_daemon};

/// The script's start, Session and page, each with its own settings; the
/// Connection's close aborts the Run.
const SCRIPT: &str = "sdk-python/run/settings-sent-by-scope.json";

#[test]
fn each_scope_sends_its_own_settings_and_a_key_it_does_not_set_is_refused_unsent() {
    let env = TestEnv::new();
    let (process, endpoint) = scripted_daemon(&env, SCRIPT);
    let runtime = goodfetch_test_support::real_time_runtime();
    runtime.block_on(async {
        let (connection, _said) = ClientConnection::open(
            ConnectionRuntime::driven(tokio::runtime::Handle::current()),
            &Network::Os,
            &Endpoint::Daemon(endpoint),
            daemon_hello(),
        )
        .await
        .unwrap();

        let refused = connection
            .start()
            .settings(Settings::new().set(timeout::RUN, Duration::from_secs(60)))
            .await;
        let run = connection
            .start()
            .settings(
                Settings::new()
                    .set(timeout::NAVIGATION, Duration::from_secs(30))
                    .set(checkpoint::INTERVAL, None),
            )
            .await
            .unwrap();
        let session = run
            .new_session()
            .settings(Settings::new().set(browser::ACCESS_MODE, browser::AccessMode::Stealth))
            .await
            .unwrap();
        let page = session
            .new_page()
            .settings(Settings::new().set(content::EXTRACTOR, content::Extractor::Trafilatura))
            .await
            .unwrap();
        connection.close().await.unwrap();
        drop((page, session, run));

        let Err(failure) = &refused else {
            panic!("{refused:?}");
        };
        let details: SettingNotApplicableDetails = failure.details().unwrap();
        assert_eq!(
            (failure.code(), details.key.as_str(), details.fix.as_str()),
            (
                ErrorCode::SETTING_NOT_APPLICABLE,
                "timeout.run",
                "set it in the settings of the connection",
            )
        );
    });
    let status = process.child().wait().unwrap();
    assert!(
        status.success(),
        "the Scripted engine played its script whole"
    );
}
