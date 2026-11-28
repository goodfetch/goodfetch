//! A process Plugin's instance as the OS runs it (ADR 0033). The engine,
//! which runs many threads, never confines itself: it starts the launcher,
//! the Product executable's hidden `plugin-shim` mode, through
//! [`Children`], in a process group of its own, with the [`Launch`] as its
//! one variable and nothing inherited. The launcher confines its one
//! thread and becomes the Plugin, whose stdin and stdout are the plugin
//! protocol's pipes. Stopping the instance kills its group, waits for it
//! and deletes its private temporary folder.

use std::io;
use std::path::{Path, PathBuf};
use std::process::{ExitStatus, Stdio};

use goodfetch_confinement::launch::{LAUNCH_VARIABLE, Launch, SHIM_COMMAND, UNAVAILABLE_STATUSES};
use goodfetch_protocol::plugin::PluginResourceLimit;
use tokio::process::{Child, ChildStdin, ChildStdout, Command};

use crate::children::Children;
use crate::tasks::TaskOwner;

/// A started instance's process, killed when dropped.
#[derive(Debug)]
pub struct PluginProcess {
    child: Child,
    group: Children,
    folder: PathBuf,
}

/// The plugin protocol's pipes to a started instance: what the engine
/// writes to it and reads from it.
#[derive(Debug)]
pub struct Pipes {
    pub stdin: ChildStdin,
    pub stdout: ChildStdout,
}

impl PluginProcess {
    /// Starts the Plugin `launch` describes through `launcher`, the
    /// Product executable. The instance's private temporary folder,
    /// `launch.folder`, must exist, since the policy grants it: [`stop`]
    /// deletes it.
    ///
    /// Call it from an async task's thread, which lives as long as the
    /// engine: on Linux, the kernel kills the instance when the thread that
    /// started it ends ([`Children::spawn`]).
    ///
    /// # Errors
    ///
    /// When the OS refuses to start the launcher.
    ///
    /// [`stop`]: Self::stop
    pub fn start(launcher: &Path, launch: &Launch) -> io::Result<(Self, Pipes)> {
        Self::start_with_env(launcher, launch, &[])
    }

    /// Starts the Plugin `launch` describes through `launcher`, passing
    /// `launcher_env` alongside [`LAUNCH_VARIABLE`] (for example
    /// `GOODFETCH_HOME` for `goodfetch-sandbox`'s allowed-root check; the
    /// launcher clears its environment before `exec`ing the Plugin).
    ///
    /// # Errors
    ///
    /// When the OS refuses to start the launcher.
    pub fn start_with_env(
        launcher: &Path,
        launch: &Launch,
        launcher_env: &[(String, String)],
    ) -> io::Result<(Self, Pipes)> {
        let command = shim_command(launcher, launch, launcher_env)?;
        let mut group = Children::new()?;
        let mut child = group.spawn(command)?;
        let (Some(stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
            return Err(io::Error::other("the instance's pipes were not opened"));
        };
        let process = Self {
            child,
            group,
            folder: launch.folder.clone(),
        };
        Ok((process, Pipes { stdin, stdout }))
    }

    /// Starts the Plugin on Linux as [`Self::start`] does, passing
    /// `control_fd` inherited at `target_fd` for `plugin-shim`'s `proxy_tcp`
    /// loopback relay (ADR 0033, issue 833).
    ///
    /// # Errors
    ///
    /// When the OS refuses to start the launcher.
    #[cfg(target_os = "linux")]
    pub fn start_with_fd(
        launcher: &Path,
        launch: &Launch,
        control_fd: std::os::unix::io::OwnedFd,
        target_fd: libc::c_int,
    ) -> io::Result<(Self, Pipes)> {
        Self::start_with_fd_and_env(launcher, launch, &[], control_fd, target_fd)
    }

    /// Starts the Plugin on Linux as [`Self::start_with_env`] does, passing
    /// `control_fd` inherited at `target_fd` for the `proxy_tcp` loopback
    /// relay.
    ///
    /// # Errors
    ///
    /// When the OS refuses to start the launcher.
    #[cfg(target_os = "linux")]
    pub fn start_with_fd_and_env(
        launcher: &Path,
        launch: &Launch,
        launcher_env: &[(String, String)],
        control_fd: std::os::unix::io::OwnedFd,
        target_fd: libc::c_int,
    ) -> io::Result<(Self, Pipes)> {
        let command = shim_command(launcher, launch, launcher_env)?;
        let mut group = Children::new()?;
        let mut child = group.spawn_with_fd(command, control_fd, target_fd)?;
        let (Some(stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
            return Err(io::Error::other("the instance's pipes were not opened"));
        };
        let process = Self {
            child,
            group,
            folder: launch.folder.clone(),
        };
        Ok((process, Pipes { stdin, stdout }))
    }

    /// The OS process ID of the started child while it is still alive.
    #[must_use]
    pub fn pid(&self) -> Option<u32> {
        self.child.id()
    }

    /// Waits for the instance to end by itself.
    ///
    /// # Errors
    ///
    /// When the OS cannot tell how it ended.
    ///
    /// # Cancel safety
    ///
    /// Cancel-safe: dropping the future leaves the instance running.
    pub async fn ended(&mut self) -> io::Result<Ended> {
        self.child.wait().await.map(Ended::of)
    }

    /// Kills the instance with every process of its group and waits for it,
    /// leaving its private temporary folder intact for a restart.
    pub async fn kill_group(mut self) {
        self.group.kill_all();
        let _ = self.child.wait().await;
    }

    /// Kills the instance with every process of its group, waits for it,
    /// then deletes its private temporary folder on `owner`'s blocking
    /// policy.
    ///
    /// # Errors
    ///
    /// When the folder cannot be deleted.
    pub async fn stop(mut self, owner: &TaskOwner) -> io::Result<()> {
        self.group.kill_all();
        // Already ended or killed: the status no longer matters.
        let _ = self.child.wait().await;
        let folder = self.folder;
        owner
            .blocking(move || {
                #[expect(
                    clippy::disallowed_methods,
                    reason = "it runs on the tasks module's blocking policy"
                )]
                match std::fs::remove_dir_all(&folder) {
                    Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error),
                    _ => Ok(()),
                }
            })
            .await
    }
}

/// How an instance ended by itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ended {
    /// Its launcher found that this host cannot confine it, so it never
    /// started: `confinement_unavailable`.
    ConfinementUnavailable,
    /// The OS killed it for exceeding a declared resource limit (such as
    /// `SIGXCPU` for `RLIMIT_CPU` or `SIGABRT` on `RLIMIT_DATA` exhaustion).
    LimitExceeded(PluginResourceLimit),
    /// It exited with this status, its launcher's when it never started.
    Exited(Option<i32>),
}

impl Ended {
    fn of(status: ExitStatus) -> Self {
        #[cfg(unix)]
        {
            use std::os::unix::process::ExitStatusExt as _;
            match status.signal() {
                Some(libc::SIGXCPU) => return Self::LimitExceeded(PluginResourceLimit::CpuTime),
                Some(libc::SIGABRT) => return Self::LimitExceeded(PluginResourceLimit::Memory),
                _ => {}
            }
        }
        match status.code() {
            Some(code)
                if u8::try_from(code).is_ok_and(|code| UNAVAILABLE_STATUSES.contains(&code)) =>
            {
                Self::ConfinementUnavailable
            }
            code => Self::Exited(code),
        }
    }
}

fn shim_command(
    launcher: &Path,
    launch: &Launch,
    launcher_env: &[(String, String)],
) -> io::Result<Command> {
    let request = serde_json::to_string(launch).map_err(io::Error::other)?;
    let mut command = Command::new(launcher);
    command
        .arg(SHIM_COMMAND)
        .env_clear()
        .env(LAUNCH_VARIABLE, request)
        .envs(launcher_env.iter().map(|(k, v)| (k.as_str(), v.as_str())))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    Ok(command)
}

#[cfg(all(test, unix))]
mod tests;
