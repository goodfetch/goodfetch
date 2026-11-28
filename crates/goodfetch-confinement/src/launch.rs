//! Starting a process Plugin confined (ADR 0033): the engine, which runs
//! many threads, never confines itself, so it starts a launcher, the
//! Product executable's hidden `plugin-shim` mode, with a [`Launch`] in its
//! environment and nothing else. The launcher's one thread confines itself
//! to the launch's policy, then replaces itself with the Plugin, which
//! keeps the confinement: an empty environment holding only the policy's
//! variables and [`COOKIE_VARIABLE`], its private temporary folder as its
//! working folder, no handle beyond stdio, and its manifest's resource
//! limits (`linux::limits`).
//!
//! The launcher writes nothing: its stdio are the Plugin's. Before the
//! Plugin starts, it answers through its exit status alone
//! ([`LaunchError::status`]).

use std::ffi::OsStr;
use std::io;
use std::path::{Path, PathBuf};

use goodfetch_protocol::home::PLUGINS;
use goodfetch_protocol::plugin::{COOKIE, COOKIE_VARIABLE};
use serde::{Deserialize, Serialize};

use crate::policy::Policy;

/// The launcher's one variable, which holds its [`Launch`] as JSON.
pub const LAUNCH_VARIABLE: &str = "GOODFETCH_PLUGIN_LAUNCH";

/// The Product executable's argument that runs it as the launcher.
pub const SHIM_COMMAND: &str = "plugin-shim";

/// Parameters for the `plugin-shim` loopback relay at `proxy_tcp` (ADR 0033,
/// issue 833): carries the inherited control FD and per-instance proxy
/// credentials injected into the Plugin's proxy environment variables.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProxyRelay {
    /// Inherited `AF_UNIX` control socket file descriptor (on Linux, FD 3).
    pub control_fd: i32,
    /// Per-instance proxy username.
    pub username: String,
    /// Per-instance proxy password.
    pub password: String,
}

/// What the launcher starts, and under which policy.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Launch {
    /// The policy it confines itself to before it starts the Plugin.
    pub policy: Policy,
    /// The Plugin's program: its entrypoint, or its runtime's interpreter.
    pub program: PathBuf,
    /// The program's arguments, its own name excluded.
    pub args: Vec<String>,
    /// The instance's private temporary folder, the Plugin's working folder.
    pub folder: PathBuf,
    /// Present for `proxy_tcp` when `plugin-shim` relays loopback TCP inside
    /// an isolated network namespace to the engine's per-instance proxy.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proxy_relay: Option<ProxyRelay>,
    /// When `true`, apply every confinement layer the host offers instead of
    /// failing with `Unavailable` when a layer is missing: set only for a
    /// first-party Plugin whose Operator policy sets
    /// `plugins.<name>.reduced_confinement = true` (ADR 0033).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub reduced_confinement: bool,
}

impl Launch {
    /// The Plugin's whole environment: the policy's variables and the
    /// cookie.
    pub fn env(&self) -> impl Iterator<Item = (&str, &str)> {
        self.policy
            .env
            .iter()
            .map(|(name, value)| (name.as_str(), value.as_str()))
            .chain([(COOKIE_VARIABLE, COOKIE)])
    }
}

/// Why the launcher started no Plugin.
#[derive(Debug, thiserror::Error)]
pub enum LaunchError {
    /// Its variable is missing or holds no [`Launch`].
    #[error("the launcher's request cannot be read")]
    Request,
    /// This OS has no applier yet (#867, #873).
    #[error("this OS cannot confine a Plugin yet")]
    Unsupported,
    /// The host lacks what confinement needs.
    #[error("this host cannot confine a Plugin: {0}")]
    Unavailable(String),
    /// Confining failed for another reason.
    #[error("confining the Plugin failed: {0}")]
    Confine(String),
    /// The working folder cannot be entered.
    #[error("cannot enter the instance's folder: {0}")]
    Folder(io::Error),
    /// The handles beyond stdio cannot be closed.
    #[error("cannot close the handles beyond stdio: {0}")]
    Handles(io::Error),
    /// The OS refused to start the program.
    #[error("cannot start the Plugin: {0}")]
    Start(io::Error),
    /// The OS refused one of the Plugin's resource limits.
    #[error("cannot set the Plugin's resource limits: {0}")]
    Limits(io::Error),
    /// The helper refused to start a program outside the Plugin store and
    /// runtime cache (ADR 0033, issue 884).
    #[error("`{}` is outside the Plugin store and runtime cache", .0.display())]
    DisallowedProgram(PathBuf),
}

/// The launcher's exit statuses that tell the host cannot confine a Plugin,
/// which the engine refuses with `confinement_unavailable`.
pub const UNAVAILABLE_STATUSES: [u8; 2] = [247, 248];

impl LaunchError {
    /// The launcher's exit status for it, from 246 to 254: past every
    /// status the confinement probe reports (`linux::probe`), so a runner
    /// that starts the probe through its launcher tells one from the other.
    pub fn status(&self) -> u8 {
        match self {
            Self::Request => 246,
            Self::Unsupported => 247,
            Self::Unavailable(_) => 248,
            Self::Confine(_) => 249,
            Self::Folder(_) => 250,
            Self::Handles(_) => 251,
            Self::Start(_) => 252,
            Self::Limits(_) => 253,
            Self::DisallowedProgram(_) => 254,
        }
    }
}

/// The folders `goodfetch-sandbox` allows a Plugin's program to live in under
/// Goodfetch `home` (and optional `browsers_path` override): the Plugin store
/// (`<home>/plugins`), the shared Browser/runtime cache (`<home>/browsers` or
/// `browsers_path`), and `<home>/runtimes`.
#[must_use]
pub fn allowed_roots(home: &Path, browsers_path: Option<&Path>) -> Vec<PathBuf> {
    let mut roots = vec![
        home.join(PLUGINS),
        home.join("browsers"),
        home.join("runtimes"),
    ];
    if let Some(browsers) = browsers_path {
        roots.push(browsers.to_path_buf());
    }
    roots
}

/// Validates that `program` canonicalizes to an existing regular file strictly
/// inside one of `roots` (Plugin store or runtime cache), refusing traversal
/// or symlinks that escape those roots (ADR 0033, issue 884).
pub fn validate_program_in_roots(
    program: &Path,
    roots: &[PathBuf],
) -> Result<PathBuf, LaunchError> {
    let Ok(canonical_program) = std::fs::canonicalize(program) else {
        return Err(LaunchError::DisallowedProgram(program.to_path_buf()));
    };
    if !canonical_program.is_file() {
        return Err(LaunchError::DisallowedProgram(program.to_path_buf()));
    }
    for root in roots {
        let Ok(canonical_root) = std::fs::canonicalize(root) else {
            continue;
        };
        if canonical_program.starts_with(&canonical_root) && canonical_program != canonical_root {
            return Ok(canonical_program);
        }
    }
    Err(LaunchError::DisallowedProgram(program.to_path_buf()))
}

/// Runs the `goodfetch-sandbox` helper (ADR 0033, issue 884): parses its
/// [`Launch`], verifies `launch.program` lies inside `allowed_roots` (the
/// Plugin store or runtime cache), creates namespaces, drops every capability,
/// sets `no_new_privs`, installs the helper's baseline seccomp filter, then
/// installs Landlock and the caller's policy filter, and replaces itself with
/// the Plugin. Returns only on error.
pub fn sandbox_shim(launch: Option<&OsStr>, allowed_roots: &[PathBuf]) -> LaunchError {
    let Some(launch) = launch
        .and_then(OsStr::to_str)
        .and_then(|text| serde_json::from_str::<Launch>(text).ok())
    else {
        return LaunchError::Request;
    };
    let program = match validate_program_in_roots(&launch.program, allowed_roots) {
        Ok(program) => program,
        Err(error) => return error,
    };
    let mut verified = launch;
    verified.program = program;
    start_sandbox(&verified)
}

/// Runs the launcher: reads its [`Launch`] from `launch`, the value of its
/// environment's [`LAUNCH_VARIABLE`], confines the process and starts the
/// Plugin in its place. Call it first in the Product executable's `main`,
/// before anything starts a thread. It returns only when no Plugin
/// started.
pub fn shim(launch: Option<&OsStr>) -> LaunchError {
    let Some(launch) = launch
        .and_then(OsStr::to_str)
        .and_then(|text| serde_json::from_str::<Launch>(text).ok())
    else {
        return LaunchError::Request;
    };
    start(&launch)
}

#[cfg(target_os = "linux")]
fn start(launch: &Launch) -> LaunchError {
    start_linux(launch, false)
}

#[cfg(target_os = "linux")]
fn start_sandbox(launch: &Launch) -> LaunchError {
    start_linux(launch, true)
}

#[cfg(target_os = "linux")]
fn start_linux(launch: &Launch, sandbox_helper: bool) -> LaunchError {
    use std::os::unix::process::CommandExt as _;

    use crate::linux::{
        ConfineError, MechanismKind, confine_for_sandbox, confine_self_reduced,
        confine_self_with_relay,
    };

    if let Err(error) = crate::linux::close_beyond_stdio() {
        return LaunchError::Handles(error);
    }
    // Entered before the confinement, whose seccomp lists allow no `chdir`;
    // a mount namespace's new root keeps it, as the policy exposes it.
    let folder = match std::fs::canonicalize(&launch.folder)
        .and_then(|folder| std::env::set_current_dir(&folder).map(|()| folder))
    {
        Ok(folder) => folder,
        Err(error) => return LaunchError::Folder(error),
    };
    let control_fd = launch.proxy_relay.as_ref().map(|relay| relay.control_fd);
    let relay_port = if sandbox_helper {
        // The helper never runs reduced confinement: its security boundary is
        // that it always confines with capabilities dropped, `no_new_privs`,
        // its baseline seccomp filter, and the caller's policy filter.
        match confine_for_sandbox(&launch.policy, MechanismKind::of_kernel(), control_fd) {
            Ok((_, port)) => port,
            Err(ConfineError::Unavailable(reason)) => {
                return LaunchError::Unavailable(reason.to_owned());
            }
            Err(error) => return LaunchError::Confine(error.to_string()),
        }
    } else if launch.reduced_confinement {
        match confine_self_reduced(&launch.policy, control_fd) {
            Ok(port) => port,
            Err(error) => return LaunchError::Confine(error.to_string()),
        }
    } else {
        match confine_self_with_relay(&launch.policy, MechanismKind::of_kernel(), control_fd) {
            Ok((_, port)) => port,
            Err(ConfineError::Unavailable(reason)) => {
                return LaunchError::Unavailable(reason.to_owned());
            }
            Err(error) => return LaunchError::Confine(error.to_string()),
        }
    };
    match std::env::current_dir() {
        Ok(working) if working == folder => {}
        Ok(working) => {
            return LaunchError::Folder(std::io::Error::other(format!(
                "the confined launcher works from `{}`",
                working.display()
            )));
        }
        Err(error) => return LaunchError::Folder(error),
    }
    if let Err(error) = crate::linux::limits::limit_self(&launch.policy.limits) {
        return LaunchError::Limits(error);
    }
    let mut env: Vec<(String, String)> = launch
        .env()
        .map(|(k, v)| (k.to_owned(), v.to_owned()))
        .collect();
    if let (Some(relay), Some(port)) = (&launch.proxy_relay, relay_port) {
        let http_url = format!(
            "http://{}:{}@127.0.0.1:{port}",
            relay.username, relay.password
        );
        let socks_url = format!(
            "socks5h://{}:{}@127.0.0.1:{port}",
            relay.username, relay.password
        );
        for key in ["HTTP_PROXY", "HTTPS_PROXY", "http_proxy", "https_proxy"] {
            env.push((key.to_owned(), http_url.clone()));
        }
        for key in ["ALL_PROXY", "all_proxy"] {
            env.push((key.to_owned(), socks_url.clone()));
        }
    }
    #[expect(
        clippy::disallowed_methods,
        reason = "the launcher's one thread replaces itself with the Plugin: no runtime runs here (ADR 0033)"
    )]
    let error = std::process::Command::new(&launch.program)
        .args(&launch.args)
        .env_clear()
        .envs(env)
        .exec();
    LaunchError::Start(error)
}

#[cfg(not(target_os = "linux"))]
fn start(_launch: &Launch) -> LaunchError {
    LaunchError::Unsupported
}

#[cfg(not(target_os = "linux"))]
fn start_sandbox(_launch: &Launch) -> LaunchError {
    LaunchError::Unsupported
}

#[cfg(test)]
mod tests;
