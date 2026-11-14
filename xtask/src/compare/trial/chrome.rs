//! Chrome for Testing as a trial takes it: the folder of the build
//! Playwright installed, and Foxhound's Browser cache laid out from it.

use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};

use crate::test_facts::{CHROME_FOR_TESTING_BUILD, PLAYWRIGHT_REVISION};

// Every tool of the headless browser scenario runs one browser, the Chrome
// for Testing build Collie pins (`CHROME_FOR_TESTING_BUILD`), from the
// folder Playwright installs it in (`PLAYWRIGHT_REVISION`); test support's
// `browsers.rs` lays out the same build for the tests.
/// Chrome for Testing's folder and executable in it, on this platform.
#[cfg(target_os = "macos")]
const CHROME_FOLDER: &str = if cfg!(target_arch = "aarch64") {
    "chrome-mac-arm64"
} else {
    "chrome-mac-x64"
};
#[cfg(target_os = "macos")]
pub(crate) const CHROME: &str =
    "Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing";
#[cfg(not(target_os = "macos"))]
const CHROME_FOLDER: &str = "chrome-linux64";
#[cfg(not(target_os = "macos"))]
pub(crate) const CHROME: &str = "chrome";

/// The folder of the Chrome for Testing build Playwright installed
/// (`PLAYWRIGHT_BROWSERS_PATH`, else its default folder), which the
/// headless browser scenario runs; none when it is not installed.
pub(crate) fn chrome_folder() -> Option<PathBuf> {
    let browsers = if let Some(path) = std::env::var_os("PLAYWRIGHT_BROWSERS_PATH") {
        PathBuf::from(path)
    } else {
        let home = PathBuf::from(std::env::var_os("HOME")?);
        if cfg!(target_os = "macos") {
            home.join("Library/Caches/ms-playwright")
        } else {
            home.join(".cache/ms-playwright")
        }
    };
    let folder = browsers
        .join(format!("chromium-{PLAYWRIGHT_REVISION}"))
        .join(CHROME_FOLDER);
    folder.join(CHROME).is_file().then_some(folder)
}

/// Lays out, in a folder of its own, a Browser cache holding `chrome`'s
/// files as Foxhound's pinned build, as a prepared `GOODFETCH_BROWSERS_PATH`
/// holds it: hard links where the folders share a file system, copies
/// otherwise.
pub(crate) fn browser_cache(chrome: &Path) -> Result<tempfile::TempDir> {
    let root = tempfile::tempdir().context("creating the Browser cache's folder")?;
    let entry = root
        .path()
        .join("browser")
        .join("chrome-for-testing")
        .join(CHROME_FOR_TESTING_BUILD);
    link_tree(chrome, &entry.join("files").join(CHROME_FOLDER))
        .context("laying out the Browser cache")?;
    std::fs::write(entry.join("lock"), "").context("writing the Browser cache's lock")?;
    let installed = serde_json::json!({
        "format_version": "0.1",
        "artifact": {
            "kind": "browser",
            "name": "chrome-for-testing",
            "build": CHROME_FOR_TESTING_BUILD,
        },
        "digest": "sha256:0000000000000000000000000000000000000000000000000000000000000000",
        "installed_at": "2026-01-01T00:00:00Z",
        "used_at": "2026-01-01T00:00:00Z",
    });
    std::fs::write(entry.join("entry.json"), installed.to_string())
        .context("writing the Browser cache's entry")?;
    Ok(root)
}

/// `from`'s files under `to`, each a hard link, or a copy across file
/// systems; symbolic links kept as such.
fn link_tree(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let (source, target) = (entry.path(), to.join(entry.file_name()));
        let kind = entry.file_type()?;
        if kind.is_dir() {
            link_tree(&source, &target)?;
        } else if kind.is_symlink() {
            #[cfg(unix)]
            std::os::unix::fs::symlink(std::fs::read_link(&source)?, &target)?;
        } else if std::fs::hard_link(&source, &target).is_err() {
            std::fs::copy(&source, &target)?;
        }
    }
    Ok(())
}
