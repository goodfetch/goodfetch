//! One trial, run in a process of its own (`xtask compare trial`): it marks
//! the start on the Proving ground, starts the cell's tool as its only
//! child (twice, when the scenario kills it halfway and resumes it), so the
//! OS's accounting of its waited-for children is the tool's alone, samples
//! the tool's process tree meanwhile, and once the tool exited reads the
//! Proving ground's observations, the data folder it left and, for
//! Foxhound, its Records' commit times. It prints the trial's figures as one
//! JSON line. A counted trial runs the tool under strace or Callgrind
//! instead, and prints its counts with the same work.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous: a trial runs the tool as a blocking child process and reads its files (STYLE.md)"
)]

use std::collections::HashMap;
use std::ffi::OsString;
use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::rss::crawl::Site;
use crate::test_facts::{CHROME_FOR_TESTING_BUILD, PLAYWRIGHT_REVISION};

use super::foxhound::Engine;
use super::matrix::{Cell, Config, Scenario, Site as SiteKind};
use super::observed::{self, MARKER, Observed};
use super::sampler::{self, Sampler};
use super::trials::{MILESTONES, Measured, Work};

/// How a trial runs its tool.
#[derive(Clone, Debug)]
pub(crate) enum Mode {
    /// Timed, sampled and observed.
    Timed,
    /// Under `strace -f -c`, which writes its counts to this file (and
    /// `<file>.2` for a resumed process).
    Syscalls(PathBuf),
    /// Under Callgrind, which writes its output to this file.
    Callgrind(PathBuf),
}

/// What a counted trial prints.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct Counted {
    pub(crate) count: u64,
    pub(crate) syncs: u64,
    pub(crate) work: Work,
}

/// strace's program name, and the system calls that sync a file or the
/// file system.
const STRACE: &str = "strace";
const SYNC_CALLS: [&str; 5] = ["fsync", "fdatasync", "sync_file_range", "syncfs", "sync"];
/// Valgrind's program name.
const VALGRIND: &str = "valgrind";

/// How often a resumed scenario's trial asks the Proving ground how many
/// requests it received.
const POLL: Duration = Duration::from_millis(20);

/// The moment a trial's timing starts from.
#[expect(
    clippy::disallowed_methods,
    reason = "xtask has no Clock handle (ADR 0059): a trial's seconds are measured on the OS's monotonic clock"
)]
pub(crate) fn now() -> Instant {
    Instant::now()
}

/// The wall clock, which a Record's commit time is given in.
#[expect(
    clippy::disallowed_methods,
    reason = "xtask has no Clock handle (ADR 0059): a Record's commit time is a wall-clock time"
)]
fn wall() -> jiff::Timestamp {
    jiff::Timestamp::now()
}

/// Sends the start marker to the Proving ground at `site`: one request of
/// the trial's own on `host`, the site's first page's, which the
/// observations time every other request from.
#[expect(
    clippy::disallowed_types,
    reason = "xtask is synchronous and no engine: the marker is one raw HTTP/1.1 request on its own connection, so the Proving ground records it apart from the tool's (ADR 0060 bans sockets in the engine's async crates)"
)]
fn mark(site: &Site<'_>, host: &str) -> Result<()> {
    let mut stream =
        std::net::TcpStream::connect(site.address).context("connecting to the Proving ground")?;
    write!(
        stream,
        "GET {MARKER} HTTP/1.1\r\nHost: {host}:{}\r\nConnection: close\r\n\r\n",
        site.port()
    )
    .context("sending the start marker")?;
    let mut answer = Vec::new();
    stream
        .read_to_end(&mut answer)
        .context("reading the start marker's answer")?;
    Ok(())
}

/// The requests the Proving ground at `address` received so far.
pub(crate) fn request_count(address: &str) -> Result<u64> {
    let answer: Value = ureq::get(&format!("http://{address}/request-count"))
        .call()
        .context("asking the Proving ground's request count")?
        .body_mut()
        .read_json()
        .context("parsing the Proving ground's request count")?;
    answer["requests"]
        .as_u64()
        .context("the request count holds no count")
}

/// The Proving ground's observations.
fn observations(address: &str) -> Result<Value> {
    ureq::get(&format!("http://{address}/observations"))
        .call()
        .context("reading the Proving ground's observations")?
        .body_mut()
        .with_config()
        .limit(u64::MAX)
        .read_json()
        .context("parsing the Proving ground's observations")
}

/// The program that runs a cell's tool and the arguments that come before
/// the trial's own: the executable itself, or for a tool run by an
/// interpreter, the interpreter with the tool's script.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Invocation {
    pub(crate) program: PathBuf,
    pub(crate) arguments: Vec<OsString>,
}

/// Node.js's executable, which runs Crawlee's side.
const NODE: &str = "node";

/// The Python environment `uv sync` makes beside Scrapy's side, and its
/// interpreter in it.
const PYTHON_ENVIRONMENT: &str = ".venv";
#[cfg(not(windows))]
const PYTHON: &str = "bin/python";
#[cfg(windows)]
const PYTHON: &str = "Scripts/python.exe";

/// The packages `pnpm install` leaves beside Crawlee's side.
const NODE_MODULES: &str = "node_modules";

/// The first file named `name` in a folder of `PATH`.
fn on_path(name: &str) -> Option<PathBuf> {
    std::env::split_paths(&std::env::var_os("PATH")?)
        .map(|folder| folder.join(name))
        .find(|path| path.is_file())
}

/// The folder of a side's script.
fn side_of(script: &Path) -> &Path {
    script.parent().unwrap_or_else(|| Path::new("."))
}

/// How `config`'s tool runs `executable`: Foxhound's and Spider's are
/// executables, Crawlee's is its side's script run by Node.js from `PATH`,
/// and Scrapy's its side's script run by the interpreter of the Python
/// environment beside it.
pub(crate) fn invocation(config: Config, executable: &Path) -> Invocation {
    match config {
        Config::Foxhound { .. } | Config::Spider { .. } => Invocation {
            program: executable.to_owned(),
            arguments: Vec::new(),
        },
        Config::Crawlee => Invocation {
            program: on_path(NODE).unwrap_or_else(|| PathBuf::from(NODE)),
            arguments: vec![executable.as_os_str().to_owned()],
        },
        Config::Scrapy => Invocation {
            program: side_of(executable).join(PYTHON_ENVIRONMENT).join(PYTHON),
            arguments: vec![executable.as_os_str().to_owned()],
        },
    }
}

/// What a tool takes on disk: its executable, and for a tool run by an
/// interpreter, the interpreter's executable and the packages its side
/// installed (`node_modules`, or the Python environment; the Python
/// standard library outside it is not counted); with `browser`, the
/// browser's folder too, the same for every tool.
fn install_bytes(config: Config, executable: &Path, browser: Option<&Path>) -> u64 {
    let file_bytes = |path: &Path| {
        std::fs::canonicalize(path)
            .and_then(std::fs::metadata)
            .map_or(0, |metadata| metadata.len())
    };
    let browser_bytes = browser.map_or(0, footprint);
    let packages = match config {
        Config::Foxhound { .. } | Config::Spider { .. } => {
            return file_bytes(executable) + browser_bytes;
        }
        Config::Crawlee => NODE_MODULES,
        Config::Scrapy => PYTHON_ENVIRONMENT,
    };
    file_bytes(&invocation(config, executable).program)
        + footprint(&side_of(executable).join(packages))
        + browser_bytes
}

// Every tool of the headless browser scenario runs one browser, the Chrome
// for Testing build Collie pins (`CHROME_FOR_TESTING_BUILD`), from the
// folder Playwright installs it in (`PLAYWRIGHT_REVISION`); test support's
// `browsers.rs` lays out the same build for the tests.
/// Chrome for Testing's folder and executable in it, on this platform.
#[cfg(target_os = "macos")]
const CHROME_FOLDER: &str = if cfg!(target_arch = "aarch64") {
    "chrome-mac-arm64"
} else {
    "chrome-mac-x64"
};
#[cfg(target_os = "macos")]
const CHROME: &str = "Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing";
#[cfg(not(target_os = "macos"))]
const CHROME_FOLDER: &str = "chrome-linux64";
#[cfg(not(target_os = "macos"))]
const CHROME: &str = "chrome";

/// The folder of the Chrome for Testing build Playwright installed
/// (`PLAYWRIGHT_BROWSERS_PATH`, else its default folder), which the
/// headless browser scenario runs; none when it is not installed.
pub(crate) fn chrome_folder() -> Option<PathBuf> {
    let browsers = if let Some(path) = std::env::var_os("PLAYWRIGHT_BROWSERS_PATH") {
        PathBuf::from(path)
    } else {
        let home = PathBuf::from(std::env::var_os("HOME")?);
        if cfg!(target_os = "macos") {
            home.join("Library/Caches/ms-playwright")
        } else {
            home.join(".cache/ms-playwright")
        }
    };
    let folder = browsers
        .join(format!("chromium-{PLAYWRIGHT_REVISION}"))
        .join(CHROME_FOLDER);
    folder.join(CHROME).is_file().then_some(folder)
}

/// Lays out, in a folder of its own, a Browser cache holding `chrome`'s
/// files as Foxhound's pinned build, as a prepared `GOODFETCH_BROWSERS_PATH`
/// holds it: hard links where the folders share a file system, copies
/// otherwise.
fn browser_cache(chrome: &Path) -> Result<tempfile::TempDir> {
    let root = tempfile::tempdir().context("creating the Browser cache's folder")?;
    let entry = root
        .path()
        .join("browser")
        .join("chrome-for-testing")
        .join(CHROME_FOR_TESTING_BUILD);
    link_tree(chrome, &entry.join("files").join(CHROME_FOLDER))
        .context("laying out the Browser cache")?;
    std::fs::write(entry.join("lock"), "").context("writing the Browser cache's lock")?;
    let installed = serde_json::json!({
        "format_version": "0.1",
        "artifact": {
            "kind": "browser",
            "name": "chrome-for-testing",
            "build": CHROME_FOR_TESTING_BUILD,
        },
        "digest": "sha256:0000000000000000000000000000000000000000000000000000000000000000",
        "installed_at": "2026-01-01T00:00:00Z",
        "used_at": "2026-01-01T00:00:00Z",
    });
    std::fs::write(entry.join("entry.json"), installed.to_string())
        .context("writing the Browser cache's entry")?;
    Ok(root)
}

/// `from`'s files under `to`, each a hard link, or a copy across file
/// systems; symbolic links kept as such.
fn link_tree(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let (source, target) = (entry.path(), to.join(entry.file_name()));
        let kind = entry.file_type()?;
        if kind.is_dir() {
            link_tree(&source, &target)?;
        } else if kind.is_symlink() {
            #[cfg(unix)]
            std::os::unix::fs::symlink(std::fs::read_link(&source)?, &target)?;
        } else if std::fs::hard_link(&source, &target).is_err() {
            std::fs::copy(&source, &target)?;
        }
    }
    Ok(())
}

/// The command that runs `invocation`, under `mode`'s counting tool; the
/// `process`-th process of a resumed trial writes strace's counts to a file
/// of its own.
fn command(invocation: &Invocation, mode: &Mode, process: u32) -> Command {
    let mut command = match mode {
        Mode::Timed => Command::new(&invocation.program),
        Mode::Syscalls(output) => {
            let mut strace = Command::new(STRACE);
            let mut file = output.clone().into_os_string();
            if process > 1 {
                file.push(format!(".{process}"));
            }
            strace
                .args(["-f", "-c", "-U", "calls,name", "-o"])
                .arg(file)
                .arg("--")
                .arg(&invocation.program);
            strace
        }
        Mode::Callgrind(output) => {
            let mut valgrind = Command::new(VALGRIND);
            valgrind
                .args(["--tool=callgrind", "--quiet"])
                .arg(format!("--callgrind-out-file={}", output.display()))
                .arg(&invocation.program);
            valgrind
        }
    };
    command.args(&invocation.arguments);
    command
}

/// The tool's own process: the child, or under strace the process strace
/// started.
fn tool_pid(child: &Child, mode: &Mode) -> u32 {
    match mode {
        Mode::Syscalls(_) => {
            // strace starts the tool as soon as it runs; wait for it.
            for _ in 0..500 {
                if let Some(&pid) = sampler::tree(child.id()).get(1) {
                    return pid;
                }
                std::thread::sleep(Duration::from_millis(2));
            }
            child.id()
        }
        Mode::Timed | Mode::Callgrind(_) => child.id(),
    }
}

/// Kills the process `pid` with `SIGKILL`.
#[cfg(unix)]
#[expect(
    unsafe_code,
    reason = "a process other than the child is killed through the OS's C interface: kill"
)]
pub(crate) fn kill(pid: u32) -> Result<()> {
    let pid = libc::pid_t::try_from(pid).context("a process ID fits a pid_t")?;
    // SAFETY: `kill` takes any process ID and signal number.
    if unsafe { libc::kill(pid, libc::SIGKILL) } != 0 {
        return Err(std::io::Error::last_os_error()).context("killing the tool");
    }
    Ok(())
}

#[cfg(not(unix))]
pub(crate) fn kill(_pid: u32) -> Result<()> {
    bail!("a resumed scenario kills its tool with SIGKILL, which only Unix has")
}

/// What the OS accounted to the trial's waited-for children.
#[derive(Clone, Copy, Default)]
struct Usage {
    user_seconds: f64,
    system_seconds: f64,
    peak_bytes: u64,
    read_bytes: u64,
    written_bytes: u64,
}

#[cfg(unix)]
#[expect(
    unsafe_code,
    reason = "the OS's accounting of the waited-for children is read through its C interface: getrusage"
)]
fn usage() -> Result<Usage> {
    // SAFETY: `rusage` is plain data, for which all zeroes is a valid value.
    let mut usage: libc::rusage = unsafe { std::mem::zeroed() };
    // SAFETY: `usage` is a valid, writable `rusage` that outlives the call.
    if unsafe { libc::getrusage(libc::RUSAGE_CHILDREN, &raw mut usage) } != 0 {
        return Err(std::io::Error::last_os_error()).context("reading getrusage");
    }
    let seconds = |time: libc::timeval| {
        #[expect(
            clippy::cast_precision_loss,
            reason = "seconds and microseconds of CPU time stay far below 2^52"
        )]
        let seconds = time.tv_sec as f64 + time.tv_usec as f64 / 1e6;
        seconds
    };
    let count = |value: libc::c_long| u64::try_from(value).unwrap_or(0);
    let maximum = count(usage.ru_maxrss);
    Ok(Usage {
        user_seconds: seconds(usage.ru_utime),
        system_seconds: seconds(usage.ru_stime),
        // macOS counts the maximum resident set in bytes, Linux in KiB.
        peak_bytes: if cfg!(target_os = "macos") {
            maximum
        } else {
            maximum * 1024
        },
        // Blocks of 512 bytes, as Linux counts its I/O accounting's bytes.
        read_bytes: count(usage.ru_inblock) * 512,
        written_bytes: count(usage.ru_oublock) * 512,
    })
}

#[cfg(not(unix))]
fn usage() -> Result<Usage> {
    Ok(Usage::default())
}

/// What `folder` holds on disk: the blocks its files take.
fn footprint(folder: &Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return 0;
    };
    entries
        .flatten()
        .map(|entry| match entry.metadata() {
            Ok(metadata) if metadata.is_dir() => footprint(&entry.path()),
            Ok(metadata) => on_disk(&metadata),
            Err(_) => 0,
        })
        .sum()
}

#[cfg(unix)]
fn on_disk(metadata: &std::fs::Metadata) -> u64 {
    use std::os::unix::fs::MetadataExt as _;
    metadata.blocks() * 512
}

#[cfg(not(unix))]
fn on_disk(metadata: &std::fs::Metadata) -> u64 {
    metadata.len()
}

/// Starts one of the tool's processes and drives it to the crawl's end, or,
/// with `kill_at`, to that many requests received, when it is killed.
/// `resume` drives a process that resumes the killed crawl.
struct Driven {
    child: Child,
    engine: Option<Engine>,
}

/// Everything one trial needs.
pub(crate) struct Plan<'a> {
    pub(crate) cell: &'a Cell,
    pub(crate) config: Config,
    pub(crate) executable: &'a Path,
    pub(crate) site: Site<'a>,
    pub(crate) mode: Mode,
    pub(crate) goodfetch_variables: &'a [OsString],
    /// Chrome for Testing's folder, which the headless browser scenario's
    /// tools run.
    pub(crate) browser: Option<&'a Path>,
}

impl Plan<'_> {
    /// The site the cell's scenario crawls.
    fn kind(&self) -> SiteKind {
        self.cell.scenario.site()
    }

    /// The browser the cell's tool runs, in the headless browser scenario.
    fn browser(&self) -> Result<Option<&Path>> {
        if self.cell.scenario != Scenario::Browser {
            return Ok(None);
        }
        self.browser
            .map(Some)
            .context("the headless browser scenario takes Chrome for Testing's folder, --browser")
    }

    /// Starts the tool's `process`-th process on `home`, Foxhound's
    /// engine opened, with its Browser cache at `cache`, if any.
    fn spawn(
        &self,
        home: &Path,
        process: u32,
        mode: &Mode,
        cache: Option<&Path>,
    ) -> Result<Driven> {
        let mut command = command(&invocation(self.config, self.executable), mode, process);
        command.stderr(Stdio::inherit());
        // No tool reads Goodfetch's variables a shell may hold, such as a
        // test's: Foxhound refuses one it does not deploy with.
        for name in self.goodfetch_variables {
            command.env_remove(name);
        }
        if let Config::Foxhound { .. } = self.config {
            // The executable as shipped, with its own data folder and its
            // default logging, which it writes to stderr.
            command
                .arg(goodfetch_protocol::cli::ATTACHED_ENGINE)
                .env(goodfetch_protocol::settings::HOME_VARIABLE, home)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped());
            if let Some(cache) = cache {
                command.env(goodfetch_protocol::settings::BROWSERS_PATH_VARIABLE, cache);
            }
        } else {
            // The other tools crawl from their arguments, in their data
            // folder, and print their count last on stdout.
            command
                .args(["--url", &self.site_url()])
                .args(["--concurrency", &self.site.concurrency.to_string()])
                .args(["--scenario", self.cell.scenario.name()])
                .current_dir(home)
                .stdin(Stdio::null())
                .stdout(Stdio::piped());
            if !self.kind().by_name() {
                command.args([
                    "--host",
                    &format!("{}:{}", self.kind().host_of(1), self.site.port()),
                ]);
            }
            if let Some(browser) = self.browser()? {
                let chrome = browser.join(CHROME);
                match self.config {
                    // Spider's `chrome` feature finds its browser there.
                    Config::Spider { .. } => {
                        command.env("CHROME_BIN", chrome);
                    }
                    _ => {
                        command.arg("--browser").arg(chrome);
                    }
                }
            }
        }
        let mut child = command.spawn().context("starting the tool")?;
        let engine = match self.config {
            Config::Foxhound { .. } => {
                let stdin = child.stdin.take().context("the engine's stdin is piped")?;
                let stdout = child
                    .stdout
                    .take()
                    .context("the engine's stdout is piped")?;
                match Engine::open(stdin, stdout) {
                    Ok(engine) => Some(engine),
                    Err(error) => {
                        let _ = child.kill();
                        let _ = child.wait();
                        return Err(error);
                    }
                }
            }
            Config::Spider { .. } | Config::Crawlee | Config::Scrapy => None,
        };
        Ok(Driven { child, engine })
    }

    /// The site's first page: on the Proving ground's address for a site a
    /// tool reaches with a `Host` header, else on its host's name.
    fn site_url(&self) -> String {
        if self.kind().by_name() {
            self.start_url()
        } else {
            format!("http://{}/sites/{}/1", self.site.address, self.site.pages)
        }
    }

    /// The site's first page on its host's name.
    fn start_url(&self) -> String {
        format!(
            "http://{}:{}/sites/{}/1",
            self.kind().host_of(1),
            self.site.port(),
            self.site.pages
        )
    }

    /// Foxhound's `run_state.durability` and, at `periodic`, its
    /// `run_state.commit_interval` in milliseconds.
    fn durability(&self) -> (&'static str, Option<u64>) {
        match self.config {
            Config::Foxhound {
                durability,
                commit_interval,
            } => (durability, commit_interval),
            Config::Spider { .. } | Config::Crawlee | Config::Scrapy => ("", None),
        }
    }

    /// Reads a tool's output to its end: its last line is its count.
    fn read_count(&self, child: &mut Child) -> Result<()> {
        let mut text = String::new();
        child
            .stdout
            .take()
            .context("the tool's stdout is piped")?
            .read_to_string(&mut text)
            .context("reading the tool's output")?;
        if !text
            .lines()
            .last()
            .is_some_and(|line| line.starts_with("pages="))
        {
            bail!("{} printed no count: {}", self.cell.name, text.trim_end());
        }
        Ok(())
    }

    /// Runs the tool through the crawl, killed halfway and resumed when
    /// the scenario says so, and returns its processes, each waited for.
    fn crawl(&self, home: &Path, sampler: Option<&Sampler>, cache: Option<&Path>) -> Result<u32> {
        let resumes = self.cell.scenario.resumes();
        let mut driven = self.spawn(home, 1, &self.mode, cache)?;
        if let Some(sampler) = sampler {
            sampler.follow(tool_pid(&driven.child, &self.mode));
        }
        let result = (|| -> Result<u32> {
            if let Some(engine) = driven.engine.as_mut() {
                engine.start(
                    &self.site,
                    self.cell.scenario,
                    &self.start_url(),
                    self.durability(),
                )?;
                if !resumes {
                    engine.finish()?;
                    return Ok(1);
                }
            } else {
                if resumes {
                    bail!("{} cannot resume a crawl", self.cell.name);
                }
                self.read_count(&mut driven.child)?;
                return Ok(1);
            }
            // Killed once half the site's pages were asked for, then
            // resumed by a new process on the same home.
            while request_count(self.site.address)? < self.site.pages / 2 {
                std::thread::sleep(POLL);
            }
            kill(tool_pid(&driven.child, &self.mode))?;
            Ok(2)
        })();
        let processes = match result {
            Ok(processes) => processes,
            Err(error) => {
                let _ = driven.child.kill();
                let _ = driven.child.wait();
                return Err(error);
            }
        };
        if let Some(engine) = driven.engine.take() {
            engine.close();
        }
        let status = driven.child.wait().context("waiting for the tool")?;
        if processes == 1 {
            if !status.success() {
                bail!("the tool exited with {status}");
            }
            return Ok(1);
        }
        let mut second = self.spawn(home, 2, &self.mode, cache)?;
        if let Some(sampler) = sampler {
            sampler.follow(tool_pid(&second.child, &self.mode));
        }
        let engine = second
            .engine
            .as_mut()
            .context("a resumed tool is Foxhound's engine")?;
        let finished = engine
            .resume(&self.site, self.cell.scenario, self.durability())
            .and_then(|()| engine.finish());
        if let Err(error) = finished {
            let _ = second.child.kill();
            let _ = second.child.wait();
            return Err(error);
        }
        if let Some(engine) = second.engine.take() {
            engine.close();
        }
        let status = second.child.wait().context("waiting for the tool")?;
        if !status.success() {
            bail!("the resumed tool exited with {status}");
        }
        Ok(2)
    }

    /// Foxhound's Records' commit times, in seconds from `started`, by
    /// path, read by an engine of their own on `home` once the measured
    /// processes are gone.
    fn records(&self, home: &Path, started: jiff::Timestamp) -> Result<HashMap<String, f64>> {
        let mut driven = self.spawn(home, 1, &Mode::Timed, None)?;
        let engine = driven.engine.as_mut().context("Foxhound's engine")?;
        let read = engine.records(&self.site);
        if let Some(engine) = driven.engine.take() {
            engine.close();
        }
        let _ = driven.child.wait();
        let mut times = HashMap::new();
        for (url, committed) in read? {
            let committed: jiff::Timestamp = committed
                .parse()
                .with_context(|| format!("a Record's commit time: {committed}"))?;
            let seconds = committed.duration_since(started).as_secs_f64();
            let path = url
                .split_once("://")
                .and_then(|(_, rest)| rest.find('/').map(|slash| rest[slash..].to_owned()))
                .unwrap_or(url);
            times
                .entry(path)
                .and_modify(|first: &mut f64| *first = first.min(seconds))
                .or_insert(seconds);
        }
        Ok(times)
    }

    /// What the Proving ground observed of the trial on the site's hosts.
    fn observe(&self, seconds: f64, records: Option<&HashMap<String, f64>>) -> Result<Observed> {
        let hosts = self.kind().hosts();
        let hosts: Vec<&str> = hosts.iter().map(String::as_str).collect();
        observed::observe(
            &observations(self.site.address)?,
            &observed::Site {
                hosts: &hosts,
                pages: self.site.pages,
            },
            seconds,
            records,
        )
    }

    /// Foxhound's Browser cache, laid out for its trial of the headless
    /// browser scenario.
    fn cache(&self) -> Result<Option<tempfile::TempDir>> {
        match (self.config, self.browser()?) {
            (Config::Foxhound { .. }, Some(browser)) => Ok(Some(browser_cache(browser)?)),
            _ => Ok(None),
        }
    }

    /// Runs a timed trial.
    pub(crate) fn timed(&self) -> Result<Measured> {
        let home = tempfile::tempdir().context("creating the tool's data folder")?;
        let cache = self.cache()?;
        mark(&self.site, &self.kind().host_of(1))?;
        let started_wall = wall();
        let started = now();
        let sampler = Sampler::start(started);
        let crawled = self.crawl(
            home.path(),
            Some(&sampler),
            cache.as_ref().map(tempfile::TempDir::path),
        );
        let seconds = started.elapsed().as_secs_f64();
        let taken = sampler.finish();
        let processes = crawled?;
        let usage = usage()?;
        let footprint_bytes = footprint(home.path());
        let records = match self.config {
            Config::Foxhound { .. } if super::foxhound::keeps_records(self.cell.scenario) => {
                Some(self.records(home.path(), started_wall)?)
            }
            Config::Foxhound { .. } | Config::Spider { .. } | Config::Crawlee | Config::Scrapy => {
                None
            }
        };
        let observed = self.observe(seconds, records.as_ref())?;
        let milestones: Vec<(u64, f64)> = MILESTONES
            .iter()
            .filter_map(|&pages| {
                let index = usize::try_from(pages).ok()?.checked_sub(1)?;
                observed.page_times.get(index).map(|&at| (pages, at))
            })
            .collect();
        let tree = taken.map(|samples| sampler::figures(&samples, &milestones));
        let peak_bytes = usage.peak_bytes;
        Ok(measured(
            seconds,
            processes,
            usage,
            peak_bytes,
            tree,
            footprint_bytes,
            install_bytes(self.config, self.executable, self.browser()?),
            observed,
        ))
    }

    /// Runs a counted trial: its count and the work it did.
    pub(crate) fn counted(&self) -> Result<Counted> {
        let home = tempfile::tempdir().context("creating the tool's data folder")?;
        let cache = self.cache()?;
        mark(&self.site, &self.kind().host_of(1))?;
        let processes = self.crawl(
            home.path(),
            None,
            cache.as_ref().map(tempfile::TempDir::path),
        )?;
        let work = self.observe(1.0, None)?.work;
        let (count, syncs) = match &self.mode {
            Mode::Syscalls(output) => {
                let (mut count, mut syncs) = (0, 0);
                for process in 1..=processes {
                    let mut file = output.clone().into_os_string();
                    if process > 1 {
                        file.push(format!(".{process}"));
                    }
                    let file = PathBuf::from(file);
                    let text = std::fs::read_to_string(&file)
                        .with_context(|| format!("reading {}", file.display()))?;
                    let (calls, synced) = system_calls_of(&text)?;
                    count += calls;
                    syncs += synced;
                }
                (count, syncs)
            }
            Mode::Callgrind(output) => {
                let text = std::fs::read_to_string(output)
                    .with_context(|| format!("reading {}", output.display()))?;
                (instructions_of(&text)?, 0)
            }
            Mode::Timed => bail!("a timed trial counts nothing"),
        };
        Ok(Counted { count, syncs, work })
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "one trial's figures, from as many sources"
)]
fn measured(
    seconds: f64,
    processes: u32,
    usage: Usage,
    peak_bytes: u64,
    sampled: Option<super::trials::Sampled>,
    footprint_bytes: u64,
    executable_bytes: u64,
    observed: Observed,
) -> Measured {
    Measured {
        seconds,
        processes,
        user_seconds: usage.user_seconds,
        system_seconds: usage.system_seconds,
        peak_bytes,
        sampled,
        read_bytes: usage.read_bytes,
        written_bytes: usage.written_bytes,
        footprint_bytes,
        executable_bytes,
        work: observed.work,
        requests_sent: observed.requests_sent,
        duplicates: observed.duplicates,
        errors: observed.errors,
        received_bytes: observed.received_bytes,
        sent_bytes: observed.sent_bytes,
        connections: observed.connections,
        tls_connections: observed.tls_connections,
        startup_seconds: observed.startup_seconds,
        first_page_seconds: observed.first_page_seconds,
        first_record_seconds: observed.first_record_seconds,
        curve: observed.curve,
        latency: observed.latency,
        latency_end: observed.latency_end,
    }
}

/// Why this host cannot run a counted cell under `program`, when it
/// cannot.
fn missing(program: &str) -> Option<String> {
    let installed = Command::new(program)
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success());
    (!installed).then(|| format!("{program} is not installed on this host"))
}

/// Why this host cannot run the Callgrind cell, when it cannot.
pub(crate) fn callgrind_missing() -> Option<String> {
    missing(VALGRIND)
}

/// Why this host cannot run the system-call cell, when it cannot.
pub(crate) fn strace_missing() -> Option<String> {
    missing(STRACE)
}

/// The instructions a Callgrind output file totals: its `totals:` line, of
/// the one event it counted.
pub(crate) fn instructions_of(output: &str) -> Result<u64> {
    output
        .lines()
        .find_map(|line| line.strip_prefix("totals:"))
        .context("Callgrind's output holds no `totals:` line")?
        .trim()
        .parse()
        .context("parsing Callgrind's total")
}

/// The system calls `strace -c -U calls,name` counted, and those of them
/// that sync: its `total` row, and the sum of its [`SYNC_CALLS`] rows.
pub(crate) fn system_calls_of(output: &str) -> Result<(u64, u64)> {
    let mut total = None;
    let mut syncs = 0;
    for line in output.lines() {
        let mut words = line.split_whitespace();
        let (Some(calls), Some(name), None) = (words.next(), words.next(), words.next()) else {
            continue;
        };
        let Ok(calls) = calls.parse::<u64>() else {
            continue;
        };
        if name == "total" {
            total = Some(calls);
        } else if SYNC_CALLS.contains(&name) {
            syncs += calls;
        }
    }
    Ok((total.context("strace's counts hold no `total` row")?, syncs))
}

/// Parses the scenario and cell a `trial` command names.
pub(crate) fn cell_of(scenario: Scenario, name: &str) -> Result<(&'static Cell, Config)> {
    let cell = super::matrix::cell(scenario, name)
        .with_context(|| format!("no cell `{name}` in scenario {}", scenario.name()))?;
    match cell.status {
        super::matrix::Status::Measured(config) => Ok((cell, config)),
        super::matrix::Status::NotSupported(_) => bail!(
            "the cell `{name}` of scenario {} runs no trial",
            scenario.name()
        ),
    }
}

#[cfg(test)]
mod tests;
