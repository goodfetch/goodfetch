//! A Run's browser, launched by the engine (ADRs 0017, 0024, 0098 and
//! 0115): its build prepared in the Browser cache from the call that
//! launches it, so a missing build raises its code there and nowhere
//! earlier, held under a shared lock until the process exits; the process
//! started over its debugging pipe, never a debugging port, on a profile of
//! its own in the Run's folder; and its end: when the Run stops, or the
//! browser pool closes it, the browser is asked to close, then killed if it
//! does not, its profile removed and its build released.

mod arguments;
#[cfg(unix)]
mod pipe;
mod preferences;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;

use goodfetch_cdp::Connection;
use goodfetch_engine::CallError;
use goodfetch_engine::browser_cache::{
    BrowserCache, BrowserChoice, BrowserSource, BuildPlatform, CacheLocation, HeldArtifact, Pins,
    Platform, PlatformNotice, browser_source, installed_browser,
};
use goodfetch_engine::children::Children;
use goodfetch_engine::deadline::Deadline;
use goodfetch_engine::happy_eyeballs::Delays;
use goodfetch_engine::home::Home;
use goodfetch_engine::http_client::Resolve;
use goodfetch_engine::metrics::Counted;
use goodfetch_engine::pinned_download::Fetching;
use goodfetch_engine::runs::Run;
use goodfetch_engine::settings::RunSettings;
use goodfetch_protocol::browser_cache::{
    BROWSER_NOT_INSTALLED, BrowserNotInstalledDetails, MissingBrowser,
};
use goodfetch_protocol::egress::EGRESS_OFFLINE;
use goodfetch_protocol::page::{
    BROWSER_HEADLESS, BROWSER_LAUNCH_FAILED, BrowserExit, BrowserLaunchFailedDetails,
};
use goodfetch_protocol::settings::{NETWORK_RESOLVE, TIMEOUT_CLOSE};
use tokio::process::{Child, Command};
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

/// The folder of the Run's browser profiles, in its folder: one per
/// browser, by its number in the Run's browser pool.
const PROFILES: &str = "browser-profiles";

/// The folder of a Chromium profile's default user, in its data directory.
const DEFAULT_PROFILE: &str = "Default";

/// A launched browser: its connection, how its process ended once it did,
/// and its executable.
pub(crate) struct Launched {
    /// The CDP connection over its pipe.
    pub(crate) connection: Connection,
    /// `None` while it runs; how it ended once it exited.
    pub(crate) exit: watch::Receiver<Option<BrowserExit>>,
    /// Its executable.
    pub(crate) executable: PathBuf,
    /// Whether it runs headless, so its pages need the identity it shows
    /// with a window.
    pub(crate) headless: bool,
    /// Its process's ID, by which its memory is read.
    pub(crate) process: Option<u32>,
}

/// The executable of Chrome for Testing's build for `platform`, in the
/// folder its archive unpacks to.
fn executable_in(files: &Path, platform: BuildPlatform) -> Option<PathBuf> {
    let mac = |folder: &str| {
        files
            .join(folder)
            .join("Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing")
    };
    Some(match platform {
        BuildPlatform::MacArm64 => mac("chrome-mac-arm64"),
        BuildPlatform::MacX64 => mac("chrome-mac-x64"),
        BuildPlatform::LinuxX64 => files.join("chrome-linux64").join("chrome"),
        BuildPlatform::WindowsX64 => files.join("chrome-win64").join("chrome.exe"),
        BuildPlatform::LinuxArm64 => return None,
    })
}

/// What a download of a missing build may do under the Run's settings.
fn fetching(run: &Run) -> Fetching {
    let Some(settings) = run.settings().map(RunSettings::resolved) else {
        return Fetching::new(run.clock());
    };
    let offline = settings
        .get(EGRESS_OFFLINE.name)
        .filter(|offline| offline.value == serde_json::Value::Bool(true))
        .map(|offline| offline.layer);
    let resolve = settings
        .hosts(NETWORK_RESOLVE.name)
        .into_iter()
        .fold(Resolve::new(), |mapping, (host, addresses)| {
            mapping.with(host, addresses)
        });
    Fetching {
        offline,
        resolve,
        delays: Delays::of(settings),
        ..Fetching::new(run.clock())
    }
}

/// The browser's executable, the build it holds while it runs, and its
/// kind, as `goodfetch.browser.running`'s `browser` label names it.
async fn executable(
    run: &Run,
    home: &Home,
    pins: &Pins,
) -> Result<(PathBuf, Option<HeldArtifact>, &'static str), CallError> {
    let choice = BrowserChoice::Default;
    match browser_source(Platform::current(), &choice, &BTreeMap::new()) {
        BrowserSource::Pinned {
            name,
            platform,
            notice,
        } => {
            notice_of(notice);
            let cache = BrowserCache::new(&CacheLocation::new(
                home.root(),
                home.browsers_path().map(Path::to_path_buf),
            ));
            let pinned = pins.browser(name, platform).ok_or_else(|| {
                CallError::new(
                    &BROWSER_NOT_INSTALLED,
                    format!("this version pins no {name} build for this platform"),
                    &BrowserNotInstalledDetails {
                        missing: MissingBrowser::Path {
                            path: cache.root().display().to_string(),
                        },
                        fix: "Name an installed browser by its path".to_owned(),
                    },
                )
            })?;
            let now = jiff::Timestamp::try_from(run.clock().wall()).unwrap_or_default();
            let prepared = cache
                .prepare(
                    pinned,
                    &pins.pinned_by,
                    &fetching(run),
                    now,
                    &mut |_progress| {},
                )
                .await
                .map_err(|error| error.to_call_error())?;
            let files = prepared.artifact.files();
            let executable = executable_in(&files, platform)
                .ok_or_else(|| launch_failed(&files, None, "no executable for this platform"))?;
            Ok((executable, Some(prepared.artifact), "chrome_for_testing"))
        }
        BrowserSource::Installed { candidates, notice } => {
            notice_of(notice);
            let path = installed_browser(&choice, &candidates, pins.pinned_by.product)
                .await
                .map_err(|error| error.to_call_error())?;
            Ok((path, None, "chromium"))
        }
    }
}

fn notice_of(notice: Option<PlatformNotice>) {
    if let Some(notice) = notice {
        tracing::info!(%notice, "the Run's browser is not the default build");
    }
}

fn launch_failed(executable: &Path, exit: Option<BrowserExit>, reason: &str) -> CallError {
    CallError::new(
        &BROWSER_LAUNCH_FAILED,
        format!(
            "the browser at {} did not start: {reason}",
            executable.display()
        ),
        &BrowserLaunchFailedDetails {
            executable: executable.display().to_string(),
            exit,
        },
    )
}

/// How a process ended.
fn exit_of(status: std::process::ExitStatus) -> BrowserExit {
    #[cfg(unix)]
    let signal = std::os::unix::process::ExitStatusExt::signal(&status);
    #[cfg(not(unix))]
    let signal = None;
    BrowserExit {
        code: status.code(),
        signal,
    }
}

/// Launches a browser of the Run from the Browser cache of `home`, under
/// the Run's settings, on the profile numbered `number` in the Run's
/// folder; its tasks are the Run's, and `stopping`, cancelled when the Run
/// stops, closes it.
///
/// # Errors
///
/// The Browser cache's codes for a missing build (`browser_not_installed`,
/// `artifact_unavailable_offline`, `artifact_download_failed`), and
/// `browser_launch_failed` for a process that does not start.
///
/// # Cancel safety
///
/// Dropping the future before the browser started leaves no process; after,
/// the Run's stop closes it.
pub(crate) async fn launch(
    run: &Run,
    home: &Home,
    pins: &Pins,
    number: u64,
    stopping: CancellationToken,
) -> Result<Launched, CallError> {
    let (executable, held, kind) = executable(run, home, pins).await?;
    let profile = run.folder().join(PROFILES).join(number.to_string());
    let created = profile.join(DEFAULT_PROFILE);
    run.owner()
        .blocking(
            #[expect(
                clippy::disallowed_methods,
                reason = "it runs on the tasks module's blocking policy"
            )]
            move || {
                let file = created.join("Preferences");
                std::fs::create_dir_all(&created)?;
                let existing = match std::fs::read_to_string(&file) {
                    Ok(text) => Some(text),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                    Err(error) => return Err(error),
                };
                std::fs::write(file, preferences::starting_preferences(existing.as_deref()))
            },
        )
        .await
        .map_err(|error| launch_failed(&executable, None, &error.to_string()))?;
    let settings = run.settings().map(RunSettings::resolved);
    let headless = settings
        .and_then(|settings| settings.boolean(BROWSER_HEADLESS.name))
        .unwrap_or(true);
    let resolve = settings
        .map(|settings| settings.hosts(NETWORK_RESOLVE.name))
        .unwrap_or_default();
    let built = arguments::arguments(&arguments::Launch {
        profile: &profile,
        headless,
        resolve: &resolve,
        shared_memory: shared_memory(run).await,
    });
    for warning in &built.warnings {
        tracing::warn!(%warning, "the Run's browser launches degraded");
    }
    let mut command = Command::new(&executable);
    command
        .args(&built.arguments)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let (connection, driver, child) =
        start(command).map_err(|error| launch_failed(&executable, None, &error.to_string()))?;
    let process = child.child.id();
    let (exits, exit) = watch::channel(None);
    run.spawn(supervise(
        (run.clone(), stopping),
        child,
        (connection.clone(), driver),
        exits,
        held,
        profile,
        run.metrics().browser_running(kind),
    ));
    Ok(Launched {
        connection,
        exit,
        executable,
        headless,
        process,
    })
}

/// The size of `/dev/shm`, where Linux mounts one of its own.
#[cfg(target_os = "linux")]
async fn shared_memory(run: &Run) -> Option<u64> {
    run.owner()
        .blocking(|| {
            #[expect(
                clippy::disallowed_methods,
                reason = "it runs on the tasks module's blocking policy"
            )]
            std::fs::read_to_string("/proc/self/mounts")
        })
        .await
        .ok()
        .and_then(|mounts| arguments::shared_memory_of(&mounts))
}

/// No `/dev/shm` to measure outside Linux.
#[cfg(not(target_os = "linux"))]
#[expect(
    clippy::unused_async,
    reason = "it stands in for Linux's measure, which reads a file off the async workers"
)]
async fn shared_memory(_run: &Run) -> Option<u64> {
    None
}

/// The driver of a browser's connection, which its supervisor runs.
type Driving = std::pin::Pin<Box<dyn Future<Output = goodfetch_cdp::Ended> + Send>>;

/// The browser's process, started through the engine's `children` module
/// in a process group of its own, which dies with the value (ADR 0024).
pub(crate) struct Process {
    child: Child,
    _group: Children,
}

/// Starts `command` with its debugging pipe: the browser's connection,
/// its driver, and its process.
#[cfg(unix)]
fn start(mut command: Command) -> std::io::Result<(Connection, Driving, Process)> {
    let (engine, browser) = pipe::attach(&mut command)?;
    let mut group = Children::new()?;
    let child = group.spawn(command)?;
    // The parent's copies of the browser's ends close now, so its end of the
    // pipe closes when it exits.
    drop(browser);
    let (connection, driver) = Connection::new(engine.reader, engine.writer);
    Ok((
        connection,
        Box::pin(driver.run()),
        Process {
            child,
            _group: group,
        },
    ))
}

#[cfg(not(unix))]
fn start(_command: Command) -> std::io::Result<(Connection, Driving, Process)> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "the debugging pipe is not built on this OS yet",
    ))
}

/// Runs the browser's connection, waits for its process to exit or for
/// `stopping`, closes it then, within the Run's `timeout.close`, after
/// which it is killed, and releases its build and profile.
async fn supervise(
    (run, stopping): (Run, CancellationToken),
    process: Process,
    (connection, driver): (Connection, Driving),
    exits: watch::Sender<Option<BrowserExit>>,
    held: Option<HeldArtifact>,
    profile: PathBuf,
    running: Counted,
) {
    let Process {
        mut child,
        _group: group,
    } = process;
    let close = Deadline::of_run(&run, &TIMEOUT_CLOSE);
    let clock = run.clock();
    let (_, status) = tokio::join!(driver, async {
        let status = tokio::select! {
            status = child.wait() => status,
            () = stopping.cancelled() => {
                let closing = clock.now();
                let _ = connection
                    .send_raw("Browser.close", None, serde_json::json!({}), close.duration())
                    .await;
                let exited = tokio::select! {
                    status = child.wait() => Some(status),
                    () = clock.sleep_until(closing + close.duration()) => None,
                };
                if let Some(status) = exited {
                    status
                } else {
                    let _ = child.start_kill();
                    child.wait().await
                }
            }
        };
        drop(connection);
        drop(running);
        status
    });
    let exit = status.ok().map(exit_of);
    tracing::info!(?exit, "a browser of the Run exited");
    exits.send_replace(Some(exit.unwrap_or(BrowserExit {
        code: None,
        signal: None,
    })));
    // The group goes with its last process, and the build is released.
    drop(group);
    drop(held);
    run.owner()
        .blocking(move || {
            #[expect(
                clippy::disallowed_methods,
                reason = "it runs on the tasks module's blocking policy"
            )]
            let _ = std::fs::remove_dir_all(profile);
        })
        .await;
}
