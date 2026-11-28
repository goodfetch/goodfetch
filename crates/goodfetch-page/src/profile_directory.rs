//! Pre-launch validation for a `profile_directory` Session source used in
//! place (ADR 0056; SA-11; ticket #737):
//! - refuses Chrome/Chromium/Edge/Brave's default OS user data directory,
//!   pointing to `running_browser`;
//! - checks browser family markers (`Local State`, `Last Version`,
//!   `compatibility.ini`, `prefs.js`) and refuses mismatches with
//!   `profile_directory_browser_mismatch`;
//! - checks browser lock files (`SingletonLock`, `lockfile`, `parent.lock`,
//!   `.parentlock`) and refuses active locks with `profile_directory_in_use`;
//! - checks the profile's recorded major version against the browser's and
//!   refuses downgrades with `profile_directory_newer_than_browser`;
//! - checks whether the profile fixes `browser.locale` and refuses a
//!   conflicting explicit `browser.locale` with `setting_locked`.

use std::path::{Path, PathBuf};

use goodfetch_engine::CallError;
use goodfetch_protocol::jsonrpc::ProtocolFailure;
use goodfetch_protocol::page::session::{
    PROFILE_DIRECTORY_BROWSER_MISMATCH, PROFILE_DIRECTORY_IN_USE,
    PROFILE_DIRECTORY_NEWER_THAN_BROWSER, ProfileDirectoryBrowserMismatchDetails,
    ProfileDirectoryInUseDetails, ProfileDirectoryNewerThanBrowserDetails,
};
use goodfetch_protocol::settings::session_source::BrowserName;
use goodfetch_protocol::settings::{
    CAPABILITY_UNAVAILABLE, CapabilityUnavailableDetails, SESSION_SOURCE, SETTING_LOCKED,
    SettingLimit, SettingLockedDetails, SettingsLayer,
};

/// A validated Chromium-family Profile directory ready to launch in place.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ProfileDirectory {
    pub(crate) path: PathBuf,
    pub(crate) chromium_profile: Option<String>,
    pub(crate) browser: BrowserName,
}

/// Validates `path` for launching `browser` in place (runs inside
/// `owner.blocking(...)`).
pub(crate) fn inspect(
    path: &Path,
    browser: BrowserName,
    chromium_profile: Option<&str>,
    browser_major: Option<u32>,
    configured_locale: Option<&str>,
    layer: SettingsLayer,
) -> Result<ProfileDirectory, CallError> {
    let display = path.display().to_string();

    if is_default_browser_data_dir(path) {
        return Err(CallError::new(
            &PROFILE_DIRECTORY_IN_USE,
            format!(
                "Profile directory `{display}` is a browser's default user data directory; use `running_browser` instead of launching on it in place."
            ),
            &ProfileDirectoryInUseDetails {
                path: display,
                reason: "default_user_data_directory".to_owned(),
                fix: "running_browser".to_owned(),
            },
        ));
    }

    if !path.is_dir() {
        return Err(CallError::protocol(
            ProtocolFailure::InvalidParams,
            format!("profile_directory `{display}` does not exist or is not a directory"),
        ));
    }

    check_browser_family(path, &display, browser, layer)?;
    check_lock_files(path, &display)?;
    check_version_and_locale(
        path,
        &display,
        chromium_profile,
        browser_major,
        configured_locale,
        layer,
    )?;

    Ok(ProfileDirectory {
        path: path.to_path_buf(),
        chromium_profile: chromium_profile.map(str::to_owned),
        browser,
    })
}

fn check_browser_family(
    path: &Path,
    display: &str,
    browser: BrowserName,
    layer: SettingsLayer,
) -> Result<(), CallError> {
    let is_chromium = matches!(
        browser,
        BrowserName::Chrome | BrowserName::Chromium | BrowserName::Edge | BrowserName::Brave
    );
    let has_firefox = path.join("compatibility.ini").exists()
        || path.join("prefs.js").exists()
        || path.join("places.sqlite").exists();
    let has_chromium = path.join("Local State").exists()
        || path.join("Last Version").exists()
        || path.join("Default").join("Preferences").exists()
        || path.join("Preferences").exists();

    if is_chromium && has_firefox && !has_chromium {
        return Err(CallError::new(
            &PROFILE_DIRECTORY_BROWSER_MISMATCH,
            format!(
                "Profile directory `{display}` is a Firefox profile, not `{}`.",
                browser_name(browser)
            ),
            &ProfileDirectoryBrowserMismatchDetails {
                path: display.to_owned(),
                expected: browser_name(browser).to_owned(),
                found: "firefox".to_owned(),
            },
        ));
    }
    if !is_chromium && has_chromium && !has_firefox {
        return Err(CallError::new(
            &PROFILE_DIRECTORY_BROWSER_MISMATCH,
            format!(
                "Profile directory `{display}` is a Chromium-family profile, not `{}`.",
                browser_name(browser)
            ),
            &ProfileDirectoryBrowserMismatchDetails {
                path: display.to_owned(),
                expected: browser_name(browser).to_owned(),
                found: "chromium".to_owned(),
            },
        ));
    }
    if !is_chromium {
        return Err(CallError::new(
            &CAPABILITY_UNAVAILABLE,
            format!(
                "This engine launches `profile_directory` in place only for Chromium-family browsers so far, not `{}`.",
                browser_name(browser)
            ),
            &CapabilityUnavailableDetails {
                key: SESSION_SOURCE.name.to_owned(),
                layer,
                kind: "profile_directory".to_owned(),
                fix: "`state_file`".to_owned(),
            },
        ));
    }
    Ok(())
}

fn check_lock_files(path: &Path, display: &str) -> Result<(), CallError> {
    // `SingletonLock` may be a dangling symlink on Linux, so check `symlink_metadata`.
    for lock_name in ["SingletonLock", "lockfile", "parent.lock", ".parentlock"] {
        if std::fs::symlink_metadata(path.join(lock_name)).is_ok() {
            return Err(CallError::new(
                &PROFILE_DIRECTORY_IN_USE,
                format!(
                    "Profile directory `{display}` is locked (`{lock_name}` exists); close the running browser or use `running_browser`."
                ),
                &ProfileDirectoryInUseDetails {
                    path: display.to_owned(),
                    reason: format!("lock file `{lock_name}` is present"),
                    fix: "close the browser using this profile or use `running_browser`".to_owned(),
                },
            ));
        }
    }
    Ok(())
}

fn check_version_and_locale(
    path: &Path,
    display: &str,
    chromium_profile: Option<&str>,
    browser_major: Option<u32>,
    configured_locale: Option<&str>,
    layer: SettingsLayer,
) -> Result<(), CallError> {
    if let Some((profile_version, profile_major)) = read_chromium_version(path)
        && let Some(browser_major) = browser_major
        && profile_major > browser_major
    {
        return Err(CallError::new(
            &PROFILE_DIRECTORY_NEWER_THAN_BROWSER,
            format!(
                "Profile directory `{display}` was written by Chromium {profile_version}, newer than browser major {browser_major}."
            ),
            &ProfileDirectoryNewerThanBrowserDetails {
                path: display.to_owned(),
                profile_version,
                browser_version: browser_major.to_string(),
            },
        ));
    }

    if let Some(requested) = configured_locale
        && let Some(fixed) = read_profile_locale(path, chromium_profile)
        && !fixed.eq_ignore_ascii_case(requested)
    {
        return Err(CallError::new(
            &SETTING_LOCKED,
            format!(
                "`browser.locale` is fixed to `{fixed}` by Profile directory `{display}` and cannot be overridden to `{requested}`."
            ),
            &SettingLockedDetails {
                key: "browser.locale".to_owned(),
                layer,
                limit: SettingLimit {
                    key: "browser.locale".to_owned(),
                    value: serde_json::Value::String(fixed),
                    layer: SettingsLayer::Run,
                    origin: None,
                },
                fix: "remove `browser.locale` or use a fresh Session".to_owned(),
            },
        ));
    }
    Ok(())
}

fn browser_name(browser: BrowserName) -> &'static str {
    match browser {
        BrowserName::Chrome => "chrome",
        BrowserName::Edge => "edge",
        BrowserName::Brave => "brave",
        BrowserName::Chromium => "chromium",
        BrowserName::Firefox => "firefox",
        BrowserName::Safari => "safari",
    }
}

/// Whether `path` matches a browser's OS default user data directory.
#[expect(
    clippy::disallowed_methods,
    reason = "reads HOME/LOCALAPPDATA purely for path comparison inside blocking inspection"
)]
fn is_default_browser_data_dir(path: &Path) -> bool {
    let normalized = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let mut defaults: Vec<PathBuf> = Vec::new();
    if let Ok(home) = std::env::var("HOME") {
        let home = PathBuf::from(home);
        // Linux defaults
        defaults.push(home.join(".config/google-chrome"));
        defaults.push(home.join(".config/google-chrome-beta"));
        defaults.push(home.join(".config/google-chrome-unstable"));
        defaults.push(home.join(".config/chromium"));
        defaults.push(home.join(".config/microsoft-edge"));
        defaults.push(home.join(".config/BraveSoftware/Brave-Browser"));
        // macOS defaults
        defaults.push(home.join("Library/Application Support/Google/Chrome"));
        defaults.push(home.join("Library/Application Support/Chromium"));
        defaults.push(home.join("Library/Application Support/Microsoft Edge"));
        defaults.push(home.join("Library/Application Support/BraveSoftware/Brave-Browser"));
    }
    if let Ok(local) = std::env::var("LOCALAPPDATA") {
        let local = PathBuf::from(local);
        defaults.push(local.join("Google/Chrome/User Data"));
        defaults.push(local.join("Chromium/User Data"));
        defaults.push(local.join("Microsoft/Edge/User Data"));
        defaults.push(local.join("BraveSoftware/Brave-Browser/User Data"));
    }
    defaults.into_iter().any(|candidate| {
        let cand = std::fs::canonicalize(&candidate).unwrap_or(candidate);
        cand == normalized
    })
}

#[expect(
    clippy::disallowed_methods,
    reason = "called inside blocking inspect()"
)]
fn read_chromium_version(path: &Path) -> Option<(String, u32)> {
    if let Ok(text) = std::fs::read_to_string(path.join("Last Version")) {
        let version = text.trim();
        if let Some(major) = parse_major(version) {
            return Some((version.to_owned(), major));
        }
    }
    if let Ok(text) = std::fs::read_to_string(path.join("Local State"))
        && let Ok(json) = serde_json::from_str::<serde_json::Value>(&text)
        && let Some(version) = json
            .pointer("/user_experience_metrics/stability/stats_version")
            .and_then(serde_json::Value::as_str)
        && let Some(major) = parse_major(version)
    {
        return Some((version.to_owned(), major));
    }
    None
}

fn parse_major(version: &str) -> Option<u32> {
    version.split('.').next()?.parse::<u32>().ok()
}

#[expect(
    clippy::disallowed_methods,
    reason = "called inside blocking inspect()"
)]
fn read_profile_locale(path: &Path, chromium_profile: Option<&str>) -> Option<String> {
    let subdir = chromium_profile.unwrap_or("Default");
    if let Ok(text) = std::fs::read_to_string(path.join(subdir).join("Preferences"))
        && let Ok(json) = serde_json::from_str::<serde_json::Value>(&text)
        && let Some(lang) = json
            .pointer("/intl/accept_languages")
            .and_then(serde_json::Value::as_str)
        && let Some(first) = lang
            .split(',')
            .next()
            .map(str::trim)
            .filter(|s| !s.is_empty())
    {
        return Some(first.to_owned());
    }
    if let Ok(text) = std::fs::read_to_string(path.join("Local State"))
        && let Ok(json) = serde_json::from_str::<serde_json::Value>(&text)
        && let Some(locale) = json
            .pointer("/intl/app_locale")
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
    {
        return Some(locale.to_owned());
    }
    None
}

#[cfg(test)]
mod tests;
