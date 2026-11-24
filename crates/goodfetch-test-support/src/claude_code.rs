//! The tests' copy of the Claude Code release
//! `test-support/claude-code/pins.toml` pins, which `ci/claude-code-fetch.sh`
//! downloads, and its `plugin validate --strict` of a Product's Claude Code
//! plugin (story 11-239), run with no network: auto-update, telemetry,
//! error reporting and nonessential traffic off, its configuration in the
//! test's folder, and every proxy variable naming a listener of the test,
//! so a connection it tries fails the validation rather than reach the
//! network. The copy is Anthropic's proprietary software, downloaded for
//! the tests alone (the pins file's header).
#![expect(
    clippy::disallowed_methods,
    reason = "test support runs the validator in the test's thread, never on an async worker"
)]
#![expect(
    clippy::disallowed_types,
    reason = "a loopback listener on the test's own thread that only counts the connections the validator tries, never an async worker's socket (ADR 0060)"
)]

use std::fs;
use std::io::{self, ErrorKind};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use serde::Deserialize;

use crate::browsers::downloads;
use crate::env::TestEnv;

/// The variable naming the folder `ci/claude-code-fetch.sh` downloads the
/// tests' copy of Claude Code into, `~/.cache/goodfetch/claude-code` by
/// default.
pub const CLAUDE_CODE_DOWNLOADS_VARIABLE: &str = "GOODFETCH_CLAUDE_CODE_DOWNLOADS";

/// The pins file, read for its release's version.
const PINS: &str = include_str!("../../../test-support/claude-code/pins.toml");

/// The variables that turn off each of Claude Code's own connections.
const OFFLINE: [(&str, &str); 4] = [
    ("DISABLE_AUTOUPDATER", "1"),
    ("DISABLE_TELEMETRY", "1"),
    ("DISABLE_ERROR_REPORTING", "1"),
    ("CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC", "1"),
];

/// The proxy variables Claude Code reads, each pointed at the test's
/// listener.
const PROXIES: [&str; 6] = [
    "HTTPS_PROXY",
    "HTTP_PROXY",
    "ALL_PROXY",
    "https_proxy",
    "http_proxy",
    "all_proxy",
];

/// How often the run is checked for its end and for a connection.
const POLL: Duration = Duration::from_millis(10);

/// Why the validation could not be read.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ClaudeCodeError {
    /// The tests' copy of the pinned release is missing.
    #[error(
        "Claude Code {version} is not at {path}: run `ci/claude-code-fetch.sh` (part of `just fetch`; a CI job also sets GOODFETCH_FETCH_CLAUDE_CODE=yes)"
    )]
    NotFetched {
        /// The pinned release.
        version: String,
        /// Where the copy should be.
        path: PathBuf,
    },
    /// Claude Code connected to the proxy listener: it tried the network.
    #[error("Claude Code tried the network: {0} connection(s) reached the test's proxy")]
    TriedNetwork(usize),
    /// The pins file does not parse.
    #[error("test-support/claude-code/pins.toml does not parse: {0}")]
    Pins(#[from] toml::de::Error),
    /// The copy could not be started or its output read.
    #[error("Claude Code could not run: {0}")]
    Io(#[from] io::Error),
    /// Its standard output is no JSON report.
    #[error("Claude Code printed no JSON report ({error}): {output}")]
    Report {
        /// Why the report does not parse.
        error: serde_json::Error,
        /// Its standard output, then its standard error.
        output: String,
    },
}

/// `plugin validate --strict --json`'s verdict.
#[derive(Debug)]
#[non_exhaustive]
pub struct PluginValidation {
    /// Whether it exited with success.
    pub passed: bool,
    /// Its JSON report: `success`, and the `manifest` it read, `null` for a
    /// folder it validated as a bare skills tree.
    pub report: serde_json::Value,
    /// Its standard output, then its standard error.
    pub output: String,
}

#[derive(Deserialize)]
struct Pins {
    version: String,
}

/// The tests' copy of the pinned release for this platform.
///
/// # Errors
///
/// [`ClaudeCodeError::NotFetched`] when the copy is missing.
pub fn claude_code_copy() -> Result<PathBuf, ClaudeCodeError> {
    let version = toml::from_str::<Pins>(PINS)?.version;
    let executable = if cfg!(windows) {
        "claude.exe"
    } else {
        "claude"
    };
    let path = downloads(CLAUDE_CODE_DOWNLOADS_VARIABLE, "claude-code")
        .join(&version)
        .join(executable);
    if path.is_file() {
        Ok(path)
    } else {
        Err(ClaudeCodeError::NotFetched { version, path })
    }
}

/// Runs the tests' copy's `plugin validate --strict --json` on the plugin
/// at `plugin`, with no network and its configuration in `env`'s folder.
/// The path is made absolute first: on Windows a path rooted with no drive,
/// as a cross-compiled test's `CARGO_MANIFEST_DIR`, would otherwise be read
/// on the drive of the validator's working folder, the test's.
///
/// # Errors
///
/// [`claude_code_copy`]'s, [`ClaudeCodeError::TriedNetwork`] when anything
/// connected to the proxy listener during the run,
/// [`ClaudeCodeError::Io`] when it could not run and
/// [`ClaudeCodeError::Report`] when it printed no JSON report.
pub fn validate_plugin(env: &TestEnv, plugin: &Path) -> Result<PluginValidation, ClaudeCodeError> {
    let copy = claude_code_copy()?;
    let folder = env.folder().join("claude-code");
    fs::create_dir_all(&folder)?;
    let listener = TcpListener::bind(("127.0.0.1", 0))?;
    listener.set_nonblocking(true)?;
    let proxy = format!("http://{}", listener.local_addr()?);
    let (stdout, stderr) = (folder.join("stdout"), folder.join("stderr"));
    let mut command = env.process_command(&copy);
    command
        .args(["plugin", "validate", "--strict", "--json"])
        .arg(std::path::absolute(plugin)?)
        .env("HOME", &folder)
        .env("USERPROFILE", &folder)
        .env("CLAUDE_CONFIG_DIR", folder.join("config"))
        .env("NO_PROXY", "")
        .env("no_proxy", "")
        .envs(OFFLINE)
        .envs(PROXIES.map(|variable| (variable, proxy.as_str())))
        .stdin(Stdio::null())
        .stdout(fs::File::create(&stdout)?)
        .stderr(fs::File::create(&stderr)?);
    let mut child = command.spawn()?;
    let mut connections = 0;
    let status = loop {
        connections += accepted(&listener)?;
        if let Some(status) = child.try_wait()? {
            break status;
        }
        std::thread::sleep(POLL);
    };
    connections += accepted(&listener)?;
    if connections > 0 {
        return Err(ClaudeCodeError::TriedNetwork(connections));
    }
    let report = fs::read_to_string(&stdout)?;
    let output = report.clone() + &fs::read_to_string(&stderr)?;
    match serde_json::from_str(&report) {
        Ok(report) => Ok(PluginValidation {
            passed: status.success(),
            report,
            output,
        }),
        Err(error) => Err(ClaudeCodeError::Report { error, output }),
    }
}

/// The connections waiting on `listener`, each closed once counted.
fn accepted(listener: &TcpListener) -> io::Result<usize> {
    let mut count = 0;
    loop {
        match listener.accept() {
            Ok(_) => count += 1,
            Err(error) if error.kind() == ErrorKind::WouldBlock => return Ok(count),
            Err(error) => return Err(error),
        }
    }
}
