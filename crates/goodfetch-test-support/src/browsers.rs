//! A Browser cache that holds the Chrome for Testing build Collie pins,
//! for a test that launches a browser (ADRs 0024, 0059 and 0098): laid out
//! as a prepared `GOODFETCH_BROWSERS_PATH` copy holds it, in the test's own
//! folder, its files taken from the same build Playwright installs for the
//! repository's own browser tests (`pnpm exec playwright install`), so no
//! test downloads a browser. Its files are hard links, so a launch's digest
//! check reads the pinned bytes, a clean-up that deletes the build never
//! touches Playwright's, and a test's folder, kept when the test fails,
//! holds no bytes of the build of its own: links to Playwright's files where
//! the system allows them, else to one copy of the build laid out beside the
//! tests' folders, which every test's cache then links (#2664: where
//! Playwright's files belong to another user, as root's `/ms-playwright` on
//! the gate host, Linux's `protected_hardlinks` refuses the link, and each
//! test's folder held a 400 MB copy). A file is copied only where neither
//! link can be made.
#![expect(
    clippy::disallowed_methods,
    reason = "test support lays out the cache in the test's thread, never on an async worker"
)]

use std::ffi::OsStr;
use std::fs;
use std::io;
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::process::Stdio;
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

/// The name the full build is pinned under, and its entry's folder.
const CHROME_FOR_TESTING: &str = "chrome-for-testing";

/// The name the headless shell of the same release is pinned under, and
/// its entry's folder (#3224).
const CHROME_HEADLESS_SHELL: &str = "chrome-headless-shell";

/// The folder the headless shell's archive unpacks to on this platform.
fn shell_platform_folder() -> Option<String> {
    let platform = platform_folder()?.strip_prefix("chrome-")?;
    Some(format!("{CHROME_HEADLESS_SHELL}-{platform}"))
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
    chrome_for_testing_at(env.folder().join("browsers"), tests_folder(env))
}

/// The folder `env`'s folder is in, where the tests' folders go, and the
/// build's copy their caches link.
fn tests_folder(env: &TestEnv) -> &Path {
    env.folder().parent().unwrap_or(env.folder())
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
    chrome_for_testing_at(env.home().join(HOME_BROWSERS), tests_folder(env))
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

/// Starts the pinned build of the Browser cache at `cache` headless, as
/// [`chrome_headless`] picks it, to load `url` once, print the page's DOM
/// once loaded, its scripts run (`--dump-dom`), and exit: a stock launch
/// but for `switches`, for a test of what a site sees of the pinned
/// browser. `host` resolves to `address`, and no certificate is checked,
/// which changes nothing the ClientHello sends, so a site of the Proving
/// ground is reached over TLS. Each call takes a profile of its own in
/// `env`'s folder, so several may run at once.
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
    let mut command = env.process_command(chrome_headless(cache));
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

/// A browser of the pinned build that a test drives over CDP itself.
pub struct DebuggedBrowser {
    /// Its process.
    pub process: Process,
    /// The loopback port it listens for CDP on.
    pub port: u16,
    /// Its browser target's WebSocket URL on that port.
    pub url: String,
}

/// Starts the pinned build of the Browser cache at `cache` headless, as
/// [`chrome_headless`] picks it, on the profile at `profile`, with
/// `switches` beside a stock headless launch's, listening for CDP on a
/// loopback port it picks and reports; it opens `about:blank` and writes
/// nothing to the test's output.
///
/// # Errors
///
/// When the browser cannot start.
///
/// # Panics
///
/// When it reports no port within its deadline.
pub fn chrome_for_testing_debugged(
    env: &TestEnv,
    cache: &Path,
    profile: &Path,
    switches: impl IntoIterator<Item: AsRef<OsStr>>,
) -> io::Result<DebuggedBrowser> {
    let mut command = env.process_command(chrome_headless(cache));
    command
        .args([
            "--headless",
            "--no-first-run",
            "--use-mock-keychain",
            "--password-store=basic",
            "--remote-debugging-port=0",
        ])
        .args(switches)
        .arg(format!("--user-data-dir={}", profile.display()))
        .arg("about:blank")
        .stderr(Stdio::null());
    let process = env.spawn(&mut command)?;
    let port = reported_debugging_port(profile);
    let reported = fs::read_to_string(profile.join("DevToolsActivePort"))?;
    let path = reported.lines().nth(1).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "the browser reported no path on its port's second line",
        )
    })?;
    Ok(DebuggedBrowser {
        process,
        port,
        url: format!("ws://127.0.0.1:{port}{path}"),
    })
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

/// The executable of the headless shell in the Browser cache at `cache`,
/// which [`browser_cache_with_headless_shell`] lays out.
pub fn chrome_headless_shell(cache: &Path) -> PathBuf {
    let platform = shell_platform_folder().unwrap_or_default();
    entry(cache, CHROME_HEADLESS_SHELL)
        .join("files")
        .join(platform)
        .join(CHROME_HEADLESS_SHELL)
}

/// The executable a test starts headless itself from the Browser cache at
/// `cache`: the pinned build's, or, where the tests run with no GUI
/// session ([`Variables::no_gui_session`]), where the full build cannot
/// start headless, the headless shell of the same release, which
/// [`browser_cache`] then lays out too (#3224).
pub fn chrome_headless(cache: &Path) -> PathBuf {
    if Variables::from_process().no_gui_session() {
        chrome_headless_shell(cache)
    } else {
        chrome_for_testing(cache)
    }
}

/// Whether a test that needs the full build with a window, or a `stealth`
/// Session, which launches its own build, skips here: where the tests run
/// with no GUI session, as the Release suite's macOS cells do (#3224),
/// saying so with `reason` on stderr. Asked before the test's [`TestEnv`],
/// so a skipped test leaves no exercise record.
pub fn skipped_without_gui_session(reason: &str) -> bool {
    if !Variables::from_process().no_gui_session() {
        return false;
    }
    #[expect(
        clippy::print_stderr,
        reason = "a skipped test says why in its own output, which the test harness keeps"
    )]
    {
        eprintln!("skipped with no GUI session: {reason}");
    }
    true
}

/// [`browser_cache`], holding also the headless shell of the pinned
/// release, laid out as a prepared copy holds it from Playwright's own
/// (`pnpm exec playwright install chromium` installs both), for a test of
/// `browser.headless_shell` (#3224).
///
/// # Errors
///
/// [`browser_cache`]'s, and [`BrowserCacheError::NotInstalled`] when
/// Playwright has not installed the shell.
pub fn browser_cache_with_headless_shell(env: &TestEnv) -> Result<PathBuf, BrowserCacheError> {
    let root = browser_cache(env)?;
    headless_shell_at(&root, tests_folder(env))?;
    Ok(root)
}

/// The pinned Chrome for Testing build's entry in the Browser cache at
/// `cache`.
fn chrome_for_testing_entry(cache: &Path) -> PathBuf {
    entry(cache, CHROME_FOR_TESTING)
}

/// The entry of the pinned release's build `name` in the Browser cache at
/// `cache`.
fn entry(cache: &Path, name: &str) -> PathBuf {
    cache
        .join("browser")
        .join(name)
        .join(CHROME_FOR_TESTING_BUILD)
}

/// A Browser cache at `root` holding the pinned Chrome for Testing build,
/// as [`browser_cache`] describes it, its files linked from Playwright's or
/// from the build's copy in `tests_folder`, and its headless shell too
/// where the tests run with no GUI session; `root`.
fn chrome_for_testing_at(root: PathBuf, tests_folder: &Path) -> Result<PathBuf, BrowserCacheError> {
    let platform = platform_folder().ok_or(BrowserCacheError::NoBuild)?;
    let variables = Variables::from_process();
    let source = playwright_browsers(&variables)
        .unwrap_or_default()
        .join(format!("chromium-{PLAYWRIGHT_REVISION}"))
        .join(platform);
    lay_out_entry(&root, tests_folder, CHROME_FOR_TESTING, &source, platform)?;
    if variables.no_gui_session() {
        headless_shell_at(&root, tests_folder)?;
    }
    Ok(root)
}

/// The headless shell of the pinned release in the Browser cache at
/// `root`, linked as [`chrome_for_testing_at`] links the full build.
fn headless_shell_at(root: &Path, tests_folder: &Path) -> Result<(), BrowserCacheError> {
    let platform = shell_platform_folder().ok_or(BrowserCacheError::NoBuild)?;
    let source = playwright_browsers(&Variables::from_process())
        .unwrap_or_default()
        .join(format!("chromium_headless_shell-{PLAYWRIGHT_REVISION}"))
        .join(&platform);
    lay_out_entry(
        root,
        tests_folder,
        CHROME_HEADLESS_SHELL,
        &source,
        &platform,
    )
}

/// The entry of the pinned release's build `name` in the Browser cache at
/// `root`, its files at `files/<platform>` linked from Playwright's at
/// `source` or from their copy in `tests_folder`, with its lock and its
/// `entry.json`.
fn lay_out_entry(
    root: &Path,
    tests_folder: &Path,
    name: &str,
    source: &Path,
    platform: &str,
) -> Result<(), BrowserCacheError> {
    if !source.is_dir() {
        return Err(BrowserCacheError::NotInstalled(source.to_owned()));
    }
    let entry = entry(root, name);
    let build = linkable_build(source, tests_folder, platform)?;
    link_tree(&build, &entry.join("files").join(platform))?;
    fs::write(entry.join("lock"), "")?;
    let installed = serde_json::json!({
        "format_version": "0.1",
        "artifact": {
            "kind": "browser",
            "name": name,
            "build": CHROME_FOR_TESTING_BUILD,
        },
        "digest": "sha256:0000000000000000000000000000000000000000000000000000000000000000",
        "installed_at": START_TIME,
        "used_at": START_TIME,
    });
    fs::write(entry.join("entry.json"), installed.to_string())?;
    Ok(())
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

/// The tree of the build at `source` that a cache in `tests_folder` can
/// hard-link: `source` itself when the system allows a link to its files
/// there, else the build's copy in `tests_folder`, as [`build_copy`] lays
/// it out.
fn linkable_build(source: &Path, tests_folder: &Path, platform: &str) -> io::Result<PathBuf> {
    if links_allowed(source, tests_folder)? {
        return Ok(source.to_owned());
    }
    build_copy(source, tests_folder, platform)
}

/// The copy of the build at `source` in `tests_folder`,
/// `goodfetch-chrome-for-testing-<build>-<platform>`: laid out by the first
/// test that needs it and kept for the others and for later runs. One test
/// at a time lays it out, under a lock beside it, so tests that start
/// together hold one copy, not one each; it is laid out in a folder of its
/// own and renamed in place once whole, so a test never links a part of one,
/// and a part left by a test killed while laying it out goes with the next
/// layout.
fn build_copy(source: &Path, tests_folder: &Path, platform: &str) -> io::Result<PathBuf> {
    let name = format!("goodfetch-chrome-for-testing-{CHROME_FOR_TESTING_BUILD}-{platform}");
    let copy = tests_folder.join(&name);
    if copy.is_dir() {
        return Ok(copy);
    }
    let lock = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(tests_folder.join(format!("{name}.lock")))?;
    lock.lock()?;
    if copy.is_dir() {
        return Ok(copy);
    }
    let part = format!(".{name}-");
    for entry in fs::read_dir(tests_folder)? {
        let entry = entry?;
        if entry.file_name().to_string_lossy().starts_with(&part) {
            fs::remove_dir_all(entry.path())?;
        }
    }
    let laying_out = tempfile::Builder::new()
        .prefix(&part)
        .tempdir_in(tests_folder)?;
    let whole = laying_out.path().join(platform);
    link_tree(source, &whole)?;
    fs::rename(&whole, &copy)?;
    Ok(copy)
}

/// Whether a file of the tree at `source` can be hard-linked into
/// `tests_folder`: tried on its first file, in a folder of its own that
/// goes with the answer. A tree with no file has nothing to link.
fn links_allowed(source: &Path, tests_folder: &Path) -> io::Result<bool> {
    let Some(first) = first_file(source)? else {
        return Ok(true);
    };
    let probe = tempfile::Builder::new()
        .prefix(".goodfetch-link-probe-")
        .tempdir_in(tests_folder)?;
    Ok(fs::hard_link(&first, probe.path().join("file")).is_ok())
}

/// The first regular file of the tree at `folder`, in its folders' order.
fn first_file(folder: &Path) -> io::Result<Option<PathBuf>> {
    for entry in fs::read_dir(folder)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        if kind.is_file() {
            return Ok(Some(entry.path()));
        }
        if kind.is_dir()
            && let Some(file) = first_file(&entry.path())?
        {
            return Ok(Some(file));
        }
    }
    Ok(None)
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
    let folder = root.join("browser").join(CHROME_FOR_TESTING).join(build);
    let written = (|| -> io::Result<()> {
        fs::create_dir_all(folder.join("files"))?;
        fs::write(folder.join("files").join("chrome"), "#!chrome\n")?;
        fs::write(folder.join("lock"), "")?;
        let entry = serde_json::json!({
            "format_version": "0.1",
            "artifact": { "kind": "browser", "name": CHROME_FOR_TESTING, "build": build },
            "digest": "sha256:0000000000000000000000000000000000000000000000000000000000000000",
            "installed_at": used_at,
            "used_at": used_at,
        });
        fs::write(folder.join("entry.json"), entry.to_string())
    })();
    written.unwrap_or_else(|error| panic!("the entry of `{build}` cannot be laid out: {error}"));
    folder
}

#[cfg(test)]
mod tests;
