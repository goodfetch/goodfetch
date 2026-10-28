//! A Browser cache that holds the Chrome for Testing build Collie pins,
//! for a test that launches a browser (ADRs 0024, 0059 and 0098): laid out
//! as a prepared `GOODFETCH_BROWSERS_PATH` copy holds it, in the test's own
//! folder, its files taken from the same build Playwright installs for the
//! repository's own browser tests (`pnpm exec playwright install`), so no
//! test downloads a browser. Its files are hard links where the folders
//! share a file system, copies otherwise, so a launch's digest check reads
//! the pinned bytes and a clean-up that deletes the build never touches
//! Playwright's.
#![expect(
    clippy::disallowed_methods,
    reason = "test support lays out the cache in the test's thread, never on an async worker"
)]

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::env::{START_TIME, TestEnv};
use crate::variables::{PLAYWRIGHT_BROWSERS_VARIABLE, Variables};

/// The Chrome for Testing build Collie pins, which Playwright's pinned
/// release (1.64.1, `browsers.json`) also runs.
pub const CHROME_FOR_TESTING_BUILD: &str = "156.0.8078.4";

/// Playwright's revision of that build, the name of its folder.
const PLAYWRIGHT_REVISION: &str = "1248";

/// The variable that points an engine at a Browser cache (ADR 0059).
pub const BROWSERS_PATH_VARIABLE: &str = "GOODFETCH_BROWSERS_PATH";

/// Why the cache could not be laid out.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum BrowserCacheError {
    /// Playwright has not installed the build: run
    /// `pnpm exec playwright install chromium`.
    #[error(
        "Playwright's Chrome for Testing {CHROME_FOR_TESTING_BUILD} is not at {0}: run `pnpm exec playwright install chromium`"
    )]
    NotInstalled(PathBuf),
    /// This platform has no Chrome for Testing build.
    #[error("Chrome for Testing has no build for this platform")]
    NoBuild,
    /// The tests' copy of Foxhound's pinned Lightpanda build is missing:
    /// run `ci/lightpanda-fetch.sh` (part of `just fetch`).
    #[error(
        "Lightpanda {LIGHTPANDA_BUILD} is not at {0}: run `ci/lightpanda-fetch.sh` (part of `just fetch`)"
    )]
    LightpandaNotFetched(PathBuf),
    /// The cache's files could not be written.
    #[error("the Browser cache cannot be laid out: {0}")]
    Io(#[from] io::Error),
}

/// The folder Chrome for Testing's archive unpacks to on this platform.
fn platform_folder() -> Option<&'static str> {
    if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        Some("chrome-mac-arm64")
    } else if cfg!(all(target_os = "macos", target_arch = "x86_64")) {
        Some("chrome-mac-x64")
    } else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        Some("chrome-linux64")
    } else if cfg!(all(target_os = "windows", target_arch = "x86_64")) {
        Some("chrome-win64")
    } else {
        None
    }
}

/// Playwright's folder of browsers: `PLAYWRIGHT_BROWSERS_PATH`, else its
/// default under the user's folders.
fn playwright_browsers(variables: &Variables) -> Option<PathBuf> {
    if let Some(path) = variables.get(PLAYWRIGHT_BROWSERS_VARIABLE) {
        return Some(PathBuf::from(path));
    }
    if cfg!(target_os = "windows") {
        return variables
            .get("LOCALAPPDATA")
            .map(|folder| Path::new(folder).join("ms-playwright"));
    }
    let home = Path::new(variables.get("HOME")?);
    Some(if cfg!(target_os = "macos") {
        home.join("Library/Caches/ms-playwright")
    } else {
        home.join(".cache/ms-playwright")
    })
}

/// A Browser cache in `env`'s folder holding the pinned Chrome for Testing
/// build, writable by the engine; its root, which the test passes as
/// `GOODFETCH_BROWSERS_PATH`. Called again in the same folder, as a test
/// that starts several Runs does, it lays out only the build's files that
/// are missing and leaves the others in place, since a browser started
/// from them may still run them (fix-queue 106).
///
/// # Errors
///
/// When Playwright has not installed the build, the platform has none, or
/// the folder cannot be written.
pub fn browser_cache(env: &TestEnv) -> Result<PathBuf, BrowserCacheError> {
    chrome_for_testing_at(env.folder().join("browsers"))
}

/// The Browser cache's folder in a Goodfetch home, where an engine started
/// without `GOODFETCH_BROWSERS_PATH` keeps its Browser cache (ADR 0098).
const HOME_BROWSERS: &str = "browsers";

/// [`browser_cache`]'s build laid out in `env`'s Goodfetch home, in the
/// Browser cache an engine started without `GOODFETCH_BROWSERS_PATH` uses,
/// so the test passes no variable: for a test that searches every file of
/// the home, such as the reset gate (ADR 0001), whose search must take in
/// the cache as a person's home holds it. Its root.
///
/// # Errors
///
/// As [`browser_cache`].
pub fn browser_cache_in_home(env: &TestEnv) -> Result<PathBuf, BrowserCacheError> {
    chrome_for_testing_at(env.home().join(HOME_BROWSERS))
}

/// A Browser cache at `root` holding the pinned Chrome for Testing build,
/// as [`browser_cache`] describes it; `root`.
fn chrome_for_testing_at(root: PathBuf) -> Result<PathBuf, BrowserCacheError> {
    let platform = platform_folder().ok_or(BrowserCacheError::NoBuild)?;
    let source = playwright_browsers(&Variables::from_process())
        .unwrap_or_default()
        .join(format!("chromium-{PLAYWRIGHT_REVISION}"))
        .join(platform);
    if !source.is_dir() {
        return Err(BrowserCacheError::NotInstalled(source));
    }
    let entry = root
        .join("browser")
        .join("chrome-for-testing")
        .join(CHROME_FOR_TESTING_BUILD);
    let files = entry.join("files").join(platform);
    link_tree(&source, &files)?;
    fs::write(entry.join("lock"), "")?;
    let installed = serde_json::json!({
        "format_version": "0.1",
        "artifact": {
            "kind": "browser",
            "name": "chrome-for-testing",
            "build": CHROME_FOR_TESTING_BUILD,
        },
        "digest": "sha256:0000000000000000000000000000000000000000000000000000000000000000",
        "installed_at": START_TIME,
        "used_at": START_TIME,
    });
    fs::write(entry.join("entry.json"), installed.to_string())?;
    Ok(root)
}

/// The Lightpanda build Foxhound pins (`goodfetch-fetch-tier`'s
/// `light_browser/pins.json`): a launch's digest check catches a drift.
pub const LIGHTPANDA_BUILD: &str = "1.0.2";

/// The variable naming the folder `ci/lightpanda-fetch.sh` downloads the
/// tests' copy of Lightpanda into, `~/.cache/goodfetch/lightpanda` by
/// default.
pub const LIGHTPANDA_DOWNLOADS_VARIABLE: &str = "GOODFETCH_LIGHTPANDA_DOWNLOADS";

/// The name of Lightpanda's release executable for this platform.
fn lightpanda_asset() -> Option<&'static str> {
    if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        Some("lightpanda-aarch64-macos")
    } else if cfg!(all(target_os = "macos", target_arch = "x86_64")) {
        Some("lightpanda-x86_64-macos")
    } else if cfg!(all(
        target_os = "linux",
        target_env = "gnu",
        target_arch = "x86_64"
    )) {
        Some("lightpanda-x86_64-linux")
    } else if cfg!(all(
        target_os = "linux",
        target_env = "gnu",
        target_arch = "aarch64"
    )) {
        Some("lightpanda-aarch64-linux")
    } else {
        None
    }
}

/// The tests' copy of the Lightpanda build Foxhound pins for this
/// platform, which `ci/lightpanda-fetch.sh` downloads: its release
/// executable, named as the release names it.
///
/// # Errors
///
/// [`BrowserCacheError::NoBuild`] on a platform Lightpanda builds nothing
/// for, [`BrowserCacheError::LightpandaNotFetched`] when the copy is
/// missing.
pub fn lightpanda_copy() -> Result<PathBuf, BrowserCacheError> {
    let asset = lightpanda_asset().ok_or(BrowserCacheError::NoBuild)?;
    let variables = Variables::from_process();
    let downloads = variables
        .get(LIGHTPANDA_DOWNLOADS_VARIABLE)
        .map(PathBuf::from)
        .or_else(|| {
            variables
                .get("HOME")
                .map(|home| Path::new(home).join(".cache/goodfetch/lightpanda"))
        })
        .unwrap_or_default();
    let source = downloads.join(LIGHTPANDA_BUILD).join(asset);
    if source.is_file() {
        Ok(source)
    } else {
        Err(BrowserCacheError::LightpandaNotFetched(source))
    }
}

/// [`browser_cache`], holding also the Lightpanda build Foxhound pins for
/// the light-browser tier (ADR 0020), laid out as a prepared copy holds
/// it from the tests' own copy, which `ci/lightpanda-fetch.sh` downloads,
/// so no test downloads it either. Called again in the same folder, it
/// leaves an executable already laid out in place, as [`browser_cache`]
/// does.
///
/// # Errors
///
/// [`browser_cache`]'s, and [`BrowserCacheError::LightpandaNotFetched`]
/// when the tests' copy is missing.
pub fn browser_cache_with_lightpanda(env: &TestEnv) -> Result<PathBuf, BrowserCacheError> {
    let source = lightpanda_copy()?;
    let asset = lightpanda_asset().ok_or(BrowserCacheError::NoBuild)?;
    let root = browser_cache(env)?;
    let entry = root
        .join("browser")
        .join("lightpanda")
        .join(LIGHTPANDA_BUILD);
    let files = entry.join("files");
    fs::create_dir_all(&files)?;
    link_file(&source, &files.join(asset))?;
    fs::write(entry.join("lock"), "")?;
    let installed = serde_json::json!({
        "format_version": "0.1",
        "artifact": {
            "kind": "browser",
            "name": "lightpanda",
            "build": LIGHTPANDA_BUILD,
        },
        "digest": "sha256:0000000000000000000000000000000000000000000000000000000000000000",
        "installed_at": START_TIME,
        "used_at": START_TIME,
    });
    fs::write(entry.join("entry.json"), installed.to_string())?;
    Ok(root)
}

/// `source`'s tree at `target`: each file as [`link_file`] lays it out,
/// and each symbolic link the same link unless one is already there.
fn link_tree(source: &Path, target: &Path) -> io::Result<()> {
    fs::create_dir_all(target)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let from = entry.path();
        let to = target.join(entry.file_name());
        let kind = entry.file_type()?;
        if kind.is_dir() {
            link_tree(&from, &to)?;
        } else if kind.is_symlink() {
            if fs::symlink_metadata(&to).is_err() {
                symlink(&fs::read_link(&from)?, &to)?;
            }
        } else {
            link_file(&from, &to)?;
        }
    }
    Ok(())
}

/// `source` at `target`: a hard link, or a copy across file systems. A
/// file already at `target`, laid out by an earlier call in the same
/// folder, stays as it is: a browser started from it may still run it,
/// and Linux refuses to open a running executable for writing
/// (`ETXTBSY`), while over a hard link a copy would empty the source.
fn link_file(source: &Path, target: &Path) -> io::Result<()> {
    if fs::symlink_metadata(target).is_ok() {
        return Ok(());
    }
    if fs::hard_link(source, target).is_err() {
        fs::copy(source, target)?;
    }
    Ok(())
}

#[cfg(unix)]
fn symlink(original: &Path, link: &Path) -> io::Result<()> {
    std::os::unix::fs::symlink(original, link)
}

#[cfg(windows)]
fn symlink(original: &Path, link: &Path) -> io::Result<()> {
    std::os::windows::fs::symlink_file(original, link)
}

/// Lays out a stand-in entry of the build `build` in the Browser cache at
/// `root`, installed and last used at `used_at`, as `browser clean` reads
/// one: its files (a placeholder `chrome`), its lock and its `entry.json`.
/// Answers its folder.
///
/// # Panics
///
/// When the folder cannot be written.
pub fn lay_out_browser_entry(root: &Path, build: &str, used_at: &str) -> PathBuf {
    let folder = root.join("browser").join("chrome-for-testing").join(build);
    let written = (|| -> io::Result<()> {
        fs::create_dir_all(folder.join("files"))?;
        fs::write(folder.join("files").join("chrome"), "#!chrome\n")?;
        fs::write(folder.join("lock"), "")?;
        let entry = serde_json::json!({
            "format_version": "0.1",
            "artifact": { "kind": "browser", "name": "chrome-for-testing", "build": build },
            "digest": "sha256:0000000000000000000000000000000000000000000000000000000000000000",
            "installed_at": used_at,
            "used_at": used_at,
        });
        fs::write(folder.join("entry.json"), entry.to_string())
    })();
    written.unwrap_or_else(|error| panic!("the entry of `{build}` cannot be laid out: {error}"));
    folder
}
