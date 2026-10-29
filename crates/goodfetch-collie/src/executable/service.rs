//! The major's per-user service and the Daemon's tokens, from both
//! Products' executables (ADRs 0024, 0031 and 0098): `daemon install`,
//! `daemon uninstall`, `daemon rotate-token` and `daemon
//! http-listener-token`. The definitions and commands are the engine's pure
//! [`service::plan`]; this module writes the files, runs the service
//! manager's commands, and keeps the records: the service's, in the major's
//! folder, the persistent token of a socket-activated Daemon, and the
//! versioned copy the service starts, which the list of installed
//! executables records at its own path so the Browser cache's clean-up
//! keeps it until `daemon uninstall` forgets it.
//!
//! A Daemon running when the service is installed or removed stops first,
//! since the service's socket takes its place. `daemon rotate-token`
//! replaces the persistent token, which a socket-activated Daemon and the
//! foreground Daemon take, and the HTTP listener's token (spec #608); an
//! installed service's Daemon and a running foreground Daemon stop with it,
//! so the old persistent token stops working at once, and any other Daemon
//! refuses the old tokens from its next start. `daemon http-listener-token` prints
//! the HTTP listener's token on stdout, the one command that prints a
//! credential, and logs nothing of it.
#![expect(
    clippy::disallowed_methods,
    reason = "the CLI writes the service's files and runs its service manager before and outside any async worker (ADR 0060)"
)]

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::{ExitCode, Stdio};

use goodfetch_engine::browser_cache;
use goodfetch_engine::daemon::service::{
    self, ManagerCommand, ServiceRequest, activates_sockets, manager_of_this_os,
};
use goodfetch_engine::installed::fingerprint;
use goodfetch_engine::transport;
use goodfetch_protocol::console::{ADDRESS as CONSOLE_ADDRESS, default_address};
use goodfetch_protocol::daemon::{
    DaemonStopping, DaemonToken, DaemonUninstallOutput, HttpListenerToken, InstalledService,
    ServiceManager, TokenRotated,
};
use goodfetch_protocol::executables::{ExecutableKind, InstalledExecutable};

use super::browsers;
use super::daemon::{Failure, Machine};
use goodfetch_cli::output::{self, Human};

fn unavailable(what: &str) -> impl Fn(std::io::Error) -> Failure + '_ {
    move |error| Failure::Unavailable(format!("{what}: {error}"))
}

/// `daemon install`.
pub(super) fn install(machine: &Machine<'_>, json: bool, human: &Human) -> ExitCode {
    match installed(machine) {
        Ok((service, stopped)) => {
            if json {
                output::json(&machine.status_of(None));
            } else {
                if let Some(stopped) = &stopped {
                    output::line(&format!(
                        "The Daemon of major {} stopped for its service, and its {} live Runs with it",
                        machine.major(),
                        stopped.live_runs
                    ));
                }
                output::line(&describe(&service));
            }
            ExitCode::SUCCESS
        }
        Err(failure) => failure.print(json, human),
    }
}

/// `daemon uninstall`.
pub(super) fn uninstall(machine: &Machine<'_>, json: bool, human: &Human) -> ExitCode {
    match uninstalled(machine) {
        Ok(uninstalled) => {
            if json {
                output::json(&uninstalled);
            } else {
                match &uninstalled.removed {
                    Some(service) => output::line(&format!(
                        "Removed the service {} of major {}",
                        service.name,
                        machine.major()
                    )),
                    None => output::line(&format!(
                        "No service of major {} is installed",
                        machine.major()
                    )),
                }
            }
            ExitCode::SUCCESS
        }
        Err(failure) => failure.print(json, human),
    }
}

/// `daemon rotate-token`.
pub(super) fn rotate_token(machine: &Machine<'_>, json: bool, human: &Human) -> ExitCode {
    match rotated(machine) {
        Ok(rotated) => {
            if json {
                output::json(&rotated);
            } else {
                output::line(&format!(
                    "New tokens are in {} and {}; the next Daemon takes them and refuses the old ones",
                    machine.files().token_file().display(),
                    machine.files().http_listener_token_file().display()
                ));
            }
            ExitCode::SUCCESS
        }
        Err(failure) => failure.print(json, human),
    }
}

/// `daemon http-listener-token`: the token alone on stdout, or as
/// `HttpListenerToken` under `--json`; nothing of it anywhere else.
pub(super) fn http_listener_token(machine: &Machine<'_>, json: bool, human: &Human) -> ExitCode {
    let token = match machine.files().http_listener_token() {
        Ok(token) => token,
        Err(error) => {
            return Failure::Unavailable(format!(
                "the HTTP listener's token cannot be read or made: {error}"
            ))
            .print(json, human);
        }
    };
    if json {
        output::json(&HttpListenerToken {
            token: DaemonToken::new(token),
        });
    } else {
        output::line(&token);
    }
    ExitCode::SUCCESS
}

/// What `daemon status` says of `service`.
pub(super) fn describe(service: &InstalledService) -> String {
    match (&service.socket, service.manager) {
        (Some(socket), ServiceManager::Launchd) => format!(
            "Installed as the launch agent {}: the first connection to {} starts the Daemon",
            service.name,
            socket.display()
        ),
        (Some(socket), _) => format!(
            "Installed as the systemd user units {}: the first connection to {} starts the Daemon",
            service.name,
            socket.display()
        ),
        (None, _) => format!(
            "Installed as the logon task {}, which starts the Daemon at each logon",
            service.name
        ),
    }
}

fn installed(machine: &Machine<'_>) -> Result<(InstalledService, Option<DaemonStopping>), Failure> {
    let manager = manager_of_this_os().ok_or_else(|| {
        Failure::Unavailable("this OS has no service manager Goodfetch registers with".to_owned())
    })?;
    let executable = machine.executable.clone().ok_or_else(|| {
        Failure::Unavailable("this executable's own path cannot be read".to_owned())
    })?;
    let files = machine.files();
    let earlier = files
        .service()
        .map_err(unavailable("the installed service's record cannot be read"))?;
    let stopped = machine.stop()?;
    if let Some(earlier) = &earlier {
        remove(machine, earlier, true)?;
    }
    let identity = machine.identity;
    let copy = browser_cache::place_executable_copy(
        &machine.cache(),
        &executable,
        identity.product,
        &identity.product_version,
        now(),
    )
    .map_err(unavailable("the Daemon's versioned copy cannot be made"))?;
    record_copy(machine, &copy.executable, &copy.id)?;
    let socket = if activates_sockets(manager) {
        let socket = files
            .service_socket()
            .map_err(unavailable("the service's socket has no folder"))?;
        remove_file(&socket)?;
        files.persistent_token().map_err(unavailable(
            "the Daemon's persistent token cannot be written",
        ))?;
        Some(socket)
    } else {
        None
    };
    let console = match &socket {
        Some(_) => console_address(machine),
        None => None,
    };
    let domain = launchd_domain(machine, manager)?;
    let plan = service::plan(&ServiceRequest {
        manager,
        home: machine.home,
        version: &identity.product_version,
        executable: &copy.executable,
        build: &copy.id.build,
        socket: socket.as_deref(),
        console,
        environment: machine.environment,
        launchd_domain: domain.as_deref(),
    })
    .map_err(|error| Failure::Unavailable(error.to_string()))?;
    for file in &plan.files {
        if let Some(folder) = file.path.parent() {
            std::fs::create_dir_all(folder)
                .map_err(unavailable("the service's folder cannot be made"))?;
        }
        std::fs::write(&file.path, &file.contents)
            .map_err(unavailable("the service's definition cannot be written"))?;
    }
    files
        .record_service(&plan.service)
        .map_err(unavailable("the installed service cannot be recorded"))?;
    if let Err(failure) = run(machine, &plan.register) {
        if let Err(cleanup) = remove(machine, &plan.service, false) {
            let _ = cleanup.report();
        }
        return Err(failure);
    }
    Ok((plan.service, stopped))
}

/// The Console's address the service manager listens on for the Daemon
/// it starts (ADR 0031): `console.address` from the machine's layers, else
/// its default, port 0 replaced by a port free now, since the service
/// manager listens there before any Daemon reports where it is; none, and
/// the Console's address left to the Daemon, when it is no `host:port` or
/// no port is free.
fn console_address(machine: &Machine<'_>) -> Option<SocketAddr> {
    let identity = machine.identity;
    let configured =
        browsers::machine_settings(machine.home, identity.product, &identity.product_version)
            .ok()
            .and_then(|settings| {
                settings
                    .value(CONSOLE_ADDRESS.name)
                    .as_str()
                    .map(str::to_owned)
            })
            .unwrap_or_else(|| default_address(&identity.product_version).to_string());
    let Ok(address) = configured.parse::<SocketAddr>() else {
        tracing::warn!(address = %configured, "`console.address` is no `host:port`, so the first visit to the Console does not start the Daemon");
        return None;
    };
    match transport::free_port(address) {
        Ok(address) => Some(address),
        Err(error) => {
            tracing::warn!(%error, %address, "no port is free for the Console, so the first visit to the Console does not start the Daemon");
            None
        }
    }
}

fn uninstalled(machine: &Machine<'_>) -> Result<DaemonUninstallOutput, Failure> {
    let Some(service) = machine
        .files()
        .service()
        .map_err(unavailable("the installed service's record cannot be read"))?
    else {
        return Ok(DaemonUninstallOutput { removed: None });
    };
    machine.stop()?;
    remove(machine, &service, false)?;
    Ok(DaemonUninstallOutput {
        removed: Some(service),
    })
}

fn rotated(machine: &Machine<'_>) -> Result<TokenRotated, Failure> {
    let files = machine.files();
    let activated = files
        .service()
        .map_err(unavailable("the installed service's record cannot be read"))?
        .is_some_and(|service| service.socket.is_some());
    // The foreground Daemon at `daemon.address` takes the persistent token
    // too: reached with the old token before it is replaced, it stops once
    // it is, so the old token stops working at once (ADR 0031's #221 note).
    // One that refuses this Client takes no token of this major.
    let foreground = match machine.foreground() {
        Ok(foreground) => foreground,
        Err(Failure::Refused { .. } | Failure::DaemonRefused(_)) => None,
        Err(failure) => return Err(failure),
    };
    files
        .rotate_token()
        .map_err(unavailable("the persistent token cannot be replaced"))?;
    files
        .rotate_http_listener_token()
        .map_err(unavailable("the HTTP listener's token cannot be replaced"))?;
    let rotated_at = now();
    if let Some(mut foreground) = foreground {
        let _: DaemonStopping = foreground.call("daemon.stop", &serde_json::json!({}))?;
        foreground.close();
    }
    if activated {
        // Connections that opened with the old token close with their
        // Daemon; the next connection starts one that takes the new token.
        machine.stop()?;
    }
    Ok(TokenRotated { rotated_at })
}

/// Removes `service`: stops and unregisters it, removes its files and
/// socket, forgets the copy it started, and, unless `keep_token`, its
/// persistent token.
fn remove(
    machine: &Machine<'_>,
    service: &InstalledService,
    keep_token: bool,
) -> Result<(), Failure> {
    let domain = launchd_domain(machine, service.manager)?;
    let unregistering = service::unregister(service, domain.as_deref());
    run(machine, &unregistering.before_removal)?;
    for file in &service.files {
        remove_file(file)?;
    }
    run(machine, &unregistering.after_removal)?;
    if let Some(socket) = &service.socket {
        remove_file(socket)?;
    }
    let list = machine.list();
    let entries = list.read().map_err(unavailable(
        "the list of installed executables cannot be read",
    ))?;
    for entry in entries
        .iter()
        .filter(|entry| Path::new(&entry.path) == service.executable)
    {
        list.forget(entry).map_err(unavailable(
            "the list of installed executables cannot be written",
        ))?;
    }
    let files = machine.files();
    if !keep_token {
        files
            .remove_token()
            .map_err(unavailable("the persistent token cannot be removed"))?;
    }
    files.forget_service().map_err(unavailable(
        "the installed service's record cannot be removed",
    ))
}

/// Records the versioned copy the service starts as an executable at its
/// own path, which pins itself and this version's downloads, so the
/// clean-up keeps it while it is there.
fn record_copy(
    machine: &Machine<'_>,
    path: &Path,
    copy: &goodfetch_protocol::browser_cache::ArtifactId,
) -> Result<(), Failure> {
    let (size, modified_at) =
        fingerprint(path).map_err(unavailable("the Daemon's versioned copy cannot be read"))?;
    machine
        .list()
        .record(&InstalledExecutable {
            kind: ExecutableKind::ProductExecutable,
            product: machine.identity.product,
            product_version: machine.identity.product_version.clone(),
            path: path.display().to_string(),
            size,
            modified_at,
            pins: crate::pinned_artifacts()
                .into_iter()
                .map(|artifact| artifact.id)
                .collect(),
            copy: Some(copy.clone()),
            last_seen: now(),
        })
        .map_err(unavailable(
            "the list of installed executables cannot be written",
        ))
}

/// The launchd domain of the user's agents: `gui/<uid>` in a login
/// session, else `user/<uid>`; `None` under another service manager.
#[cfg(unix)]
fn launchd_domain(
    machine: &Machine<'_>,
    manager: ServiceManager,
) -> Result<Option<String>, Failure> {
    use std::os::unix::fs::MetadataExt as _;
    if manager != ServiceManager::Launchd {
        return Ok(None);
    }
    std::fs::create_dir_all(machine.home.root())
        .map_err(unavailable("Goodfetch home cannot be made"))?;
    // Goodfetch home is the user's own, so its owner is this process's user.
    let uid = std::fs::metadata(machine.home.root())
        .map_err(unavailable("Goodfetch home cannot be read"))?
        .uid();
    let gui = format!("gui/{uid}");
    let answered = std::process::Command::new(program(machine, "launchctl"))
        .args(["print", gui.as_str()])
        .env_clear()
        .envs(machine.environment)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success());
    Ok(Some(if answered { gui } else { format!("user/{uid}") }))
}

#[cfg(not(unix))]
#[expect(
    clippy::unnecessary_wraps,
    reason = "reading the domain can fail on Unix, never here: one signature serves both"
)]
fn launchd_domain(
    _machine: &Machine<'_>,
    _manager: ServiceManager,
) -> Result<Option<String>, Failure> {
    Ok(None)
}

/// Runs each command in turn; one whose failure is expected never fails.
fn run(machine: &Machine<'_>, commands: &[ManagerCommand]) -> Result<(), Failure> {
    for command in commands {
        let program = program(machine, command.program);
        let output = std::process::Command::new(&program)
            .args(&command.arguments)
            .env_clear()
            .envs(machine.environment)
            .stdin(Stdio::null())
            .output();
        let shown = || {
            let arguments: Vec<String> = command
                .arguments
                .iter()
                .map(|argument| argument.to_string_lossy().into_owned())
                .collect();
            format!("{} {}", program.display(), arguments.join(" "))
        };
        match output {
            Ok(output) if output.status.success() => {
                tracing::debug!(command = shown(), "the service manager answered");
            }
            Ok(output) if command.may_fail => tracing::debug!(
                command = shown(),
                stderr = %String::from_utf8_lossy(&output.stderr).trim(),
                "the service manager had nothing to remove"
            ),
            Err(error) if command.may_fail => {
                tracing::debug!(command = shown(), %error, "the service manager did not run");
            }
            Ok(output) => {
                return Err(Failure::Unavailable(format!(
                    "`{}` failed: {}",
                    shown(),
                    String::from_utf8_lossy(&output.stderr).trim()
                )));
            }
            Err(error) => {
                return Err(Failure::Unavailable(format!(
                    "`{}` did not run: {error}",
                    shown()
                )));
            }
        }
    }
    Ok(())
}

/// The service manager's program: found on the environment's `PATH`, or
/// at its usual place when the environment has none.
fn program(machine: &Machine<'_>, name: &str) -> PathBuf {
    let has_path = machine
        .environment
        .keys()
        .any(|key| key.eq_ignore_ascii_case("PATH"));
    if has_path {
        return PathBuf::from(name);
    }
    match name {
        "launchctl" => PathBuf::from("/bin/launchctl"),
        "systemctl" => PathBuf::from("/usr/bin/systemctl"),
        "schtasks" => machine
            .environment
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case("SystemRoot"))
            .map_or_else(
                || PathBuf::from(r"C:\Windows"),
                |(_, root)| PathBuf::from(root),
            )
            .join("System32")
            .join("schtasks.exe"),
        other => PathBuf::from(other),
    }
}

fn remove_file(path: &Path) -> Result<(), Failure> {
    match std::fs::remove_file(path) {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(Failure::Unavailable(
            format!("{} cannot be removed: {error}", path.display()),
        )),
        _ => Ok(()),
    }
}

fn now() -> jiff::Timestamp {
    let now = std::time::SystemTime::now();
    jiff::Timestamp::try_from(now).expect("invariant: the system clock reads a representable time")
}
