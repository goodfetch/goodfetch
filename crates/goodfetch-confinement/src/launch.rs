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
use std::path::PathBuf;

use goodfetch_protocol::plugin::{COOKIE, COOKIE_VARIABLE};
use serde::{Deserialize, Serialize};

use crate::policy::Policy;

/// The launcher's one variable, which holds its [`Launch`] as JSON.
pub const LAUNCH_VARIABLE: &str = "GOODFETCH_PLUGIN_LAUNCH";

/// The Product executable's argument that runs it as the launcher.
pub const SHIM_COMMAND: &str = "plugin-shim";

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
}

/// The launcher's exit statuses that tell the host cannot confine a Plugin,
/// which the engine refuses with `confinement_unavailable`.
pub const UNAVAILABLE_STATUSES: [u8; 2] = [247, 248];

impl LaunchError {
    /// The launcher's exit status for it, from 246 to 253: past every
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
        }
    }
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
    use std::os::unix::process::CommandExt as _;

    use crate::linux::{ConfineError, confine_self};

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
    match confine_self(&launch.policy) {
        Ok(_) => {}
        Err(ConfineError::Unavailable(reason)) => {
            return LaunchError::Unavailable(reason.to_owned());
        }
        Err(error) => return LaunchError::Confine(error.to_string()),
    }
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
    // Set once confined, so that a namespace mechanism's former self, which
    // waits outside, holds none of them.
    if let Err(error) = crate::linux::limits::limit_self(&launch.policy.limits) {
        return LaunchError::Limits(error);
    }
    #[expect(
        clippy::disallowed_methods,
        reason = "the launcher's one thread replaces itself with the Plugin: no runtime runs here (ADR 0033)"
    )]
    let error = std::process::Command::new(&launch.program)
        .args(&launch.args)
        .env_clear()
        .envs(launch.env())
        .exec();
    LaunchError::Start(error)
}

#[cfg(not(target_os = "linux"))]
fn start(_launch: &Launch) -> LaunchError {
    LaunchError::Unsupported
}

#[cfg(test)]
mod tests;
