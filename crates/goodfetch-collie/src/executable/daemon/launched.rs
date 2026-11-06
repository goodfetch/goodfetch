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
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::{ExitCode, Stdio};
use std::sync::Arc;
use std::time::Duration;

use goodfetch_engine::browser_cache::{self, MaxUnusedAge};
use goodfetch_engine::clock::Clock;
use goodfetch_engine::daemon::files::{DaemonFiles, FileLock};
use goodfetch_engine::daemon::{
    ContentOrigin, DaemonConsole, DaemonEnd, DaemonOptions, Holds, Succession,
};
use goodfetch_engine::endpoint::new_token;
use goodfetch_engine::home::Home;
use goodfetch_engine::installed::{InstalledList, Seen, pick, stores_floor};
use goodfetch_engine::ops::Ops;
use goodfetch_engine::people::SignInPolicy;
use goodfetch_engine::run_state::install::InstallDatabase;
use goodfetch_engine::settings::Resolved;
use goodfetch_engine::transport::{
    ConsoleListener, DaemonListener, Network, activated_console, listen_console,
};
use goodfetch_engine::{Dependencies, Engine, Identity};
use goodfetch_protocol::browser_cache::{ArtifactId, ArtifactKind};
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::console::{
    ADDRESS as CONSOLE_ADDRESS, CONSOLE_PORT_IN_USE, CONTENT_ADDRESS, CONTENT_PUBLIC_URL,
    ConsoleFailure, ConsoleState, PUBLIC_URL, default_address,
};
use goodfetch_protocol::daemon::{DaemonEndpoint, DaemonRuntimeFile, DaemonToken, IDLE_TIMEOUT};
use goodfetch_protocol::people::{SIGN_IN_IDLE_EXPIRY, SIGN_IN_RATE};
use goodfetch_protocol::settings::SETTING_INVALID;

use super::{Failure, Machine, multi_thread_runtime, now_of, per_install, unavailable};
use goodfetch_cli::output;

use crate::executable::browsers;

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
        .args(["daemon", "launched", "--copy", build])
        .args(keep_alive.then_some("--keep-alive"))
        .env_clear()
        .envs(environment)
        .current_dir(home.root())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    detach(&mut command);
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
    if service && let Some(exit) = give_place_to_the_pick(machine) {
        return exit;
    }
    let files = machine.files();
    let identity = engine.identity();
    let _copy = copy.and_then(|build| {
        let id = ArtifactId {
            kind: ArtifactKind::ExecutableCopy,
            name: format!("goodfetch-{}", identity.product.as_str()),
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
    let settings =
        browsers::machine_settings(machine.home, identity.product, &identity.product_version).ok();
    let idle_timeout = settings
        .as_ref()
        .and_then(|settings| settings.value(IDLE_TIMEOUT.name).as_u64())
        .map_or(Duration::from_secs(600), Duration::from_millis);
    let console_address = settings.as_ref().map_or_else(
        || default_address(&identity.product_version).to_string(),
        |settings| {
            settings.value(CONSOLE_ADDRESS.name).as_str().map_or_else(
                || default_address(&identity.product_version).to_string(),
                str::to_owned,
            )
        },
    );
    let install = match install_database(machine, &dependencies, settings.as_ref()) {
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
    let runtime = multi_thread_runtime();
    let ended = runtime.block_on(async {
        let (listener, endpoint, token, activated) =
            take_listener(machine, &files, &socket, service).await?;
        let (console, console_state) = if engine.serves_console() {
            console_of_launch(machine, activated, &console_address, settings.as_ref()).await
        } else {
            (DaemonConsole::Absent, ConsoleState::Absent)
        };
        let published = DaemonRuntimeFile {
            endpoint: endpoint.clone(),
            token: DaemonToken::new(token.clone()),
            version: identity.product_version.clone(),
            pid,
            console: console_state,
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
            console,
            drain: None,
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

/// How long the Console's sign-ins last and how often an address may try
/// to sign in: `console.sign_in.idle_expiry` and `console.sign_in.rate`,
/// which only an Operator policy sets, else their defaults.
fn sign_in_policy(settings: Option<&Resolved>) -> SignInPolicy {
    let defaults = SignInPolicy::default();
    let Some(settings) = settings else {
        return defaults;
    };
    SignInPolicy {
        idle_expiry: settings
            .value(SIGN_IN_IDLE_EXPIRY.name)
            .as_u64()
            .map_or(defaults.idle_expiry, Duration::from_millis),
        rate: settings
            .value(SIGN_IN_RATE.name)
            .as_u64()
            .unwrap_or(defaults.rate),
    }
}

/// Where the Console's content origin listens and the origin it is
/// reached at: `console.content_address` and
/// `console.content_public_url`, when set.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct ContentSettings {
    address: Option<String>,
    public_url: Option<String>,
}

/// The content origin's settings, from user configuration and the
/// Operator policy.
fn content_settings(settings: Option<&Resolved>) -> ContentSettings {
    let text = |name| {
        settings
            .and_then(|settings| settings.value(name).as_str().map(str::to_owned))
            .filter(|value| !value.is_empty())
    };
    ContentSettings {
        address: text(CONTENT_ADDRESS.name),
        public_url: text(CONTENT_PUBLIC_URL.name),
    }
}

/// `console.public_url`, from user configuration and the Operator
/// policy: the address links to the Console name, when set.
fn public_url(settings: Option<&Resolved>) -> Option<String> {
    settings
        .and_then(|settings| settings.value(PUBLIC_URL.name).as_str().map(str::to_owned))
        .filter(|value| !value.is_empty())
}

/// Whether `url` is a bare `http` or `https` origin, with at most one
/// closing slash: a host of ASCII letters, digits, hyphens and dots, or a
/// bracketed IPv6 address, and an optional port. The Console's policy
/// frames this text, so nothing else may pass: no wildcard, path, query,
/// user, space or `;`. A host beyond ASCII is given as its Punycode.
fn bare_origin(url: &str) -> bool {
    let Some(rest) = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
    else {
        return false;
    };
    let authority = rest.strip_suffix('/').unwrap_or(rest);
    let (host_is_bare, port) = if let Some(bracketed) = authority.strip_prefix('[') {
        let Some((address, after)) = bracketed.split_once(']') else {
            return false;
        };
        let is_address = !address.is_empty()
            && address
                .chars()
                .all(|c| c.is_ascii_hexdigit() || c == ':' || c == '.');
        let port = if after.is_empty() {
            None
        } else {
            let Some(port) = after.strip_prefix(':') else {
                return false;
            };
            Some(port)
        };
        (is_address, port)
    } else {
        let (host, port) = authority.rsplit_once(':').unzip();
        let host = host.unwrap_or(authority);
        let is_name = host.split('.').all(|label| {
            !label.is_empty() && label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        });
        (is_name, port)
    };
    host_is_bare
        && port.is_none_or(|port| {
            port.chars().all(|c| c.is_ascii_digit()) && port.parse::<u16>().is_ok()
        })
}

/// The Console's content origin (ADR 0031), beside the Console listening
/// at `console`: its own listener at `console.content_address` when set;
/// else, with `console.content_public_url`, the Console's listener, which
/// tells the two apart by `Host` (ADR 0049); else the Console's host on a
/// free port. Its addresses name `console.content_public_url` when set.
/// None, logged, when it cannot listen or its public URL is no origin: the
/// Console then serves without it.
async fn start_content(console: SocketAddr, settings: &ContentSettings) -> Option<ContentOrigin> {
    let public = match settings.public_url.as_deref() {
        Some(url) if bare_origin(url) => Some(url.trim_end_matches('/').to_owned()),
        Some(url) => {
            tracing::error!(
                url,
                "`console.content_public_url` is no bare `http` or `https` origin (no path, query, user or wildcard), so the Console shows no Run's files; set it to an origin such as `https://content.example.com`, then run `daemon restart`"
            );
            return None;
        }
        None => None,
    };
    let address = match (settings.address.as_deref(), &public) {
        (Some(address), _) => {
            let Ok(parsed) = address.parse::<SocketAddr>() else {
                tracing::error!(
                    address,
                    "`console.content_address` is no `host:port`, so the Console shows no Run's files; set it to an address and port, then run `daemon restart`"
                );
                return None;
            };
            parsed
        }
        (None, Some(url)) => {
            return Some(ContentOrigin {
                listener: None,
                url: url.clone(),
            });
        }
        (None, None) => SocketAddr::new(console.ip(), 0),
    };
    let bound_to = listen_console(address).await.and_then(|opened| {
        let bound = opened.local_address()?;
        Ok((opened, bound))
    });
    match bound_to {
        Ok((listener, bound)) => {
            let url = public.unwrap_or_else(|| format!("http://{bound}"));
            tracing::info!(%url, "the Console's content origin serves");
            Some(ContentOrigin {
                listener: Some(listener),
                url,
            })
        }
        Err(error) => {
            tracing::error!(%error, %address, "the Console's content origin cannot listen, so the Console shows no Run's files; set `console.content_address` to a free port, then run `daemon restart`");
            None
        }
    }
}

/// The Daemon's Console (ADR 0031): its listener at `address`, the
/// `console.address` setting, and the URL it is reached at, with its
/// sign-in policy and its content origin from `settings`; or, when the address is no `host:port` or another
/// program holds its port, why it did not start, which fails only the
/// Console, never the Daemon's protocol.
pub(in crate::executable) async fn start_console(
    address: &str,
    settings: Option<&Resolved>,
) -> (DaemonConsole, ConsoleState) {
    let Ok(parsed) = address.parse::<SocketAddr>() else {
        return failed(ConsoleFailure {
            code: SETTING_INVALID.name.to_owned(),
            message: format!("`console.address` is `{address}`, which is no `host:port`, so the Console did not start."),
            fix: "Set `console.address` to an address and port, such as `127.0.0.1:7800`, then run `daemon restart`.".to_owned(),
        });
    };
    match listen_console(parsed).await {
        Ok(listener) => console_on(listener, address, settings).await,
        Err(error) if error.kind() == std::io::ErrorKind::AddrInUse => failed(ConsoleFailure {
            code: CONSOLE_PORT_IN_USE.name.to_owned(),
            message: format!("Another program holds {address}, so the Console did not start; the Daemon still serves its Clients."),
            fix: "Set `console.address` to a free port in user configuration (port 0 takes any free one), or stop the program that holds it, then run `daemon restart`.".to_owned(),
        }),
        Err(error) => failed(ConsoleFailure {
            code: SETTING_INVALID.name.to_owned(),
            message: format!("The Console cannot listen at {address}: {error}."),
            fix: "Set `console.address` to an address this machine has and a port it may take, then run `daemon restart`.".to_owned(),
        }),
    }
}

/// The Console served on `listener`, bound at `address`, with its sign-in
/// policy and its content origin from `settings`, or why it is not.
async fn console_on(
    listener: ConsoleListener,
    address: &str,
    settings: Option<&Resolved>,
) -> (DaemonConsole, ConsoleState) {
    match listener.local_address() {
        Ok(bound) => {
            let url = format!("http://{bound}/");
            tracing::info!(%url, "the Console serves");
            let content = start_content(bound, &content_settings(settings)).await;
            (
                DaemonConsole::Listening {
                    listener,
                    url: url.clone(),
                    public_url: public_url(settings),
                    sign_in: sign_in_policy(settings),
                    content,
                },
                ConsoleState::Serving(url),
            )
        }
        Err(error) => failed(ConsoleFailure {
            code: SETTING_INVALID.name.to_owned(),
            message: format!("The Console's listener at {address} has no address: {error}."),
            fix: "Set `console.address` to another address, then run `daemon restart`.".to_owned(),
        }),
    }
}

/// The Console that did not start, as `failure` says, logged.
fn failed(failure: ConsoleFailure) -> (DaemonConsole, ConsoleState) {
    tracing::error!(
        code = failure.code.as_str(),
        "{} {}",
        failure.message,
        failure.fix
    );
    (
        DaemonConsole::Failed(failure.clone()),
        ConsoleState::Failed(failure),
    )
}

/// The Daemon's Console, on the listener the service manager handed over
/// when it started this Daemon at a first visit to the Console (ADR
/// 0031), else at `address`.
async fn console_of_launch(
    machine: &Machine<'_>,
    activated: bool,
    address: &str,
    settings: Option<&Resolved>,
) -> (DaemonConsole, ConsoleState) {
    let handed = if activated {
        activated_console(machine.environment).unwrap_or_else(|error| {
            tracing::warn!(%error, "the service manager's Console socket cannot be taken, so the Console listens itself");
            None
        })
    } else {
        None
    };
    match handed {
        Some(listener) => console_on(listener, address, settings).await,
        None => start_console(address, settings).await,
    }
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
        DaemonEnd::ListenerFailed => ExitCode::FAILURE,
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
    let error = std::process::Command::new(&executable)
        .args(["daemon", "launched", "--service", "--copy", &build])
        .env_clear()
        .envs(machine.environment)
        .current_dir(machine.home.root())
        .exec();
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

#[cfg(test)]
mod tests {
    use goodfetch_engine::transport::listen_console;
    use goodfetch_protocol::console::ConsoleState;

    use super::start_console;

    /// Covers: error:console_port_in_use
    #[test]
    #[expect(
        clippy::doc_markdown,
        reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
    )]
    fn a_console_address_whose_port_another_listener_holds_fails_with_console_port_in_use() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();

        let state = runtime.block_on(async {
            let held = listen_console("127.0.0.1:0".parse().unwrap())
                .await
                .unwrap();
            let address = held.local_address().unwrap().to_string();
            start_console(&address, None).await.1
        });

        let code = match state {
            ConsoleState::Failed(failure) => failure.code,
            other => panic!("the Console started: {other:?}"),
        };
        assert_eq!(code, "console_port_in_use");
    }
}
