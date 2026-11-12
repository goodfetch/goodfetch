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
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::env::{START_TIME, TestEnv};
use crate::facts::{CHROME_FOR_TESTING_BUILD, LIGHTPANDA_BUILD};
use crate::process::Process;
use crate::variables::{PLAYWRIGHT_BROWSERS_VARIABLE, Variables};

/// Playwright's revision of the Chrome for Testing build Collie pins, which
/// Playwright's pinned release (1.64.1, `browsers.json`) also runs: the name
/// of its folder.
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

/// The folder of the Browser cache at `cache` where [`browser_cache`] lays
/// out the pinned Chrome for Testing build's files, Playwright's own: a
/// test that searches its folder for what the engine wrote may leave them
/// out, since no engine writes there, and the cache's own records beside
/// them stay searched.
pub fn chrome_for_testing_files(cache: &Path) -> PathBuf {
    chrome_for_testing_entry(cache).join("files")
}

/// The executable of the pinned build in the Browser cache at `cache`, for a
/// test that starts a browser of its own.
pub fn chrome_for_testing(cache: &Path) -> PathBuf {
    let files = chrome_for_testing_files(cache);
    if cfg!(target_os = "linux") {
        return files.join("chrome-linux64").join("chrome");
    }
    let platform = if cfg!(target_arch = "aarch64") {
        "chrome-mac-arm64"
    } else {
        "chrome-mac-x64"
    };
    files
        .join(platform)
        .join("Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing")
}

/// Starts the pinned build of the Browser cache at `cache`, headless, to
/// load `url` once, print the page's DOM once loaded, its scripts run
/// (`--dump-dom`), and exit: a stock launch but for `switches`, for a test
/// of what a site sees of the pinned browser. `host` resolves to `address`,
/// and no certificate is checked, which changes nothing the ClientHello
/// sends, so a site of the Proving ground is reached over TLS. Each call
/// takes a profile of its own in `env`'s folder, so several may run at
/// once.
///
/// # Errors
///
/// When the browser cannot start.
pub fn chrome_for_testing_dumping_dom(
    env: &TestEnv,
    cache: &Path,
    (host, address): (&str, IpAddr),
    switches: &[&str],
    url: &str,
) -> io::Result<Process> {
    let mut command = env.process_command(chrome_for_testing(cache));
    command.args([
        "--headless",
        "--no-first-run",
        "--no-default-browser-check",
        "--password-store=basic",
        "--use-mock-keychain",
        "--ignore-certificate-errors",
    ]);
    command.args(switches);
    command.arg(format!(
        "--user-data-dir={}",
        env.folder()
            .join(env.unique_name("chrome-profile"))
            .display()
    ));
    command.arg(format!("--host-resolver-rules=MAP {host} {address}"));
    command.args(["--dump-dom", url]);
    env.spawn(&mut command)
}

/// How long a test's own browser has to report its debugging port.
const PORT_DEADLINE: Duration = Duration::from_secs(30);

/// The port a browser launched with `--remote-debugging-port=0` on
/// `profile` reports in its `DevToolsActivePort` file.
///
/// # Panics
///
/// When it reports none within its deadline.
pub fn reported_debugging_port(profile: &Path) -> u16 {
    let file = profile.join("DevToolsActivePort");
    let started = Instant::now();
    loop {
        if let Some(port) = std::fs::read_to_string(&file)
            .ok()
            .and_then(|text| text.lines().next()?.parse().ok())
        {
            return port;
        }
        assert!(
            started.elapsed() < PORT_DEADLINE,
            "{} reported no port",
            file.display()
        );
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// The pinned Chrome for Testing build's entry in the Browser cache at
/// `cache`.
fn chrome_for_testing_entry(cache: &Path) -> PathBuf {
    cache
        .join("browser")
        .join("chrome-for-testing")
        .join(CHROME_FOR_TESTING_BUILD)
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
    let entry = chrome_for_testing_entry(&root);
    link_tree(&source, &chrome_for_testing_files(&root).join(platform))?;
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
