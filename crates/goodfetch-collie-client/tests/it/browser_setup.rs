//! Browser setup as typed settings where a Session opens (ADRs 0083, 0085
//! and 0121; issue 1305): the `session-browser-setup-settings` client
//! conformance case on the Scripted engine as the Daemon. Sends all 12 browser
//! setup keys in `run.new_session().settings(...)`, and refuses a Run-scope
//! key (`timeout.navigation`, `session.source`) at `run.new_session` and a
//! Session-scope key (`browser.viewport`) at `session.new_page` with
//! `setting_not_applicable` before sending anything.

use std::time::Duration;

use goodfetch_collie_client::settings::{Settings, browser, network, session, timeout};
use goodfetch_collie_client::transport::{Endpoint, Network};
use goodfetch_collie_client::{ClientConnection, ConnectionRuntime, ErrorCode};
use goodfetch_conformance::{ClientCaseRun, RustMode};
use goodfetch_protocol::page::emulation::{
    DeviceScaleFactor, Geolocation, NetworkThrottle, Viewport,
};
use goodfetch_protocol::settings::SettingNotApplicableDetails;
use goodfetch_test_support::{TestEnv, case_logs};

use crate::scripted_daemon::{daemon_hello, scripted_daemon};
use crate::start::failure;

const SCRIPT: &str = "sdk-rust/run/session-browser-setup-settings.json";

fn all_twelve_settings() -> Settings {
    Settings::new()
        .set(
            browser::VIEWPORT,
            Some(Viewport {
                width: 800,
                height: 600,
            }),
        )
        .set(
            browser::DEVICE_SCALE_FACTOR,
            DeviceScaleFactor::new(1.5).unwrap(),
        )
        .set(browser::LOCALE, Some("fr-FR".to_owned()))
        .set(browser::TIMEZONE_ID, Some("Europe/Paris".to_owned()))
        .set(browser::COLOR_SCHEME, browser::ColorScheme::Dark)
        .set(browser::JAVA_SCRIPT_ENABLED, false)
        .set(
            browser::GEOLOCATION,
            Some(Geolocation {
                latitude: 48.8566,
                longitude: 2.3522,
                accuracy: Some(10.5),
            }),
        )
        .set(
            browser::PERMISSIONS,
            vec![browser::PermissionsItem::Geolocation],
        )
        .set(browser::HEADLESS, true)
        .set(
            browser::BLOCK_RESOURCES,
            vec![browser::BlockResourcesItem::Image],
        )
        .set(browser::SERVICE_WORKERS, browser::ServiceWorkers::Block)
        .set(
            network::THROTTLE,
            Some(NetworkThrottle {
                offline: false,
                latency: 40,
                download_bytes_per_second: Some(500_000),
                upload_bytes_per_second: Some(250_000),
            }),
        )
}

/// The `session-browser-setup-settings` client conformance case, played by
/// the Scripted engine's Daemon: all 12 browser setup keys are accepted where
/// a Session opens and serialize in their wire shape; Run-scope keys at
/// `run.new_session` and Session-scope keys at `session.new_page` raise
/// `setting_not_applicable` before sending.
/// Covers: story:11-89, setting:browser.viewport, setting:browser.device_scale_factor, setting:browser.locale, setting:browser.timezone_id, setting:browser.color_scheme, setting:browser.java_script_enabled, setting:browser.geolocation, setting:browser.permissions, setting:browser.headless, setting:browser.block_resources, setting:browser.service_workers, setting:network.throttle
#[expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]
#[test]
fn session_browser_setup_settings_and_scope_refusals() {
    let mut case = ClientCaseRun::start(
        case_logs().as_deref(),
        "session-browser-setup-settings",
        RustMode::Daemon,
    )
    .unwrap();
    let env = TestEnv::new();
    let (process, endpoint) = scripted_daemon(&env, SCRIPT);
    let runtime = goodfetch_test_support::real_time_runtime();
    let (refused_run_key, refused_source_key, refused_page_key) = runtime.block_on(async {
        let (connection, _said) = ClientConnection::open(
            ConnectionRuntime::driven(tokio::runtime::Handle::current()),
            &Network::Os,
            &Endpoint::Daemon(endpoint),
            daemon_hello(),
        )
        .await
        .unwrap();
        let run = connection.start().await.unwrap();

        let refused_run_key = failure(
            run.new_session()
                .settings(Settings::new().set(timeout::NAVIGATION, Duration::from_secs(30)))
                .await,
        );
        let refused_source_key = failure(
            run.new_session()
                .settings(Settings::new().set(session::SOURCE, None))
                .await,
        );

        let opened = run
            .new_session()
            .settings(all_twelve_settings())
            .await
            .unwrap();

        let refused_page_key = failure(
            opened
                .new_page()
                .settings(Settings::new().set(
                    browser::VIEWPORT,
                    Some(Viewport {
                        width: 800,
                        height: 600,
                    }),
                ))
                .await,
        );

        opened.close().await.unwrap();
        run.finish().await.unwrap();
        connection.close().await.unwrap();
        (refused_run_key, refused_source_key, refused_page_key)
    });
    let status = process.child().wait().unwrap();

    case.check("the Scripted engine played its script whole", || {
        assert!(status.success(), "scripted daemon exited with {status}");
    });
    case.check(
        "a Run-scope key at run.new_session raises setting_not_applicable unsent",
        || {
            assert_eq!(refused_run_key.code(), ErrorCode::SETTING_NOT_APPLICABLE);
            let details: SettingNotApplicableDetails = refused_run_key.details().unwrap();
            assert_eq!(details.key, "timeout.navigation");

            assert_eq!(refused_source_key.code(), ErrorCode::SETTING_NOT_APPLICABLE);
            let source_details: SettingNotApplicableDetails = refused_source_key.details().unwrap();
            assert_eq!(source_details.key, "session.source");
        },
    );
    case.check(
        "a Session-scope browser key at session.new_page raises setting_not_applicable unsent",
        || {
            assert_eq!(refused_page_key.code(), ErrorCode::SETTING_NOT_APPLICABLE);
            let details: SettingNotApplicableDetails = refused_page_key.details().unwrap();
            assert_eq!(details.key, "browser.viewport");
        },
    );
}
