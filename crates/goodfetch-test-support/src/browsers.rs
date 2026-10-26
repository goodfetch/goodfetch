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
    let platform = platform_folder().ok_or(BrowserCacheError::NoBuild)?;
    let source = playwright_browsers(&Variables::from_process())
        .unwrap_or_default()
        .join(format!("chromium-{PLAYWRIGHT_REVISION}"))
        .join(platform);
    if !source.is_dir() {
        return Err(BrowserCacheError::NotInstalled(source));
    }
    let root = env.folder().join("browsers");
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

/// `source`'s tree at `target`: each file a hard link, or a copy across
/// file systems, and each symbolic link the same link. A file or link
/// already at `target` stays as it is: Linux refuses to open a running
/// executable for writing (`ETXTBSY`), and over a hard link a copy would
/// empty Playwright's own file.
fn link_tree(source: &Path, target: &Path) -> io::Result<()> {
    fs::create_dir_all(target)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let from = entry.path();
        let to = target.join(entry.file_name());
        let kind = entry.file_type()?;
        if !kind.is_dir() && fs::symlink_metadata(&to).is_ok() {
            continue;
        }
        if kind.is_symlink() {
            symlink(&fs::read_link(&from)?, &to)?;
        } else if kind.is_dir() {
            link_tree(&from, &to)?;
        } else if fs::hard_link(&from, &to).is_err() {
            fs::copy(&from, &to)?;
        }
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
