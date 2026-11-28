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

use std::io::{self, BufRead as _, BufReader, Write as _};
use std::path::{Path, PathBuf};
use std::process::{ExitStatus, Stdio};
use std::time::Duration;

use crate::env::TestEnv;
use crate::process::{Process, ReportedAddressError, first_line_within};
use crate::runtime_file::{DaemonEndpoint, RuntimeFile, RuntimeFileError};
use crate::tools::{Product, ToolsError, tools};

/// How long the Daemon has to print its runtime file once started.
pub const DAEMON_DEADLINE: Duration = Duration::from_secs(30);

/// How long the Daemon has to end once `daemon stop` answered.
const STOP_DEADLINE: Duration = Duration::from_secs(10);

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
    /// The HTTP JSON-RPC listener's base URL (`http://127.0.0.1:<port>`),
    /// present only when started with `--http-listener` (ADRs 0027 and 0070).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "String")]
    pub http: Option<String>,
    /// The Bearer token the HTTP listener takes in `Authorization: Bearer …`,
    /// present only when started with `--http-listener` (ADRs 0027 and 0070).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "String")]
    pub http_token: Option<String>,
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

    /// Starts the Daemon of `product`'s executable from the test tool
    /// manifest with its HTTP JSON-RPC listener enabled on loopback port 0
    /// (ADRs 0027 and 0070).
    pub fn start_with_http_listener(env: &TestEnv, product: Product) -> Result<Self, DaemonError> {
        let tools = tools()?;
        Self::start_program_with_http_listener(env, tools.path(product.tool())?, DAEMON_DEADLINE)
    }

    /// Starts `program`, a Product's executable, as the Daemon of the
    /// test's home, waiting at most `deadline` for its runtime file.
    pub fn start_program(
        env: &TestEnv,
        program: &Path,
        deadline: Duration,
    ) -> Result<Self, DaemonError> {
        Self::start_inner(env, program, deadline, false)
    }

    /// Starts `program` as the Daemon of the test's home with its HTTP
    /// JSON-RPC listener enabled on loopback port 0, describing `daemon.http`
    /// and `daemon.http_token` (ADRs 0027 and 0070).
    pub fn start_program_with_http_listener(
        env: &TestEnv,
        program: &Path,
        deadline: Duration,
    ) -> Result<Self, DaemonError> {
        Self::start_inner(env, program, deadline, true)
    }

    fn start_inner(
        env: &TestEnv,
        program: &Path,
        deadline: Duration,
        http_listener: bool,
    ) -> Result<Self, DaemonError> {
        if http_listener {
            env.append_user_configuration("http_listener.enabled = true\nhttp_listener.port = 0\n");
        }
        let mut command = env.process_command(program);
        command.args(["daemon", "launched", "--keep-alive"]);
        let process = env.spawn(&mut command).map_err(DaemonError::Start)?;
        // On any failure the process is stopped, so the test's teardown
        // finds none alive.
        let started = Self::read(env, &process, deadline).and_then(
            |(runtime_file, mut description, endpoint, version)| {
                if http_listener {
                    let (http, http_token) = discover_http_listener(
                        env,
                        program,
                        &endpoint,
                        &description.token,
                        &version,
                        deadline,
                    )?;
                    description.http = Some(http);
                    description.http_token = Some(http_token);
                }
                Ok((runtime_file, description, endpoint))
            },
        );
        match started {
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
    ) -> Result<(PathBuf, DaemonDescription, DaemonEndpoint, String), DaemonError> {
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
            http: None,
            http_token: None,
        };
        Ok((path, description, published.endpoint, published.version))
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
        match self
            .process
            .wait_within(STOP_DEADLINE)
            .map_err(DaemonError::Stop)?
        {
            Some(_) => Ok(()),
            None => Err(DaemonError::StillRunning(STOP_DEADLINE)),
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

/// Discovers the running Daemon's bound HTTP listener base URL (via
/// `daemon.status` over its socket) and its Bearer token (via
/// `<program> daemon http-listener-token --json`).
fn discover_http_listener(
    env: &TestEnv,
    program: &Path,
    endpoint: &DaemonEndpoint,
    socket_token: &str,
    product_version: &str,
    deadline: Duration,
) -> Result<(String, String), DaemonError> {
    let address = query_http_listener_address(endpoint, socket_token, product_version, deadline)?;
    let token = query_http_listener_token(env, program)?;
    Ok((format!("http://{address}"), token))
}

#[expect(
    clippy::disallowed_types,
    reason = "test support connects to the started Daemon's socket to read its bound HTTP listener address (ADRs 0060 and 0062)"
)]
fn connect_daemon(
    endpoint: &DaemonEndpoint,
    deadline: Duration,
) -> io::Result<(Box<dyn io::Write>, Box<dyn io::Read>)> {
    match endpoint {
        #[cfg(unix)]
        DaemonEndpoint::Unix(path) => {
            let stream = std::os::unix::net::UnixStream::connect(path)?;
            stream.set_read_timeout(Some(deadline))?;
            Ok((Box::new(stream.try_clone()?), Box::new(stream)))
        }
        #[cfg(not(unix))]
        DaemonEndpoint::Unix(_) => Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "a Daemon listens on loopback TCP here",
        )),
        DaemonEndpoint::Tcp(address) => {
            let stream = std::net::TcpStream::connect(address)?;
            stream.set_read_timeout(Some(deadline))?;
            Ok((Box::new(stream.try_clone()?), Box::new(stream)))
        }
    }
}

fn query_http_listener_address(
    endpoint: &DaemonEndpoint,
    socket_token: &str,
    product_version: &str,
    deadline: Duration,
) -> Result<String, DaemonError> {
    let (mut writer, reader) = connect_daemon(endpoint, deadline).map_err(|error| {
        DaemonError::HttpListener(format!("connecting to Daemon socket: {error}"))
    })?;
    let mut reader = BufReader::new(reader);
    let hello = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 0,
        "method": "hello",
        "params": {
            "protocol_version": "0.1",
            "surface": "cli",
            "token": socket_token,
            "needs": "collie",
            "product_version": product_version,
        },
    });
    writeln!(writer, "{hello}")
        .and_then(|()| writer.flush())
        .map_err(|error| DaemonError::HttpListener(format!("sending hello: {error}")))?;
    let mut hello_line = String::new();
    reader
        .read_line(&mut hello_line)
        .map_err(|error| DaemonError::HttpListener(format!("reading hello response: {error}")))?;
    let hello_response: serde_json::Value = serde_json::from_str(&hello_line)
        .map_err(|error| DaemonError::HttpListener(format!("parsing hello response: {error}")))?;
    if hello_response.get("result").is_none() {
        return Err(DaemonError::HttpListener(format!(
            "Daemon refused hello: {hello_response}"
        )));
    }
    let status = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "daemon.status",
        "params": {},
    });
    writeln!(writer, "{status}")
        .and_then(|()| writer.flush())
        .map_err(|error| DaemonError::HttpListener(format!("sending daemon.status: {error}")))?;
    let mut status_line = String::new();
    reader
        .read_line(&mut status_line)
        .map_err(|error| DaemonError::HttpListener(format!("reading daemon.status: {error}")))?;
    let status_response: serde_json::Value = serde_json::from_str(&status_line)
        .map_err(|error| DaemonError::HttpListener(format!("parsing daemon.status: {error}")))?;
    status_response["result"]["http_listener"]
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| {
            DaemonError::HttpListener(format!(
                "daemon.status reported no http_listener address: {status_response}"
            ))
        })
}

fn query_http_listener_token(env: &TestEnv, program: &Path) -> Result<String, DaemonError> {
    let mut command = env.process_command(program);
    command
        .args(["daemon", "http-listener-token", "--json"])
        .stdin(Stdio::null());
    let output = command.output().map_err(|error| {
        DaemonError::HttpListener(format!("running `daemon http-listener-token`: {error}"))
    })?;
    if !output.status.success() {
        return Err(DaemonError::HttpListener(format!(
            "`daemon http-listener-token` exited with {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        )));
    }
    let parsed: serde_json::Value = serde_json::from_slice(&output.stdout).map_err(|error| {
        DaemonError::HttpListener(format!("parsing `daemon http-listener-token`: {error}"))
    })?;
    parsed["token"]
        .as_str()
        .filter(|token| !token.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| {
            DaemonError::HttpListener(format!(
                "`daemon http-listener-token` printed no token: {parsed}"
            ))
        })
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
    /// The Daemon's HTTP listener address or token could not be discovered.
    #[error("the Daemon's HTTP listener could not be described: {0}")]
    HttpListener(String),
}
