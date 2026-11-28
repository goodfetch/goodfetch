#![expect(
    clippy::disallowed_methods,
    reason = "unit tests create temporary Profile directories, policy files, and DevToolsActivePort on disk"
)]

use std::time::Duration;

use goodfetch_protocol::page::session::{
    REMOTE_DEBUGGING_DISABLED_BY_POLICY, RUNNING_BROWSER_UNSUPPORTED,
};
use goodfetch_protocol::settings::session_source::BrowserName;
use serde_json::json;
use tempfile::tempdir;
use tokio_util::sync::CancellationToken;

use super::{
    check_browser_supported, check_remote_debugging_policy, parse_active_port, watch_active_port,
};

#[test]
fn parses_valid_devtools_active_port_and_rejects_incomplete_content() {
    assert_eq!(
        parse_active_port("9222\n/devtools/browser/abc-123\n"),
        Some("ws://127.0.0.1:9222/devtools/browser/abc-123".to_owned())
    );
    assert_eq!(parse_active_port("9222\n"), None);
    assert_eq!(parse_active_port("0\n/devtools/browser/abc\n"), None);
    assert_eq!(
        parse_active_port("not-a-port\n/devtools/browser/abc\n"),
        None
    );
    assert_eq!(parse_active_port("9222\nno-leading-slash\n"), None);
}

#[test]
fn supported_and_unsupported_browsers_match_probe_findings() {
    assert!(check_browser_supported(BrowserName::Chrome).is_ok());
    assert!(check_browser_supported(BrowserName::Edge).is_ok());
    assert!(check_browser_supported(BrowserName::Chromium).is_ok());

    for unsupported in [
        BrowserName::Firefox,
        BrowserName::Safari,
        BrowserName::Brave,
    ] {
        let err = check_browser_supported(unsupported).unwrap_err();
        assert_eq!(err.code(), RUNNING_BROWSER_UNSUPPORTED.name);
    }
}

#[test]
fn enterprise_policy_files_raise_remote_debugging_disabled_by_policy() {
    let dir = tempdir().unwrap();
    let managed = dir.path().join("policies").join("managed");
    std::fs::create_dir_all(&managed).unwrap();

    // Allowed by default when no policy restricts it.
    assert!(check_remote_debugging_policy(BrowserName::Chrome, dir.path()).is_ok());

    // RemoteDebuggingAllowed = false refuses with remote_debugging_disabled_by_policy.
    std::fs::write(
        managed.join("corp.json"),
        json!({ "RemoteDebuggingAllowed": false }).to_string(),
    )
    .unwrap();
    let err = check_remote_debugging_policy(BrowserName::Chrome, dir.path()).unwrap_err();
    assert_eq!(err.code(), REMOTE_DEBUGGING_DISABLED_BY_POLICY.name);

    // DeveloperToolsAvailability = 2 also refuses.
    std::fs::write(
        managed.join("corp.json"),
        json!({ "DeveloperToolsAvailability": 2 }).to_string(),
    )
    .unwrap();
    let err = check_remote_debugging_policy(BrowserName::Edge, dir.path()).unwrap_err();
    assert_eq!(err.code(), REMOTE_DEBUGGING_DISABLED_BY_POLICY.name);
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn os_file_watcher_notices_devtools_active_port_appearing_without_polling() {
    goodfetch_test_support::runtime().block_on(async {
        let dir = tempdir().unwrap();
        let data_dir = dir.path().to_path_buf();
        let port_file = data_dir.join("DevToolsActivePort");
        let stopping = CancellationToken::new();

        let writer_file = port_file.clone();
        let writer = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(80)).await;
            // First write a partial file (port only), then complete it.
            std::fs::write(&writer_file, "9333\n").unwrap();
            tokio::time::sleep(Duration::from_millis(40)).await;
            std::fs::write(&writer_file, "9333\n/devtools/browser/watched-1\n").unwrap();
        });

        let url = watch_active_port(
            &data_dir,
            BrowserName::Chrome,
            goodfetch_protocol::settings::SettingsLayer::Run,
            Duration::from_secs(5),
            stopping,
        )
        .await
        .unwrap();
        writer.await.unwrap();

        assert_eq!(url, "ws://127.0.0.1:9333/devtools/browser/watched-1");
    });
}
