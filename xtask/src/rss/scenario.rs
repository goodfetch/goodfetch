//! The resident-memory scenarios (ADRs 0071, 0094 and 0134): an Attached
//! engine idle after `hello` and an idle Daemon, on each Product's
//! executable, and the 1,000-page HTTP crawl of the local Proving ground,
//! on Foxhound's. Each names the Products whose executables it runs on, its
//! Performance target and how it drives the executable.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous: a scenario idles its child on a blocking sleep (STYLE.md)"
)]

use std::io::{BufRead, BufReader, Read, Write};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::{ChildStdin, ChildStdout, Command};
use std::time::Duration;

use anyhow::{Context as _, Result, bail};
use goodfetch_protocol::catalog::{PROTOCOL_VERSION, Product};
use goodfetch_protocol::connection::{HELLO, HelloParams, ProductVersion, Surface};
use goodfetch_protocol::jsonrpc::{Message, Request, RequestId, Version};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::crawl::{self, Site};

const MIB: u64 = 1024 * 1024;

/// How long an Attached engine stays idle after its `hello`, so the peak
/// covers what it allocates once settled, not only its start.
const IDLE: Duration = Duration::from_secs(1);

/// How long a Daemon stays idle once it printed its runtime file: longer
/// than an Attached engine, since its listener, Console and timers start
/// beside it.
const DAEMON_IDLE: Duration = Duration::from_secs(3);

#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, clap::ValueEnum,
)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Scenario {
    /// An Attached engine idle after `hello`.
    IdleAttachedEngine,
    /// A Daemon with no run, idle once it printed its runtime file.
    IdleDaemon,
    /// Foxhound's code-free HTTP crawl of the Proving ground's 1,000-page
    /// site at M1's concurrency.
    Crawl,
}

impl Scenario {
    pub(crate) const ALL: [Self; 3] = [Self::IdleAttachedEngine, Self::IdleDaemon, Self::Crawl];

    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::IdleAttachedEngine => "idle-attached-engine",
            Self::IdleDaemon => "idle-daemon",
            Self::Crawl => "crawl",
        }
    }

    /// The Products whose executables it runs on: each, for a figure of the
    /// executable itself, and Foxhound alone for the crawl, which it carries
    /// (ADR 0094).
    pub(crate) fn products(self) -> &'static [Product] {
        match self {
            Self::IdleAttachedEngine | Self::IdleDaemon => &[Product::Collie, Product::Foxhound],
            Self::Crawl => &[Product::Foxhound],
        }
    }

    /// Its Performance target in bytes (ADR 0134), which never gates.
    pub(crate) fn target_bytes(self) -> u64 {
        match self {
            Self::IdleAttachedEngine => 8 * MIB,
            Self::IdleDaemon => 16 * MIB,
            Self::Crawl => 32 * MIB,
        }
    }

    /// The arguments the executable takes for this scenario: the Daemon
    /// as `daemon start` launches it, kept alive so it never stops idle
    /// before it is stopped.
    pub(crate) fn arguments(self) -> &'static [&'static str] {
        match self {
            Self::IdleAttachedEngine | Self::Crawl => &["attached-engine"],
            Self::IdleDaemon => &["daemon", "launched", "--keep-alive"],
        }
    }

    /// Whether it crawls the Proving ground, which `measure` serves outside
    /// the probe, so its memory is never the measured process's.
    pub(crate) fn crawls(self) -> bool {
        self == Self::Crawl
    }

    /// Drives the started executable through the scenario until it exits:
    /// an Attached engine's input ends, a Daemon is stopped. `site` is the
    /// Proving ground's, which only the crawl reads.
    pub(crate) fn drive(
        self,
        needs: Product,
        site: Option<&Site<'_>>,
        stdin: ChildStdin,
        stdout: ChildStdout,
    ) -> Result<()> {
        match self {
            Self::IdleAttachedEngine => idle_attached_engine(needs, stdin, stdout),
            Self::IdleDaemon => idle_daemon(needs, stdin, stdout),
            Self::Crawl => {
                let site = site.context("the crawl needs the Proving ground's address")?;
                crawl::crawl(stdin, stdout, site)
            }
        }
    }
}

/// Runs the engine `command` starts as shipped: with `home` as its data
/// folder and no other Goodfetch variable of this process's environment,
/// since an engine stops on one it does not know and logs at its default
/// level without `GOODFETCH_LOG`.
pub(crate) fn as_shipped(command: &mut Command, home: &Path) {
    for (name, _) in std::env::vars_os() {
        if name.to_string_lossy().starts_with("GOODFETCH_") {
            command.env_remove(name);
        }
    }
    command.env("GOODFETCH_HOME", home);
}

/// The `hello` line a Client needing `needs` opens with, as the engine
/// reads it from its input.
pub(crate) fn hello_line(needs: Product) -> Result<Vec<u8>> {
    hello_with(needs, None)
}

/// The `hello` line, with the Daemon's per-launch token when it goes to a
/// Daemon.
fn hello_with(needs: Product, token: Option<String>) -> Result<Vec<u8>> {
    let hello = Message::Request(Request {
        jsonrpc: Version,
        id: RequestId::Number(0),
        method: HELLO.to_owned(),
        params: Some(serde_json::to_value(HelloParams {
            protocol_version: PROTOCOL_VERSION,
            surface: Surface::Rust,
            form: None,
            token,
            needs: Some(needs),
            product_version: ProductVersion::parse(env!("CARGO_PKG_VERSION"))
                .context("xtask's version is a Product version")?,
            log_level: None,
        })?),
    });
    let mut line = serde_json::to_vec(&hello)?;
    line.push(b'\n');
    Ok(line)
}

/// Opens with `hello`, checks the engine accepted it, idles, and ends the
/// input.
fn idle_attached_engine(needs: Product, mut stdin: ChildStdin, stdout: ChildStdout) -> Result<()> {
    stdin
        .write_all(&hello_line(needs)?)
        .context("sending hello")?;
    stdin.flush().context("sending hello")?;
    let mut stdout = BufReader::new(stdout);
    let mut reply = String::new();
    stdout
        .read_line(&mut reply)
        .context("reading the reply to hello")?;
    match serde_json::from_str::<Message>(&reply) {
        Ok(Message::Response(_)) => {}
        _ => bail!("the engine did not accept hello: {}", reply.trim_end()),
    }
    std::thread::sleep(IDLE);
    drop(stdin);
    // Its output stays open until it exits, so ending never fails a write.
    std::io::copy(&mut stdout, &mut std::io::sink()).context("reading the engine's output")?;
    Ok(())
}

/// Where a Daemon listens, as its runtime file names it (ADR 0024).
#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum Endpoint {
    Unix(PathBuf),
    Tcp(SocketAddr),
}

/// The parts of the runtime file a Daemon prints once ready that a Client
/// needs.
#[derive(Deserialize)]
struct RuntimeFile {
    endpoint: Endpoint,
    token: String,
}

/// Waits for the Daemon's runtime file, idles, then stops it over its own
/// endpoint with `daemon.stop`, as `daemon stop` does, so no other process
/// the probe waits for adds to the peak, and waits for its output to end.
fn idle_daemon(needs: Product, stdin: ChildStdin, stdout: ChildStdout) -> Result<()> {
    let mut stdout = BufReader::new(stdout);
    let mut line = String::new();
    if stdout
        .read_line(&mut line)
        .context("reading the Daemon's runtime file")?
        == 0
    {
        bail!("the Daemon ended before it printed its runtime file");
    }
    let runtime: RuntimeFile = serde_json::from_str(&line).with_context(|| {
        format!(
            "the Daemon's first line is no runtime file: {}",
            line.trim_end()
        )
    })?;
    std::thread::sleep(DAEMON_IDLE);
    let (reader, mut writer) = connect(&runtime.endpoint)?;
    let mut reader = BufReader::new(reader);
    writer
        .write_all(&hello_with(needs, Some(runtime.token))?)
        .and_then(|()| writer.flush())
        .context("sending hello to the Daemon")?;
    reply(&mut reader, "hello")?;
    let mut stop = serde_json::to_vec(&json!({
        "jsonrpc": "2.0", "id": 1, "method": "daemon.stop", "params": {},
    }))?;
    stop.push(b'\n');
    writer
        .write_all(&stop)
        .and_then(|()| writer.flush())
        .context("sending daemon.stop")?;
    reply(&mut reader, "daemon.stop")?;
    drop((reader, writer));
    drop(stdin);
    std::io::copy(&mut stdout, &mut std::io::sink()).context("reading the Daemon's output")?;
    Ok(())
}

/// Reads the Daemon's next line, the success answering `method`.
fn reply(reader: &mut impl BufRead, method: &str) -> Result<()> {
    let mut line = String::new();
    if reader
        .read_line(&mut line)
        .with_context(|| format!("reading the answer to {method}"))?
        == 0
    {
        bail!("the Daemon closed the connection before it answered {method}");
    }
    let answer: Value = serde_json::from_str(&line)
        .with_context(|| format!("the Daemon wrote no JSON: {}", line.trim_end()))?;
    if answer.get("result").is_none() {
        bail!("the Daemon refused {method}: {answer}");
    }
    Ok(())
}

/// A Connection to the Daemon's endpoint, its two halves.
#[expect(
    clippy::disallowed_types,
    reason = "xtask is synchronous and no engine: it reaches the measured Daemon's endpoint as a Client, as `daemon stop` does (ADR 0060 bans sockets in the engine's async crates)"
)]
fn connect(endpoint: &Endpoint) -> Result<(Box<dyn Read>, Box<dyn Write>)> {
    match endpoint {
        Endpoint::Tcp(address) => {
            let stream = std::net::TcpStream::connect(address)
                .with_context(|| format!("connecting to the Daemon at {address}"))?;
            Ok((Box::new(stream.try_clone()?), Box::new(stream)))
        }
        #[cfg(unix)]
        Endpoint::Unix(path) => {
            let stream = std::os::unix::net::UnixStream::connect(path)
                .with_context(|| format!("connecting to the Daemon at {}", path.display()))?;
            Ok((Box::new(stream.try_clone()?), Box::new(stream)))
        }
        #[cfg(not(unix))]
        Endpoint::Unix(path) => bail!(
            "the Daemon listens on a Unix socket, {}, which this OS has none of",
            path.display()
        ),
    }
}
