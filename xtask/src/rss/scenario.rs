//! The resident-memory scenarios (ADRs 0071, 0094 and 0134): an Attached
//! engine idle after `hello` and an idle Daemon, on each Product's
//! executable, and the HTTP crawl of the local Proving ground's 100-,
//! 1,000- and 10,000-page sites, on Foxhound's, with `governor.huge_pages`
//! off as shipped and on (spec #2165). Each names the Products whose
//! executables it runs on, its Performance target and how it drives the
//! executable.
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
/// than an Attached engine, since its listener and timers start beside
/// it.
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
    /// Foxhound's code-free HTTP crawl of the Proving ground's 100-page
    /// site at M1's concurrency, as shipped.
    #[value(name = "crawl-100")]
    #[serde(rename = "crawl-100")]
    Crawl100,
    /// The same crawl of the 1,000-page site.
    #[value(name = "crawl-1000")]
    #[serde(rename = "crawl-1000")]
    Crawl1000,
    /// The same crawl of the 10,000-page site.
    #[value(name = "crawl-10000")]
    #[serde(rename = "crawl-10000")]
    Crawl10000,
    /// The 100-page crawl with `governor.huge_pages` on (ADR 0138).
    #[value(name = "crawl-100-huge-pages")]
    #[serde(rename = "crawl-100-huge-pages")]
    Crawl100HugePages,
    /// The 1,000-page crawl with `governor.huge_pages` on.
    #[value(name = "crawl-1000-huge-pages")]
    #[serde(rename = "crawl-1000-huge-pages")]
    Crawl1000HugePages,
    /// The 10,000-page crawl with `governor.huge_pages` on.
    #[value(name = "crawl-10000-huge-pages")]
    #[serde(rename = "crawl-10000-huge-pages")]
    Crawl10000HugePages,
}

/// One crawl scenario's site and setting.
#[derive(Clone, Copy)]
pub(crate) struct CrawlRow {
    /// The pages of the site crawled.
    pub(crate) pages: u64,
    /// Whether its home's user configuration turns `governor.huge_pages`
    /// on, which only Linux honours.
    pub(crate) huge_pages: bool,
}

/// What a scenario does, which each of its figures follows.
#[derive(Clone, Copy)]
enum Work {
    IdleAttachedEngine,
    IdleDaemon,
    Crawl(CrawlRow),
}

/// The crawl scenarios' user configuration with huge pages on:
/// `governor.huge_pages` has Machine authority, so a Run's settings cannot
/// carry it (ADR 0138).
const HUGE_PAGES_CONFIGURATION: &str =
    "version = \"0.1\"\n\n[settings]\ngovernor.huge_pages = true\n";

impl Scenario {
    pub(crate) const ALL: [Self; 8] = [
        Self::IdleAttachedEngine,
        Self::IdleDaemon,
        Self::Crawl100,
        Self::Crawl1000,
        Self::Crawl10000,
        Self::Crawl100HugePages,
        Self::Crawl1000HugePages,
        Self::Crawl10000HugePages,
    ];

    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::IdleAttachedEngine => "idle-attached-engine",
            Self::IdleDaemon => "idle-daemon",
            Self::Crawl100 => "crawl-100",
            Self::Crawl1000 => "crawl-1000",
            Self::Crawl10000 => "crawl-10000",
            Self::Crawl100HugePages => "crawl-100-huge-pages",
            Self::Crawl1000HugePages => "crawl-1000-huge-pages",
            Self::Crawl10000HugePages => "crawl-10000-huge-pages",
        }
    }

    fn work(self) -> Work {
        let (pages, huge_pages) = match self {
            Self::IdleAttachedEngine => return Work::IdleAttachedEngine,
            Self::IdleDaemon => return Work::IdleDaemon,
            Self::Crawl100 => (100, false),
            Self::Crawl1000 => (1_000, false),
            Self::Crawl10000 => (10_000, false),
            Self::Crawl100HugePages => (100, true),
            Self::Crawl1000HugePages => (1_000, true),
            Self::Crawl10000HugePages => (10_000, true),
        };
        Work::Crawl(CrawlRow { pages, huge_pages })
    }

    /// The crawl it is, if it crawls the Proving ground, which `measure`
    /// serves outside the probe, so its memory is never the measured
    /// process's.
    pub(crate) fn crawl(self) -> Option<CrawlRow> {
        match self.work() {
            Work::Crawl(crawl) => Some(crawl),
            Work::IdleAttachedEngine | Work::IdleDaemon => None,
        }
    }

    /// The Products whose executables it runs on: each, for a figure of the
    /// executable itself, and Foxhound alone for a crawl, which it carries
    /// (ADR 0094).
    pub(crate) fn products(self) -> &'static [Product] {
        if self.crawl().is_some() {
            &[Product::Foxhound]
        } else {
            &[Product::Collie, Product::Foxhound]
        }
    }

    /// Its Performance target in bytes (ADR 0134), which never gates: a
    /// crawl's at every size, which `just targets` tightens to Spider's
    /// peak where that is lower (spec #2165).
    pub(crate) fn target_bytes(self) -> u64 {
        match self.work() {
            Work::IdleAttachedEngine => 8 * MIB,
            Work::IdleDaemon => 16 * MIB,
            Work::Crawl(_) => 32 * MIB,
        }
    }

    /// Whether it runs on this OS: the huge-pages rows on Linux alone,
    /// the only OS whose allocator `governor.huge_pages` changes.
    pub(crate) fn runs_here(self) -> bool {
        self.crawl()
            .is_none_or(|crawl| !crawl.huge_pages || cfg!(target_os = "linux"))
    }

    /// The arguments the executable takes for this scenario: the Daemon
    /// as `daemon start` launches it, kept alive so it never stops idle
    /// before it is stopped.
    pub(crate) fn arguments(self) -> &'static [&'static str] {
        match self.work() {
            Work::IdleAttachedEngine | Work::Crawl(_) => {
                &[goodfetch_protocol::cli::ATTACHED_ENGINE]
            }
            Work::IdleDaemon => &["daemon", "launched", "--keep-alive"],
        }
    }

    /// Writes `home`'s user configuration for this scenario: a huge-pages
    /// crawl's alone; every other home has none, as shipped.
    pub(crate) fn configure(self, home: &Path) -> Result<()> {
        if !self.crawl().is_some_and(|crawl| crawl.huge_pages) {
            return Ok(());
        }
        let version = ProductVersion::parse(env!("CARGO_PKG_VERSION"))
            .context("reading the workspace's Product version")?;
        let major = per_major(home, &version);
        std::fs::create_dir_all(&major).with_context(|| format!("creating {}", major.display()))?;
        let configuration = major.join(USER_CONFIGURATION_FILE);
        std::fs::write(&configuration, HUGE_PAGES_CONFIGURATION)
            .with_context(|| format!("writing {}", configuration.display()))
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
        match self.work() {
            Work::IdleAttachedEngine => idle_attached_engine(needs, stdin, stdout),
            Work::IdleDaemon => idle_daemon(needs, child, stdin, stdout),
            Work::Crawl(_) => {
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

/// Waits for the Daemon's runtime file, idles, then stops it over its own endpoint with
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
