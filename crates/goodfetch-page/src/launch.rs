//! A Run's browser, launched by the engine (ADRs 0017, 0024, 0098 and
//! 0115): its build prepared in the Browser cache from the call that
//! launches it, so a missing build raises its code there and nowhere
//! earlier, held under a shared lock until the process exits; the process
//! started over its debugging pipe, never a debugging port, on a Browser
//! data folder of its own in the Run's folder; and its end: when the Run stops, or the
//! browser pool closes it, the browser is asked to close, then killed if it
//! does not, its data folder removed and its build released.

mod arguments;
mod gui_session;
#[cfg(windows)]
mod named_pipes;
#[cfg(unix)]
mod pipe;
mod preferences;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;

use goodfetch_cdp::{CommandLog, Connection};
use goodfetch_counted_work::CountedWork;
use goodfetch_engine::CallError;
use goodfetch_engine::browser_cache::{
    BrowserCache, BrowserChoice, BrowserSource, BuildPlatform, CHROME_HEADLESS_SHELL,
    CacheLocation, HeldArtifact, Pins, Platform, PlatformNotice, browser_source, installed_browser,
};
use goodfetch_engine::children::Children;
use goodfetch_engine::deadline::Deadline;
use goodfetch_engine::dns::HostMapping;
use goodfetch_engine::egress::Policy;
use goodfetch_engine::executables;
use goodfetch_engine::home::Home;
use goodfetch_engine::metrics::Counted;
use goodfetch_engine::pinned_download::Fetching;
use goodfetch_engine::runs::Run;
use goodfetch_engine::sandbox_probe::{self, Probed};
use goodfetch_engine::settings::{Resolved, RunSettings, get_or_default};
use goodfetch_engine::transport;
use goodfetch_protocol::browser_cache::{
    BROWSER_NOT_INSTALLED, BrowserNotInstalledDetails, MissingBrowser,
};
use goodfetch_protocol::page::{
    BROWSER_HEADLESS, BROWSER_HEADLESS_SHELL, BROWSER_LAUNCH_FAILED, BrowserExit,
    BrowserLaunchFailedDetails,
};
use goodfetch_protocol::security::{SECURITY_BROWSER_SANDBOX, SECURITY_RAW_CDP_EGRESS_PROXY};
use goodfetch_protocol::settings::{
    CAPABILITY_UNAVAILABLE, CapabilityUnavailableDetails, SettingsLayer, TIMEOUT_CLOSE,
};
use tokio::io::DuplexStream;
use tokio::process::{Child, Command};
use tokio::sync::{mpsc, watch};
use tokio_util::sync::CancellationToken;

use crate::trace;

/// The folder of the Run's Browser data folders, in its folder: one per
/// browser, by its number in the Run's browser pool.
const DATA_FOLDERS: &str = "browser-data";

/// The folder of a Chromium profile's default user, in its data directory.
const DEFAULT_PROFILE: &str = "Default";

/// What Chromium adds below its temporary folder for its own socket, which
/// it stops on when that path is too long for the OS: the longest of the
/// browsers the engine starts (Chrome's folder is `.com.google.Chrome.*`).
const CHROMIUM_SOCKET: &str = "/.org.chromium.Chromium.XXXXXX/SingletonSocket";

/// The size of a scripted browser's pipe, each way.
const SCRIPTED_PIPE: usize = 1 << 16;

/// How many browsers a scripted launcher hands on before the one playing
/// them took the first: a launch waits for room past it.
const SCRIPTED_HANDOVERS: usize = 1;

/// The executable a scripted browser names, where a launched one names its
/// path.
const SCRIPTED: &str = "scripted";

/// How the engine starts a Run's browser, a value its composition chooses
/// (ADR 0059): the Browser cache's build, as a process over its debugging
/// pipe, or a scripted browser, which a benchmark or a test plays on the
/// other end of an in-memory pipe, with no process, Browser data folder or
/// build of its own.
#[derive(Clone, Debug)]
pub enum Launcher {
    /// The Browser cache's build, or the installed browser (ADR 0017).
    Executable,
    /// A scripted browser: [`Launcher::scripted`].
    Scripted(Scripted),
}

/// Where a scripted launcher hands each browser it launches, and the
/// process that browser's memory is read from.
#[derive(Clone, Debug)]
pub struct Scripted {
    browsers: mpsc::Sender<DuplexStream>,
    process: Option<u32>,
}

/// The browsers a scripted launcher launched, each its end of its pipe: the
/// engine writes each command as JSON followed by a NUL byte, as it writes
/// to Chromium's debugging pipe, and reads the answers and events so.
#[derive(Debug)]
pub struct ScriptedBrowsers(mpsc::Receiver<DuplexStream>);

impl ScriptedBrowsers {
    /// The next browser launched; `None` once the launcher is gone, with
    /// the engine that held it.
    ///
    /// # Cancel safety
    ///
    /// Cancel-safe: a browser not yet taken stays for the next call.
    pub async fn next(&mut self) -> Option<DuplexStream> {
        self.0.recv().await
    }
}

impl Launcher {
    /// A launcher of scripted browsers, each handed to the
    /// [`ScriptedBrowsers`] it answers with as it launches, whose memory
    /// the browser pool reads from `process`, as it reads a launched
    /// browser's from its process's (ADR 0140's C7): none walks nothing.
    #[must_use]
    pub fn scripted(process: Option<u32>) -> (Self, ScriptedBrowsers) {
        let (browsers, launched) = mpsc::channel(SCRIPTED_HANDOVERS);
        (
            Self::Scripted(Scripted { browsers, process }),
            ScriptedBrowsers(launched),
        )
    }
}

/// A launched browser: its connection, how its process ended once it did,
/// and its executable.
pub(crate) struct Launched {
    /// The CDP connection over its pipe.
    pub(crate) connection: Connection,
    /// `None` while it runs; how it ended once it exited.
    pub(crate) exit: watch::Receiver<Option<BrowserExit>>,
    /// Its executable.
    pub(crate) executable: PathBuf,
    /// Whether it runs the full build headless, so its pages need the
    /// identity it shows with a window. The headless shell has no window
    /// and no secure page of its own to read its client hints on, and
    /// shows its own identity (#3224).
    pub(crate) headless: bool,
    /// Its process's ID, by which its memory is read.
    pub(crate) process: Option<u32>,
    /// Cancelled once its process exited and its data folder is gone, so
    /// another launch may take the folder.
    pub(crate) released: CancellationToken,
}

/// The executable of Chrome for Testing's build for `platform`, its
/// headless shell when `shell`, in the folder its archive unpacks to.
fn executable_in(files: &Path, platform: BuildPlatform, shell: bool) -> Option<PathBuf> {
    let executable = if shell {
        platform.chrome_headless_shell_executable()
    } else {
        platform.chrome_for_testing_executable()
    };
    executable.map(|executable| {
        executable
            .split('/')
            .fold(files.to_path_buf(), |path, part| path.join(part))
    })
}

/// `capability_unavailable` for `browser.headless_shell`, set in `layer`,
/// where the engine runs an installed Chromium, which has no headless
/// shell.
fn no_shell(layer: SettingsLayer) -> CallError {
    CallError::new(
        &CAPABILITY_UNAVAILABLE,
        format!(
            "Chrome for Testing builds no headless shell for this platform, where the engine runs the distribution's Chromium: set `{}: false`.",
            BROWSER_HEADLESS_SHELL.name
        ),
        &CapabilityUnavailableDetails {
            key: BROWSER_HEADLESS_SHELL.name.to_owned(),
            layer,
            kind: CHROME_HEADLESS_SHELL.to_owned(),
            fix: format!("`{}: false`", BROWSER_HEADLESS_SHELL.name),
        },
    )
}

/// What a download of a missing build may do under the Run's settings.
async fn fetching(run: &Run) -> Fetching {
    match run.settings() {
        Some(settings) => Fetching::of(settings.resolved(), run.clock(), run.owner()).await,
        None => Fetching::new(run.clock()),
    }
}

/// The browser's executable, the build it holds while it runs, and its
/// kind, as `goodfetch.browser.running`'s `browser` label names it: the
/// pinned release's headless shell under `browser.headless_shell`, which
/// `shell` gives with the layer that set it.
async fn executable(
    run: &Run,
    (home, pins): (&Home, &Pins),
    shell: Option<SettingsLayer>,
) -> Result<(PathBuf, Option<HeldArtifact>, &'static str), CallError> {
    let choice = BrowserChoice::Default;
    match browser_source(Platform::current(), &choice, &BTreeMap::new()) {
        BrowserSource::Pinned {
            name,
            platform,
            notice,
        } => {
            notice_of(notice);
            let (name, kind) = if shell.is_some() {
                (CHROME_HEADLESS_SHELL, "chrome_headless_shell")
            } else {
                (name, "chrome_for_testing")
            };
            let cache = BrowserCache::new(
                &CacheLocation::new(home.root(), home.browsers_path().map(Path::to_path_buf)),
                run.owner().blocking_threads(),
            );
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
            let now = run.clock().timestamp();
            let fetching = fetching(run).await;
            let prepared = cache
                .prepare(pinned, &pins.pinned_by, &fetching, now, &mut |_progress| {})
                .await
                .map_err(|error| error.to_call_error())?;
            let files = prepared.artifact.files();
            let executable = executable_in(&files, platform, shell.is_some())
                .ok_or_else(|| launch_failed(&files, None, "no executable for this platform"))?;
            Ok((executable, Some(prepared.artifact), kind))
        }
        BrowserSource::Installed { candidates, notice } => {
            if let Some(layer) = shell {
                return Err(no_shell(layer));
            }
            notice_of(notice);
            let path = installed_browser(
                &run.owner().blocking_threads(),
                &choice,
                &candidates,
                pins.pinned_by.product,
            )
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

/// The CDP command log of `run`'s browsers, as [`trace::command_log`]
/// gives it, for the browser at `executable`.
///
/// # Errors
///
/// `browser_launch_failed` when the Run trace's writer cannot start.
async fn command_log(run: &Run, executable: &Path) -> Result<Option<CommandLog>, CallError> {
    trace::command_log(run).await.map_err(|error| {
        launch_failed(
            executable,
            None,
            &format!("the Run trace's CDP log cannot start: {error}"),
        )
    })
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
/// the Run's settings, on the Browser data folder numbered `number` in the Run's
/// folder, trusting `relay_authority`, the egress relay's key hash, when the Run has one;
/// its tasks are the Run's, and `stopping`, cancelled when the Run stops,
/// closes it. A scripted `launcher` hands its browser on instead.
///
/// # Errors
///
/// `browser_needs_gui_session` for the full build headless on macOS with
/// no GUI session (#3224); `capability_unavailable` for
/// `browser.headless_shell` where the engine runs an installed Chromium;
/// `browser_sandbox_unavailable` where Chromium's sandbox, on, cannot start
/// (ADR 0049); the Browser cache's codes for a missing build
/// (`browser_not_installed`, `artifact_unavailable_offline`,
/// `artifact_download_failed`),
/// `executable_blocked_by_smart_app_control` where Windows' Smart App Control
/// or another App Control policy refuses the browser (ADR 0039), and
/// `browser_launch_failed` for a process that does not start otherwise.
///
/// # Cancel safety
///
/// Dropping the future before the browser started leaves no process; after,
/// the Run's stop closes it.
pub(crate) async fn launch(
    run: &Run,
    (launcher, home, pins): (&Launcher, &Home, &Pins),
    (number, relay_authority): (u64, Option<&str>),
    stopping: CancellationToken,
) -> Result<Launched, CallError> {
    if let Launcher::Scripted(scripted) = launcher {
        return launch_scripted(run, scripted, stopping).await;
    }
    let settings = run.settings().map(RunSettings::resolved);
    let headless = get_or_default(settings, &BROWSER_HEADLESS);
    let shell = get_or_default::<bool>(settings, &BROWSER_HEADLESS_SHELL).then(|| {
        settings
            .and_then(|settings| settings.effective(&BROWSER_HEADLESS_SHELL))
            .map_or(SettingsLayer::Default, |effective| effective.layer)
    });
    if let Some(refusal) =
        gui_session::refusal(shell.is_some(), headless, gui_session::graphic_access)
    {
        return Err(refusal);
    }
    // Chromium's sandbox stays on unless switched off; where the startup
    // probe refused it, no browser starts (ADR 0049).
    let sandbox = get_or_default(settings, &SECURITY_BROWSER_SANDBOX);
    let probed = if sandbox {
        let probe = run.sandbox_probe().clone();
        Some(run.owner().blocking(move || probe.probed().clone()).await)
    } else {
        None
    };
    if let Some(refusal) = probed.as_ref().and_then(Probed::refusal) {
        return Err(refusal);
    }
    let (executable, held, kind) = executable(run, (home, pins), shell).await?;
    let data_folder = run.folder().join(DATA_FOLDERS).join(number.to_string());
    write_preferences(run, &data_folder)
        .await
        .map_err(|error| launch_failed(&executable, None, &error.to_string()))?;
    let (arguments, environment) = command_line(
        run,
        settings,
        &data_folder,
        (headless, shell.is_some()),
        sandbox,
        relay_authority,
    )
    .await;
    let mut command = Command::new(&executable);
    command
        .args(&arguments)
        .envs(environment.iter().copied())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    // A temporary folder too deep for Chromium's socket is replaced by
    // `/tmp`, where it fits.
    let temporary = std::env::temp_dir();
    let browsers = transport::temporary_folder_for_sockets(&temporary, CHROMIUM_SOCKET.len());
    if browsers != temporary {
        command.env("TMPDIR", browsers);
    }
    let log = command_log(run, &executable).await?;
    let (connection, driver, mut child) = start(command, run.metrics().counted_work().clone(), log)
        .await
        .map_err(|error| {
            executables::blocked_by_smart_app_control(&executable, &error)
                .unwrap_or_else(|| launch_failed(&executable, None, &error.to_string()))
        })?;
    let process = child.child.id();
    // Under AppArmor's restriction, the label the browser started under
    // says whether its sandbox can start: one the restriction holds is
    // closed before it fails on its own.
    if matches!(probed, Some(Probed::Restricted))
        && let Some(label) = process.and_then(sandbox_probe::label_of)
        && sandbox_probe::label_holds_user_namespaces(&label)
    {
        let _ = child.child.start_kill();
        let _ = child.child.wait().await;
        drop((connection, driver, child, held));
        return Err(sandbox_probe::apparmor_refusal(&executable, &label));
    }
    let (exits, exit) = watch::channel(None);
    let released = CancellationToken::new();
    run.spawn(supervise(Supervised {
        run: run.clone(),
        stopping,
        process: child,
        connection: connection.clone(),
        driver,
        exits,
        held,
        data_folder,
        running: run.metrics().browser_running(kind),
        released: released.clone(),
    }));
    Ok(Launched {
        connection,
        exit,
        executable,
        headless: headless && shell.is_none(),
        process,
        released,
    })
}

/// Launches a scripted browser of the Run: its end of a new pipe goes to
/// `scripted`'s [`ScriptedBrowsers`] once the one handed on before was
/// taken, and it shows headless or not as the
/// Run's settings say; `stopping` asks it to close, as it asks a launched
/// one, within the Run's `timeout.close`.
///
/// # Errors
///
/// `browser_launch_failed` once the [`ScriptedBrowsers`] are gone.
async fn launch_scripted(
    run: &Run,
    scripted: &Scripted,
    stopping: CancellationToken,
) -> Result<Launched, CallError> {
    let executable = Path::new(SCRIPTED);
    let (engine, browser) = tokio::io::duplex(SCRIPTED_PIPE);
    scripted
        .browsers
        .send(browser)
        .await
        .map_err(|_| launch_failed(executable, None, "no one plays the scripted browser"))?;
    let (reader, writer) = tokio::io::split(engine);
    let log = command_log(run, executable).await?;
    let (connection, driver) =
        Connection::new(reader, writer, run.metrics().counted_work().clone(), log);
    let (exits, exit) = watch::channel(None);
    let released = CancellationToken::new();
    run.spawn(supervise_scripted(
        run.clone(),
        (connection.clone(), Box::pin(driver.run())),
        stopping,
        (exits, released.clone()),
    ));
    Ok(Launched {
        connection,
        exit,
        executable: executable.to_path_buf(),
        headless: get_or_default(run.settings().map(RunSettings::resolved), &BROWSER_HEADLESS),
        process: scripted.process,
        released,
    })
}

/// Runs a scripted browser's connection until it ends or `stopping`, which
/// asks it to close within the Run's `timeout.close`; then says how it
/// ended, a clean exit once asked to close, and releases it.
async fn supervise_scripted(
    run: Run,
    (connection, driver): (Connection, Driving),
    stopping: CancellationToken,
    (exits, released): (watch::Sender<Option<BrowserExit>>, CancellationToken),
) {
    let close = Deadline::of_resolved(run.settings().map(RunSettings::resolved), &TIMEOUT_CLOSE);
    let closed = async {
        stopping.cancelled().await;
        let _ = connection
            .send_raw(
                "Browser.close",
                None,
                serde_json::json!({}),
                close.duration(),
            )
            .await;
    };
    tokio::select! {
        _ = driver => {}
        () = closed => {}
    }
    drop(connection);
    exits.send_replace(Some(BrowserExit {
        code: stopping.is_cancelled().then_some(0),
        signal: None,
    }));
    released.cancel();
}

/// Writes the starting preferences of the default profile in
/// `data_folder`, keeping every other preference the profile already holds.
async fn write_preferences(run: &Run, data_folder: &Path) -> std::io::Result<()> {
    let created = data_folder.join(DEFAULT_PROFILE);
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
}

/// The browser's command line under the Run's `settings`, on `data_folder`,
/// and the variables it starts with beside the engine's own; the warnings
/// of a degraded launch are logged.
async fn command_line(
    run: &Run,
    settings: Option<&Resolved>,
    data_folder: &Path,
    (headless, headless_shell): (bool, bool),
    sandbox: bool,
    relay_authority: Option<&str>,
) -> (Vec<String>, &'static [(&'static str, &'static str)]) {
    let resolve = settings.map(HostMapping::of).unwrap_or_default();
    // A policy that is no policy opens no page anyway (`setting_invalid`).
    // Behind the egress relay, which every Session's context takes as its
    // proxy, the relay holds what the script would.
    let relayed: bool = get_or_default(settings, &SECURITY_RAW_CDP_EGRESS_PROXY);
    let proxy_script = settings
        .filter(|_| !relayed)
        .and_then(|settings| Policy::of(settings).ok())
        .and_then(|policy| policy.proxy_script());
    let built = arguments::arguments(&arguments::Launch {
        data_folder,
        headless,
        headless_shell,
        resolve: &resolve,
        shared_memory: shared_memory(run).await,
        proxy_script: proxy_script.as_deref(),
        sandbox,
        relay_authority,
    });
    for warning in &built.warnings {
        tracing::warn!(%warning, "the Run's browser launches degraded");
    }
    (built.command_line, built.environment)
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
/// which records each command's method in `log`, if any, its driver, and
/// its process.
#[cfg(unix)]
#[expect(
    clippy::unused_async,
    reason = "Windows' start awaits its pipe's connection, and both have one signature"
)]
async fn start(
    mut command: Command,
    work: CountedWork,
    log: Option<CommandLog>,
) -> std::io::Result<(Connection, Driving, Process)> {
    let (engine, browser) = pipe::attach(&mut command)?;
    let mut group = Children::new()?;
    let child = group.spawn(command)?;
    // The parent's copies of the browser's ends close now, so its end of the
    // pipe closes when it exits.
    drop(browser);
    let (connection, driver) = Connection::new(engine.reader, engine.writer, work, log);
    Ok((
        connection,
        Box::pin(driver.run()),
        Process {
            child,
            _group: group,
        },
    ))
}

/// Starts `command` with its debugging pipe, as on Unix: the browser
/// inherits its ends of two named pipes, which `--remote-debugging-io-pipes`
/// names, and the parent's copies close once it started.
#[cfg(windows)]
async fn start(
    mut command: Command,
    work: CountedWork,
    log: Option<CommandLog>,
) -> std::io::Result<(Connection, Driving, Process)> {
    let pipe = named_pipes::debugging_pipe().await?;
    command.arg(pipe.browser.switch()?);
    let mut group = Children::new()?;
    let child = group.spawn_inheriting(command, pipe.browser.into_handles().into())?;
    let (connection, driver) = Connection::new(pipe.reader, pipe.writer, work, log);
    Ok((
        connection,
        Box::pin(driver.run()),
        Process {
            child,
            _group: group,
        },
    ))
}

/// What the supervisor of one launched browser holds until it exits.
struct Supervised {
    /// The browser's Run.
    run: Run,
    /// Cancelled when the Run stops or the pool closes the browser.
    stopping: CancellationToken,
    /// Its process.
    process: Process,
    /// Its connection, through which it is asked to close.
    connection: Connection,
    /// The connection's driver, which the supervisor runs.
    driver: Driving,
    /// Told how the process ended, once it did.
    exits: watch::Sender<Option<BrowserExit>>,
    /// The build it runs, held until it exits.
    held: Option<HeldArtifact>,
    /// Its Browser data folder, removed once it exited.
    data_folder: PathBuf,
    /// Its count among the running browsers.
    running: Counted,
    /// Cancelled once it exited and its data folder is gone.
    released: CancellationToken,
}

/// Runs the browser's connection, waits for its process to exit or for
/// `stopping`, closes it then, within the Run's `timeout.close`, after
/// which it is killed, and releases its build and data folder.
async fn supervise(supervised: Supervised) {
    let Supervised {
        run,
        stopping,
        process,
        connection,
        driver,
        exits,
        held,
        data_folder,
        running,
        released,
    } = supervised;
    let Process {
        mut child,
        _group: group,
    } = process;
    let close = Deadline::of_resolved(run.settings().map(RunSettings::resolved), &TIMEOUT_CLOSE);
    let clock = run.clock();
    let (_, status) = tokio::join!(driver, async {
        let status = tokio::select! {
            status = child.wait() => status,
            () = stopping.cancelled() => {
                // Asked to close, then waited for, all within the deadline,
                // whatever its connection does: a browser that no longer
                // reads its pipe is killed at the deadline all the same.
                let closing = clock.now();
                let exited = tokio::select! {
                    status = async {
                        let _ = connection
                            .send_raw("Browser.close", None, serde_json::json!({}), close.duration())
                            .await;
                        child.wait().await
                    } => Some(status),
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
            let _ = std::fs::remove_dir_all(data_folder);
        })
        .await;
    released.cancel();
}
