//! Unit tests for `profile_directory` pre-launch checks (ADR 0056; ticket
//! #737): default data dir refusal, browser family mismatch, lock detection
//! (including dangling Linux `SingletonLock` symlinks), version downgrade
//! refusal, and `browser.locale` conflict (`setting_locked`).
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]
#![expect(
    clippy::disallowed_methods,
    reason = "unit tests create temporary profile directories on disk"
)]

use goodfetch_protocol::page::session::{
    PROFILE_DIRECTORY_BROWSER_MISMATCH, PROFILE_DIRECTORY_IN_USE,
    PROFILE_DIRECTORY_NEWER_THAN_BROWSER,
};
use goodfetch_protocol::settings::session_source::BrowserName;
use goodfetch_protocol::settings::{SETTING_LOCKED, SettingsLayer};

use super::inspect;

/// Covers: story:5-66, error:profile_directory_in_use
#[test]
fn refuses_locked_profile_directory_including_singleton_lock_and_lockfile() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("Local State"), "{}").unwrap();
    std::fs::write(dir.path().join("SingletonLock"), "pid").unwrap();

    let err = inspect(
        dir.path(),
        BrowserName::Chrome,
        None,
        Some(135),
        None,
        SettingsLayer::Run,
    )
    .unwrap_err();
    assert_eq!(err.code(), PROFILE_DIRECTORY_IN_USE.name);
    const {
        assert!(!PROFILE_DIRECTORY_IN_USE.retryable);
    }

    std::fs::remove_file(dir.path().join("SingletonLock")).unwrap();
    std::fs::write(dir.path().join("lockfile"), "").unwrap();
    let err2 = inspect(
        dir.path(),
        BrowserName::Chromium,
        None,
        Some(135),
        None,
        SettingsLayer::Run,
    )
    .unwrap_err();
    assert_eq!(err2.code(), PROFILE_DIRECTORY_IN_USE.name);
}

/// Covers: story:5-66, error:profile_directory_browser_mismatch
#[test]
fn refuses_browser_family_mismatch_between_firefox_and_chromium() {
    let ff = tempfile::tempdir().unwrap();
    std::fs::write(ff.path().join("compatibility.ini"), "[Compatibility]\n").unwrap();
    let err = inspect(
        ff.path(),
        BrowserName::Chrome,
        None,
        Some(135),
        None,
        SettingsLayer::Run,
    )
    .unwrap_err();
    assert_eq!(err.code(), PROFILE_DIRECTORY_BROWSER_MISMATCH.name);
    assert_eq!(err.details().unwrap()["found"], "firefox");
    const {
        assert!(!PROFILE_DIRECTORY_BROWSER_MISMATCH.retryable);
    }

    let cr = tempfile::tempdir().unwrap();
    std::fs::write(cr.path().join("Local State"), "{}").unwrap();
    let err2 = inspect(
        cr.path(),
        BrowserName::Firefox,
        None,
        Some(135),
        None,
        SettingsLayer::Run,
    )
    .unwrap_err();
    assert_eq!(err2.code(), PROFILE_DIRECTORY_BROWSER_MISMATCH.name);
    assert_eq!(err2.details().unwrap()["found"], "chromium");
}

/// Covers: story:5-66, error:profile_directory_newer_than_browser
#[test]
fn refuses_profile_written_by_newer_browser_version() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("Local State"), "{}").unwrap();
    std::fs::write(dir.path().join("Last Version"), "199.0.9999.0").unwrap();

    let err = inspect(
        dir.path(),
        BrowserName::Chrome,
        None,
        Some(135),
        None,
        SettingsLayer::Run,
    )
    .unwrap_err();
    assert_eq!(err.code(), PROFILE_DIRECTORY_NEWER_THAN_BROWSER.name);
    const {
        assert!(!PROFILE_DIRECTORY_NEWER_THAN_BROWSER.retryable);
    }
}

/// Covers: story:5-66, story:5-76, error:setting_locked
#[test]
fn refuses_conflicting_browser_locale_and_accepts_clean_profile() {
    let dir = tempfile::tempdir().unwrap();
    let default_dir = dir.path().join("Default");
    std::fs::create_dir_all(&default_dir).unwrap();
    std::fs::write(
        default_dir.join("Preferences"),
        r#"{"intl":{"accept_languages":"fr-FR,fr"}}"#,
    )
    .unwrap();
    std::fs::write(dir.path().join("Last Version"), "135.0.0.0").unwrap();

    let locked = inspect(
        dir.path(),
        BrowserName::Chrome,
        None,
        Some(135),
        Some("en-US"),
        SettingsLayer::Run,
    )
    .unwrap_err();
    assert_eq!(locked.code(), SETTING_LOCKED.name);
    assert_eq!(locked.details().unwrap()["key"], "browser.locale");

    let ok = inspect(
        dir.path(),
        BrowserName::Chrome,
        Some("Default"),
        Some(135),
        Some("fr-FR"),
        SettingsLayer::Run,
    )
    .unwrap();
    assert_eq!(ok.path, dir.path());
    assert_eq!(ok.chromium_profile.as_deref(), Some("Default"));
}
