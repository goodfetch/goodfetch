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

use crate::rss::crawl::{HOST, Site};

use super::foxhound::Engine;
use super::matrix::{Cell, Config, Scenario};
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
fn now() -> Instant {
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
/// the trial's own on the site's host, which the observations time every
/// other request from.
#[expect(
    clippy::disallowed_types,
    reason = "xtask is synchronous and no engine: the marker is one raw HTTP/1.1 request on its own connection, so the Proving ground records it apart from the tool's (ADR 0060 bans sockets in the engine's async crates)"
)]
fn mark(site: &Site<'_>) -> Result<()> {
    let mut stream =
        std::net::TcpStream::connect(site.address).context("connecting to the Proving ground")?;
    write!(
        stream,
        "GET {MARKER} HTTP/1.1\r\nHost: {HOST}:{}\r\nConnection: close\r\n\r\n",
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
fn request_count(address: &str) -> Result<u64> {
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
        Config::Foxhound { .. } | Config::Spider => Invocation {
            program: executable.to_owned(),
            arguments: Vec::new(),
        },
        Config::Crawlee { .. } => Invocation {
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
/// standard library outside it is not counted).
fn install_bytes(config: Config, executable: &Path) -> u64 {
    let file_bytes = |path: &Path| {
        std::fs::canonicalize(path)
            .and_then(std::fs::metadata)
            .map_or(0, |metadata| metadata.len())
    };
    let packages = match config {
        Config::Foxhound { .. } | Config::Spider => return file_bytes(executable),
        Config::Crawlee { .. } => NODE_MODULES,
        Config::Scrapy => PYTHON_ENVIRONMENT,
    };
    file_bytes(&invocation(config, executable).program)
        + footprint(&side_of(executable).join(packages))
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
fn kill(pid: u32) -> Result<()> {
    let pid = libc::pid_t::try_from(pid).context("a process ID fits a pid_t")?;
    // SAFETY: `kill` takes any process ID and signal number.
    if unsafe { libc::kill(pid, libc::SIGKILL) } != 0 {
        return Err(std::io::Error::last_os_error()).context("killing the tool");
    }
    Ok(())
}

#[cfg(not(unix))]
fn kill(_pid: u32) -> Result<()> {
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
}

impl Plan<'_> {
    /// Starts the tool's `process`-th process on `home`, Foxhound's
    /// engine opened.
    fn spawn(&self, home: &Path, process: u32, mode: &Mode) -> Result<Driven> {
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
                .arg("attached-engine")
                .env("GOODFETCH_HOME", home)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped());
        } else {
            // The other tools crawl from their arguments and print their
            // count last on stdout; a killed process's is never read.
            command
                .args(["--url", &self.site_url()])
                .args(["--host", &format!("{HOST}:{}", self.site.port())])
                .args(["--concurrency", &self.site.concurrency.to_string()])
                .current_dir(home)
                .stdin(Stdio::null())
                .stdout(if self.cell.scenario.resumes() && process == 1 {
                    Stdio::null()
                } else {
                    Stdio::piped()
                });
            if let Config::Crawlee { durable: true } = self.config {
                command.arg("--storage").arg(home);
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
            Config::Spider | Config::Crawlee { .. } | Config::Scrapy => None,
        };
        Ok(Driven { child, engine })
    }

    fn site_url(&self) -> String {
        format!("http://{}/sites/{}/1", self.site.address, self.site.pages)
    }

    fn durability(&self) -> &'static str {
        match self.config {
            Config::Foxhound { durability } => durability,
            Config::Spider | Config::Crawlee { .. } | Config::Scrapy => "",
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
    fn crawl(&self, home: &Path, sampler: Option<&Sampler>) -> Result<u32> {
        let resumes = self.cell.scenario.resumes();
        let mut driven = self.spawn(home, 1, &self.mode)?;
        if let Some(sampler) = sampler {
            sampler.follow(tool_pid(&driven.child, &self.mode));
        }
        let result = (|| -> Result<u32> {
            if let Some(engine) = driven.engine.as_mut() {
                engine.start(&self.site, self.durability())?;
                if !resumes {
                    engine.finish()?;
                    return Ok(1);
                }
            } else if !resumes {
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
        let mut second = self.spawn(home, 2, &self.mode)?;
        if let Some(sampler) = sampler {
            sampler.follow(tool_pid(&second.child, &self.mode));
        }
        // Foxhound's engine resumes the Run; another tool resumes from the
        // state it left in the same folder.
        let finished = match second.engine.as_mut() {
            Some(engine) => engine
                .resume(&self.site, self.durability())
                .and_then(|()| engine.finish()),
            None => self.read_count(&mut second.child),
        };
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
        let mut driven = self.spawn(home, 1, &Mode::Timed)?;
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

    fn observed_site(&self) -> observed::Site<'static> {
        observed::Site {
            host: HOST,
            pages: self.site.pages,
        }
    }

    /// Runs a timed trial.
    pub(crate) fn timed(&self) -> Result<Measured> {
        let home = tempfile::tempdir().context("creating the tool's data folder")?;
        mark(&self.site)?;
        let started_wall = wall();
        let started = now();
        let sampler = Sampler::start(started);
        let crawled = self.crawl(home.path(), Some(&sampler));
        let seconds = started.elapsed().as_secs_f64();
        let taken = sampler.finish();
        let processes = crawled?;
        let usage = usage()?;
        let footprint_bytes = footprint(home.path());
        let records = match self.config {
            Config::Foxhound { .. } => Some(self.records(home.path(), started_wall)?),
            Config::Spider | Config::Crawlee { .. } | Config::Scrapy => None,
        };
        let observed = observed::observe(
            &observations(self.site.address)?,
            &self.observed_site(),
            seconds,
            records.as_ref(),
        )?;
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
            install_bytes(self.config, self.executable),
            observed,
        ))
    }

    /// Runs a counted trial: its count and the work it did.
    pub(crate) fn counted(&self) -> Result<Counted> {
        let home = tempfile::tempdir().context("creating the tool's data folder")?;
        mark(&self.site)?;
        let processes = self.crawl(home.path(), None)?;
        let work = observed::observe(
            &observations(self.site.address)?,
            &self.observed_site(),
            1.0,
            None,
        )?
        .work;
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
        _ => bail!(
            "the cell `{name}` of scenario {} runs no trial",
            scenario.name()
        ),
    }
}

#[cfg(test)]
mod tests;
