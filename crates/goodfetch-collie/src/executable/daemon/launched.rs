//! The Daemon's process (ADRs 0024, 0031 and 0098): started by a Client
//! or `daemon start` from the pick's versioned copy, detached from the
//! starter's process group, or by the major's service, which gives its
//! place to the pick when it is not the pick and hands the socket its
//! service manager listens on to the Daemon. It waits for the serving
//! lock, listens or takes that socket, publishes its runtime file and
//! prints it, serves until it stops, and starts the pick when it hands
//! over, except under a service, whose next connection starts the pick.
#![expect(
    clippy::disallowed_methods,
    reason = "the Daemon's process reads the machine's files and starts processes before and outside any async worker (ADR 0060)"
)]

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::io::{BufRead as _, BufReader};
use std::path::{Path, PathBuf};
use std::process::{ExitCode, Stdio};
use std::sync::Arc;
use std::time::Duration;

use goodfetch_engine::browser_cache::{self, MaxUnusedAge};
use goodfetch_engine::children::give_huge_pages_as_started;
use goodfetch_engine::clock::Clock;
use goodfetch_engine::daemon::files::{DaemonFiles, FileLock};
use goodfetch_engine::daemon::service::LaunchedCommand;
use goodfetch_engine::daemon::{DaemonEnd, DaemonOptions, Drain, Holds, Succession};
use goodfetch_engine::endpoint::new_token;
use goodfetch_engine::home::Home;
use goodfetch_engine::installed::{InstalledList, Seen, pick, stores_floor};
use goodfetch_engine::ops::Ops;
use goodfetch_engine::run_state::install::InstallDatabase;
use goodfetch_engine::settings::{Resolved, get_or_default};
use goodfetch_engine::transport::{DaemonListener, Network};
use goodfetch_engine::{Dependencies, Engine, Identity};
use goodfetch_protocol::browser_cache::{ArtifactId, ArtifactKind};
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::daemon::{
    DRAIN_GRACE, DaemonEndpoint, DaemonRuntimeFile, DaemonToken, IDLE_TIMEOUT,
};

use super::super::runtime;
use super::{Failure, Machine, now_of, per_install, unavailable};
use crate::executable::output;

/// Starts the Daemon's process from `executable`, the versioned copy of
/// build `build`, and waits for the runtime file it prints once ready.
pub(super) fn launch(
    executable: &Path,
    build: &str,
    keep_alive: bool,
    environment: &BTreeMap<OsString, OsString>,
    home: &Home,
) -> Result<DaemonRuntimeFile, Failure> {
    std::fs::create_dir_all(home.root()).map_err(unavailable("Goodfetch home cannot be made"))?;
    let mut command = std::process::Command::new(executable);
    command
        .args(LaunchedCommand::started(build, keep_alive).arguments())
        .env_clear()
        .envs(environment)
        .current_dir(home.root())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    detach(&mut command);
    give_huge_pages_as_started(&mut command);
    let mut child = command
        .spawn()
        .map_err(unavailable("the Daemon's process cannot start"))?;
    let stdout = child
        .stdout
        .take()
        .expect("invariant: the Daemon's stdout is piped");
    let mut ready = String::new();
    let read = BufReader::new(stdout).read_line(&mut ready);
    // The Daemon outlives this process: it is never waited for here.
    drop(child);
    match read {
        Ok(read) if read > 0 => serde_json::from_str(&ready).map_err(|error| {
            Failure::Unavailable(format!(
                "the Daemon printed no runtime file once ready: {error}"
            ))
        }),
        _ => Err(Failure::Unavailable(format!(
            "the Daemon exited before it was ready; its log is in {}",
            home.logs().display()
        ))),
    }
}

/// Starts the Daemon's process outside this process's group, so a signal
/// to this command's terminal never reaches it.
#[cfg(unix)]
fn detach(command: &mut std::process::Command) {
    use std::os::unix::process::CommandExt as _;
    command.process_group(0);
}

#[cfg(windows)]
fn detach(command: &mut std::process::Command) {
    use std::os::windows::process::CommandExt as _;
    const DETACHED_PROCESS: u32 = 0x0000_0008;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    command.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW);
}

#[cfg(not(any(unix, windows)))]
fn detach(_command: &mut std::process::Command) {}

/// The Daemon's process, started by [`launch`] from the copy of build
/// `copy`, or by the major's service under `service`: waits for the serving
/// lock, listens, or takes the socket its service manager hands over,
/// publishes its runtime file and prints it, serves until it stops, and
/// starts the pick when it hands over. A service's process that is not the
/// pick gives its place to the pick first.
pub(in crate::executable) fn launched(
    engine: &Engine,
    dependencies: Dependencies,
    machine: &Machine<'_>,
    copy: Option<&str>,
    keep_alive: bool,
    service: bool,
) -> ExitCode {
    // A Daemon starts only on machine settings it can read: a failure is
    // its log's, with their Error code's exit.
    let settings = match machine.settings() {
        Ok(settings) => settings,
        Err(failure) => return failure.report(),
    };
    if service && let Some(exit) = give_place_to_the_pick(machine) {
        return exit;
    }
    let files = machine.files();
    let identity = engine.identity();
    let _copy = copy.and_then(|build| {
        let id = ArtifactId {
            kind: ArtifactKind::ExecutableCopy,
            name: identity.product.executable_name().to_owned(),
            build: build.to_owned(),
        };
        browser_cache::hold_copy(&machine.cache(), &id)
            .ok()
            .flatten()
    });
    let serving = match files.serving_lock() {
        Ok(serving) => serving,
        Err(error) => {
            return unavailable("the Daemon's serving lock cannot be taken")(error).report();
        }
    };
    let idle_timeout: Duration = get_or_default(Some(settings), &IDLE_TIMEOUT);
    let drain_grace: Duration = get_or_default(Some(settings), &DRAIN_GRACE);
    let install = match install_database(machine, &dependencies, Some(settings)) {
        Ok(install) => install,
        Err(failure) => return failure.report(),
    };
    let (socket, _socket_folder) = match files.socket_place() {
        Ok(place) => place,
        Err(error) => return unavailable("the Daemon's socket has no folder")(error).report(),
    };
    // An installed service keeps its major's Daemon running (ADR 0024).
    let installed = files.service().ok().flatten().is_some();
    let pid = std::process::id();
    let runtime = runtime::multi_thread(runtime::Threads::of(Some(settings)));
    let ended = runtime.block_on(async {
        let (listener, endpoint, token, activated) =
            take_listener(machine, &files, &socket, service).await?;
        let published = DaemonRuntimeFile {
            endpoint: endpoint.clone(),
            token: DaemonToken::new(token.clone()),
            version: identity.product_version.clone(),
            pid,
        };
        files
            .publish(&published)
            .map_err(unavailable("the Daemon's runtime file cannot be written"))?;
        output::json(&published);
        tracing::info!(%endpoint, pid, "the Daemon serves");
        let options = DaemonOptions {
            token,
            endpoint,
            pid,
            idle_timeout,
            keep_alive: keep_alive || activated || installed,
            holds: Holds::new(),
            succession: Arc::new(OnTheMachine::of(
                machine,
                identity,
                files.clone(),
                pid,
                activated,
            )),
            drain: Drain {
                asked: None,
                grace: drain_grace,
            },
            install,
            ops: Ops::default(),
            websocket: None,
        };
        Ok::<_, Failure>((
            engine.run_daemon(listener, dependencies, options).await,
            activated,
        ))
    });
    runtime.shutdown_background();
    match ended {
        Ok((ended, activated)) => finish(machine, &files, pid, serving, ended, activated),
        Err(failure) => failure.report(),
    }
}

/// The install database, which the Daemon alone writes, in the
/// per-install layout that the machine's `settings` choose (ADR 0002);
/// none in the per-run layout.
pub(super) fn install_database(
    machine: &Machine<'_>,
    dependencies: &Dependencies,
    settings: Option<&Resolved>,
) -> Result<Option<InstallDatabase>, Failure> {
    if !per_install(settings) {
        return Ok(None);
    }
    InstallDatabase::of(
        machine.home,
        &machine.identity.product_version,
        &dependencies.run_state_location,
        &dependencies.run_state,
    )
    .map(Some)
    .map_err(|error| {
        Failure::Unavailable(format!(
            "the per-install layout's run state cannot be opened: {error}"
        ))
    })
}

/// The Daemon's listener, endpoint and token: the socket the service
/// manager handed over, with the persistent token, when `service` and one
/// did (`true`), otherwise a socket of its own at `socket`, with a token
/// of this launch.
async fn take_listener(
    machine: &Machine<'_>,
    files: &DaemonFiles,
    socket: &Path,
    service: bool,
) -> Result<(DaemonListener, DaemonEndpoint, String, bool), Failure> {
    let handed = if service {
        Network::Os
            .activated(machine.environment)
            .map_err(unavailable("the service manager's socket cannot be taken"))?
    } else {
        None
    };
    if let Some((listener, endpoint)) = handed {
        let token = files
            .persistent_token()
            .map_err(unavailable("the Daemon's persistent token cannot be read"))?;
        return Ok((listener, endpoint, token, true));
    }
    let (listener, endpoint) = Network::Os
        .listen(socket)
        .await
        .map_err(unavailable("the Daemon cannot listen"))?;
    let token = new_token().map_err(|error| {
        Failure::Unavailable(format!("the OS generator gave no token: {error}"))
    })?;
    Ok((listener, endpoint, token, false))
}

/// What a Daemon does once it stopped: unpublishes itself, releases its
/// serving lock, and starts the pick when it handed over.
fn finish(
    machine: &Machine<'_>,
    files: &DaemonFiles,
    pid: u32,
    serving: FileLock,
    ended: DaemonEnd,
    activated: bool,
) -> ExitCode {
    // The socket a service manager handed over stays its own.
    let unpublished = if activated {
        files.unpublish_activated(pid)
    } else {
        files.unpublish(pid)
    };
    if let Err(error) = unpublished {
        tracing::warn!(%error, "the Daemon's runtime file cannot be removed");
    }
    drop(serving);
    match ended {
        // The next connection to the service's socket starts the pick.
        DaemonEnd::HandedOver(start_lock) if activated => {
            drop(start_lock);
            ExitCode::SUCCESS
        }
        DaemonEnd::HandedOver(start_lock) => {
            let started = machine.start_pick(false);
            drop(start_lock);
            match started {
                Ok(_) => ExitCode::SUCCESS,
                Err(failure) => failure.report(),
            }
        }
        DaemonEnd::ListenerFailed | DaemonEnd::Panicked => ExitCode::FAILURE,
        DaemonEnd::Idle | DaemonEnd::Stopped | DaemonEnd::Drained => ExitCode::SUCCESS,
    }
}

/// The pick and the handover on this machine (ADR 0098).
struct OnTheMachine {
    files: DaemonFiles,
    home: Home,
    cache: PathBuf,
    product: Product,
    version: goodfetch_protocol::connection::ProductVersion,
    max_unused_age: MaxUnusedAge,
    clock: Clock,
    pid: u32,
    /// Whether its socket is the service manager's, which stays.
    keeps_socket: bool,
}

impl OnTheMachine {
    /// The pick and handover of the Daemon `pid` that `identity` runs on
    /// `machine`, whose runtime files are `files`.
    fn of(
        machine: &Machine<'_>,
        identity: &Identity,
        files: DaemonFiles,
        pid: u32,
        keeps_socket: bool,
    ) -> Self {
        Self {
            files,
            home: machine.home.clone(),
            cache: machine.cache(),
            product: identity.product,
            version: identity.product_version.clone(),
            max_unused_age: machine.max_unused_age(),
            clock: machine.clock,
            pid,
            keeps_socket,
        }
    }
}

impl Succession for OnTheMachine {
    fn is_pick(&self) -> bool {
        let entries = InstalledList::of(&self.home).read().unwrap_or_default();
        let seen: Vec<Seen<'_>> = entries
            .iter()
            .map(|entry| Seen::look(entry, &self.cache))
            .collect();
        let floor = stores_floor(&self.home, &self.version);
        pick(
            &seen,
            &self.version,
            floor,
            now_of(self.clock),
            self.max_unused_age,
        )
        .is_none_or(|picked| {
            picked.product == self.product && picked.product_version == self.version
        })
    }

    fn begin_handover(&self) -> Box<dyn Send> {
        let lock: Option<FileLock> = match self.files.start_lock() {
            Ok(lock) => Some(lock),
            Err(error) => {
                tracing::warn!(%error, "the Daemon hands over without the start lock");
                None
            }
        };
        let unpublished = if self.keeps_socket {
            self.files.unpublish_activated(self.pid)
        } else {
            self.files.unpublish(self.pid)
        };
        if let Err(error) = unpublished {
            tracing::warn!(%error, "the Daemon's runtime file cannot be removed");
        }
        tracing::info!("the Daemon hands over to its major's pick");
        Box::new(lock)
    }
}

/// A service's `daemon launched` that is not its major's pick: on macOS
/// and Linux it runs the pick's versioned copy in its place, the same
/// process, so the socket its service manager handed over and the process
/// it watches become the pick's. A logon task, which hands over no socket,
/// starts the pick unless a Daemon runs. `None` when this executable is
/// the pick, or the logon task has nothing to do but serve.
#[cfg(unix)]
fn give_place_to_the_pick(machine: &Machine<'_>) -> Option<ExitCode> {
    use std::os::unix::process::CommandExt as _;
    let picked = machine.other_pick()?;
    let (executable, build) = match machine.copy_of(&picked) {
        Ok(copy) => copy,
        Err(failure) => return Some(failure.report()),
    };
    tracing::info!(
        product = picked.product.as_str(),
        version = %picked.product_version,
        "the service starts its major's pick in its place"
    );
    let mut command = std::process::Command::new(&executable);
    command
        .args(LaunchedCommand::of_service(&build, Vec::new()).arguments())
        .env_clear()
        .envs(machine.environment)
        .current_dir(machine.home.root());
    give_huge_pages_as_started(&mut command);
    let error = command.exec();
    Some(unavailable("the pick's versioned copy cannot start")(error).report())
}

#[cfg(not(unix))]
#[expect(
    clippy::unnecessary_wraps,
    reason = "on Unix the pick serves in place, here the logon task always ends: one signature serves both"
)]
fn give_place_to_the_pick(machine: &Machine<'_>) -> Option<ExitCode> {
    let files = machine.files();
    let start = match files.start_lock() {
        Ok(start) => start,
        Err(error) => {
            return Some(unavailable("the Daemon's start lock cannot be taken")(error).report());
        }
    };
    let exit = match machine.running() {
        Ok(Some(_)) => ExitCode::SUCCESS,
        Ok(None) => match machine.start_pick(true) {
            Ok(_) => ExitCode::SUCCESS,
            Err(failure) => failure.report(),
        },
        Err(failure) => failure.report(),
    };
    drop(start);
    Some(exit)
}
