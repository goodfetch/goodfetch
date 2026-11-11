//! The Daemon started for one test (ADRs 0024, 0062 and 0069): a Product's
//! executable from the test tool manifest, Collie's unless the test needs
//! Foxhound's (ADR 0094), run as the Daemon of the test's own Goodfetch
//! home under its cleared environment. It is started as `daemon launched
//! --keep-alive`, the process a `daemon start` runs, so it is the test's
//! own child, joins the test's record and never stops for idleness while
//! the test runs. Its runtime file, which it prints once ready, is read
//! back from the home and gives the socket and token a Client needs.
//! Stopping it runs the executable's own `daemon stop`, then checks that
//! the process ended and that no runtime file is left in the home. Dropping
//! a Daemon the test did not stop stops it the same way, so it ends its
//! Runs and its children before it exits, and kills it only when that
//! fails or outlasts the deadline; either way it is reaped.
#![expect(
    clippy::disallowed_methods,
    reason = "test support reads the Daemon's runtime file, runs `daemon stop` and waits for the Daemon outside any async worker (ADR 0062)"
)]

use std::io;
use std::path::{Path, PathBuf};
use std::process::{ExitStatus, Stdio};
use std::time::{Duration, Instant};

use crate::env::TestEnv;
use crate::process::{Process, ReportedAddressError, first_line_within};
use crate::runtime_file::{DaemonEndpoint, RuntimeFile, RuntimeFileError};
use crate::tools::{Product, ToolsError, tools};

/// How long the Daemon has to print its runtime file once started.
pub const DAEMON_DEADLINE: Duration = Duration::from_secs(30);

/// How long the Daemon has to end once `daemon stop` answered.
const STOP_DEADLINE: Duration = Duration::from_secs(10);

/// How often a stopping Daemon is looked at until it ends.
const STOP_POLL: Duration = Duration::from_millis(10);

/// A started Daemon, as a test and `goodfetch-test-env`'s description see
/// it: what a Client needs to reach it.
#[derive(
    Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
#[serde(deny_unknown_fields)]
pub struct DaemonDescription {
    /// Where it listens: its Unix socket's path on macOS and Linux, its
    /// loopback TCP address (`127.0.0.1:<port>`) on Windows (ADR 0024).
    pub socket: String,
    /// The per-launch token a Client's `hello` carries.
    pub token: String,
}

/// The Daemon of one test's Goodfetch home. Its process joins the test's
/// record; [`Daemon::stop`] stops it, and dropping it stops it too,
/// killing it when it does not end.
pub struct Daemon {
    process: Process,
    /// Its executable's `daemon stop`, under the test's environment, for
    /// its drop, which has no `TestEnv` to build it from.
    stop_command: std::process::Command,
    runtime_file: PathBuf,
    description: DaemonDescription,
    /// Where it listens, as its runtime file names it.
    endpoint: DaemonEndpoint,
}

impl Daemon {
    /// Starts the Daemon of `product`'s executable from the test tool
    /// manifest.
    pub fn start(env: &TestEnv, product: Product) -> Result<Self, DaemonError> {
        let tools = tools()?;
        Self::start_program(env, tools.path(product.tool())?, DAEMON_DEADLINE)
    }

    /// Starts `program`, a Product's executable, as the Daemon of the
    /// test's home, waiting at most `deadline` for its runtime file.
    pub fn start_program(
        env: &TestEnv,
        program: &Path,
        deadline: Duration,
    ) -> Result<Self, DaemonError> {
        let mut command = env.process_command(program);
        command.args(["daemon", "launched", "--keep-alive"]);
        let process = env.spawn(&mut command).map_err(DaemonError::Start)?;
        // On any failure the process is stopped, so the test's teardown
        // finds none alive.
        match Self::read(env, &process, deadline) {
            Ok((runtime_file, description, endpoint)) => Ok(Self {
                process,
                stop_command: stop_command(env, program),
                runtime_file,
                description,
                endpoint,
            }),
            Err(error) => {
                kill(&process);
                Err(error)
            }
        }
    }

    fn read(
        env: &TestEnv,
        process: &Process,
        deadline: Duration,
    ) -> Result<(PathBuf, DaemonDescription, DaemonEndpoint), DaemonError> {
        let mut child = process.child();
        let line = match first_line_within(&mut child, deadline) {
            Ok(line) => line,
            Err(ReportedAddressError::Closed) => {
                return Err(DaemonError::Exited(
                    child.wait().map_err(DaemonError::Start)?,
                ));
            }
            Err(error) => return Err(DaemonError::Line(error)),
        };
        let printed: RuntimeFile =
            serde_json::from_str(&line).map_err(|error| DaemonError::NotRuntimeFile {
                line: line.clone(),
                reason: error.to_string(),
            })?;
        let path = RuntimeFile::path(env.home(), &printed.version);
        let published = RuntimeFile::read(&path)?;
        if published != printed {
            return Err(DaemonError::Unpublished { path });
        }
        let description = DaemonDescription {
            socket: published.endpoint.to_string(),
            token: published.token,
        };
        Ok((path, description, published.endpoint))
    }

    /// What a Client reads of it, and what `goodfetch-test-env` describes.
    pub fn description(&self) -> &DaemonDescription {
        &self.description
    }

    /// Where it listens: the endpoint a Client connects to.
    pub fn endpoint(&self) -> &DaemonEndpoint {
        &self.endpoint
    }

    /// Its process ID, such as for reading its private resident memory while
    /// it runs.
    pub fn pid(&self) -> u32 {
        self.process.child().id()
    }

    /// Stops it with its executable's `daemon stop`, under the test's
    /// environment, and checks that its process ended and that it left no
    /// runtime file in the home.
    pub fn stop(mut self, env: &TestEnv) -> Result<(), DaemonError> {
        self.stop_command = stop_command(env, &self.program());
        self.own_stop()?;
        if self.runtime_file.exists() {
            return Err(DaemonError::RuntimeFileLeft(self.runtime_file.clone()));
        }
        Ok(())
    }
}

impl Daemon {
    /// The executable its `daemon stop` runs.
    fn program(&self) -> PathBuf {
        PathBuf::from(self.stop_command.get_program())
    }

    /// Runs its `daemon stop` and waits until its process ends.
    fn own_stop(&mut self) -> Result<(), DaemonError> {
        let output = self.stop_command.output().map_err(DaemonError::Stop)?;
        if !output.status.success() {
            return Err(DaemonError::StopFailed {
                status: output.status,
                stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            });
        }
        let started = Instant::now();
        loop {
            if self
                .process
                .child()
                .try_wait()
                .map_err(DaemonError::Stop)?
                .is_some()
            {
                return Ok(());
            }
            if started.elapsed() >= STOP_DEADLINE {
                return Err(DaemonError::StillRunning(STOP_DEADLINE));
            }
            std::thread::sleep(STOP_POLL);
        }
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        if matches!(self.process.child().try_wait(), Ok(None)) {
            // Best effort: a stop that fails or outlasts its deadline leaves
            // the kill below.
            let _ = self.own_stop();
        }
        kill(&self.process);
    }
}

/// `program`'s `daemon stop`, under the test's environment.
fn stop_command(env: &TestEnv, program: &Path) -> std::process::Command {
    let mut command = env.process_command(program);
    command
        .args(["daemon", "stop", "--json"])
        .stdin(Stdio::null());
    command
}

/// Kills a process that may already have exited, and reaps it.
fn kill(process: &Process) {
    let mut child = process.child();
    // Best effort: a process that already exited needs only its wait.
    let _ = child.kill();
    let _ = child.wait();
}

/// Why a Daemon did not start, or did not stop cleanly.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum DaemonError {
    /// The test tool manifest names no such executable.
    #[error(transparent)]
    Tools(#[from] ToolsError),
    /// Its process did not start.
    #[error("the Daemon's process did not start: {0}")]
    Start(io::Error),
    /// It printed no line within the deadline.
    #[error("the Daemon printed no runtime file once ready: {0}")]
    Line(ReportedAddressError),
    /// It exited before it printed its runtime file; its own message is on
    /// stderr.
    #[error("the Daemon exited ({0}) before it was ready; its message is on stderr")]
    Exited(ExitStatus),
    /// Its first line is no runtime file.
    #[error("the Daemon's first line is no runtime file ({reason}): {line}")]
    NotRuntimeFile {
        /// The line.
        line: String,
        /// Why.
        reason: String,
    },
    /// Its runtime file cannot be read in the home.
    #[error(transparent)]
    RuntimeFile(#[from] RuntimeFileError),
    /// The home's runtime file is not the one it printed.
    #[error("the Daemon's runtime file {} is not the one it printed", path.display())]
    Unpublished {
        /// The runtime file.
        path: PathBuf,
    },
    /// `daemon stop` or the wait for the Daemon failed to run.
    #[error("the Daemon could not be stopped: {0}")]
    Stop(io::Error),
    /// `daemon stop` failed.
    #[error("`daemon stop` exited with {status}: {stderr}")]
    StopFailed {
        /// Its exit status.
        status: ExitStatus,
        /// Its stderr.
        stderr: String,
    },
    /// The Daemon still ran after `daemon stop` answered.
    #[error("the Daemon still runs {0:?} after `daemon stop`")]
    StillRunning(Duration),
    /// The Daemon left its runtime file in the home once stopped.
    #[error("the Daemon left its runtime file {} once stopped", .0.display())]
    RuntimeFileLeft(PathBuf),
}
