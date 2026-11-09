//! The resident-memory scenarios (ADRs 0071, 0094 and 0134): an Attached
//! engine idle after `hello` and an idle Daemon, on each Product's
//! executable, and the 1,000-page HTTP crawl of the local Proving ground,
//! on Foxhound's. Each names the Products whose executables it runs on, its
//! Performance target and how it drives the executable.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous: a scenario idles its child on a blocking sleep and writes its home's configuration with blocking fs (STYLE.md)"
)]

use std::ffi::OsString;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, ChildStdout, Command};
use std::time::Duration;

use anyhow::{Context as _, Result, bail};
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::connection::{HELLO, HelloParams, ProductVersion, Surface};
use goodfetch_protocol::console::ConsoleState;
use goodfetch_protocol::daemon::{DaemonEndpoint, DaemonRuntimeFile};
use goodfetch_protocol::home::{USER_CONFIGURATION_FILE, per_major};
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

/// How long a Daemon may take to exit once it answered `daemon.stop`: the
/// deadline test support's `Daemon::stop` gives one.
const STOP_DEADLINE: Duration = Duration::from_secs(10);

/// How often the probe looks whether the stopped Daemon exited.
const EXIT_POLL: Duration = Duration::from_millis(50);

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
            Self::IdleAttachedEngine | Self::Crawl => &[goodfetch_protocol::cli::ATTACHED_ENGINE],
            Self::IdleDaemon => &["daemon", "launched", "--keep-alive"],
        }
    }

    /// Whether it crawls the Proving ground, which `measure` serves outside
    /// the probe, so its memory is never the measured process's.
    pub(crate) fn crawls(self) -> bool {
        self == Self::Crawl
    }

    /// Drives the started executable, `child`, through the scenario until
    /// it exits: an Attached engine's input ends, a Daemon is stopped.
    /// `site` is the Proving ground's, which only the crawl reads.
    pub(crate) fn drive(
        self,
        needs: Product,
        site: Option<&Site<'_>>,
        child: &mut Child,
    ) -> Result<()> {
        let stdin = child.stdin.take().context("the engine's stdin is piped")?;
        let stdout = child
            .stdout
            .take()
            .context("the engine's stdout is piped")?;
        match self {
            Self::IdleAttachedEngine => idle_attached_engine(needs, stdin, stdout),
            Self::IdleDaemon => idle_daemon(needs, child, stdin, stdout),
            Self::Crawl => {
                let site = site.context("the crawl needs the Proving ground's address")?;
                crawl::crawl(stdin, stdout, site)
            }
        }
    }
}

/// Runs the engine `command` starts as shipped: with `home` as its data
/// folder and none of `goodfetch_variables`, this process's other Goodfetch
/// variables, since an engine stops on one it does not know and logs at
/// its default level without `GOODFETCH_LOG`.
pub(crate) fn as_shipped(command: &mut Command, home: &Path, goodfetch_variables: &[OsString]) {
    for name in goodfetch_variables {
        command.env_remove(name);
    }
    command.env(goodfetch_protocol::settings::HOME_VARIABLE, home);
}

/// The idle-Daemon scenario's user configuration: the Daemon's Console
/// listens on a port the OS chooses, never on its major's fixed port
/// (TESTING.md, Isolation), so scenarios run at once and beside a Daemon of
/// the machine's own.
const DAEMON_CONFIGURATION: &str =
    "version = \"0.1\"\n\n[settings]\nconsole.address = \"127.0.0.1:0\"\n";

/// Writes `home`'s user configuration for `scenario`: the idle Daemon's
/// alone; an Attached engine's home has none, as shipped.
pub(crate) fn configure(scenario: Scenario, home: &Path) -> Result<()> {
    if scenario != Scenario::IdleDaemon {
        return Ok(());
    }
    let version = ProductVersion::parse(env!("CARGO_PKG_VERSION"))
        .context("reading the workspace's Product version")?;
    let major = per_major(home, &version);
    std::fs::create_dir_all(&major).with_context(|| format!("creating {}", major.display()))?;
    let configuration = major.join(USER_CONFIGURATION_FILE);
    std::fs::write(&configuration, DAEMON_CONFIGURATION)
        .with_context(|| format!("writing {}", configuration.display()))
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
            token,
            needs: Some(needs),
            ..HelloParams::new(
                Surface::Rust,
                ProductVersion::parse(env!("CARGO_PKG_VERSION"))
                    .context("xtask's version is a Product version")?,
            )
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

/// Waits for the Daemon's runtime file, checks its Console serves, as a
/// shipped Daemon's does, idles, then stops it over its own endpoint with
/// `daemon.stop`, as `daemon stop` does, so no other process the probe
/// waits for adds to the peak, and waits for it to exit, within
/// [`STOP_DEADLINE`].
fn idle_daemon(
    needs: Product,
    child: &mut Child,
    stdin: ChildStdin,
    stdout: ChildStdout,
) -> Result<()> {
    let mut stdout = BufReader::new(stdout);
    let mut line = String::new();
    if stdout
        .read_line(&mut line)
        .context("reading the Daemon's runtime file")?
        == 0
    {
        bail!("the Daemon ended before it printed its runtime file");
    }
    let runtime: DaemonRuntimeFile = serde_json::from_str(&line).with_context(|| {
        format!(
            "the Daemon's first line is no runtime file: {}",
            line.trim_end()
        )
    })?;
    if !matches!(runtime.console, ConsoleState::Serving(_)) {
        bail!(
            "the Daemon's Console does not serve, so it is not the Daemon as shipped: {:?}",
            runtime.console
        );
    }
    std::thread::sleep(DAEMON_IDLE);
    let (reader, mut writer) = connect(&runtime.endpoint)?;
    let mut reader = BufReader::new(reader);
    writer
        .write_all(&hello_with(needs, Some(runtime.token.expose().to_owned()))?)
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
    exits_within(child, STOP_DEADLINE)?;
    // What it wrote last, which its exit closed.
    std::io::copy(&mut stdout, &mut std::io::sink()).context("reading the Daemon's output")?;
    Ok(())
}

/// Waits for the stopped Daemon `child` to exit, for at most `deadline`.
#[expect(
    clippy::disallowed_methods,
    reason = "xtask has no Clock handle (ADR 0059): the probe bounds its wait for the Daemon on the OS's monotonic clock, polling on a blocking sleep"
)]
fn exits_within(child: &mut Child, deadline: Duration) -> Result<()> {
    let started = std::time::Instant::now();
    while child
        .try_wait()
        .context("waiting for the Daemon to exit")?
        .is_none()
    {
        if started.elapsed() >= deadline {
            bail!(
                "the Daemon did not exit within {} s of daemon.stop",
                deadline.as_secs()
            );
        }
        std::thread::sleep(EXIT_POLL);
    }
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
fn connect(endpoint: &DaemonEndpoint) -> Result<(Box<dyn Read>, Box<dyn Write>)> {
    match endpoint {
        DaemonEndpoint::Tcp(address) => {
            let stream = std::net::TcpStream::connect(address)
                .with_context(|| format!("connecting to the Daemon at {address}"))?;
            Ok((Box::new(stream.try_clone()?), Box::new(stream)))
        }
        #[cfg(unix)]
        DaemonEndpoint::Unix(path) => {
            let stream = std::os::unix::net::UnixStream::connect(path)
                .with_context(|| format!("connecting to the Daemon at {}", path.display()))?;
            Ok((Box::new(stream.try_clone()?), Box::new(stream)))
        }
        #[cfg(not(unix))]
        DaemonEndpoint::Unix(path) => bail!(
            "the Daemon listens on a Unix socket, {}, which this OS has none of",
            path.display()
        ),
    }
}
