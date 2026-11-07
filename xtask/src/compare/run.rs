//! `run`: every measured cell's declared trials in one random order, each
//! against a fresh Proving ground, then the system-call cell's, the growth
//! cell's (when asked for) and the Callgrind cell's runs in the same way,
//! written to the trials file.

use std::io::{Read as _, Seek as _, SeekFrom};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use anyhow::{Context as _, Result, bail};

use crate::output;
use crate::rss::crawl::serve;

use super::matrix::{self, Cell, Config, Scenario};
use super::sampler;
use super::trial::{self, Counted, callgrind_missing, strace_missing};
use super::trials::{
    CONCURRENCY, CRAWLEE_VERSION, Comparison, CountedCell, CountedRun, Declared, GROWTH_PAGES,
    GrowthCell, INSTRUCTION_RUNS, INSTRUCTION_TOOLS, Machine, Measured, PLAYWRIGHT_VERSION,
    QUIET_LOAD, QUIET_WAIT_SECONDS, SCRAPY_PLAYWRIGHT_VERSION, SCRAPY_VERSION, SITE_PAGES,
    SPIDER_VERSION, STATISTIC, StoppedRun, Trial,
};

/// The executables a run takes.
pub(crate) struct Executables<'a> {
    pub(crate) proving_ground: &'a Path,
    pub(crate) foxhound: &'a Path,
    pub(crate) spider: &'a Path,
    /// Spider's side built with its `chrome` feature, for the headless
    /// browser scenario.
    pub(crate) spider_chrome: &'a Path,
    /// Crawlee's side's script, run by Node.js, and Scrapy's, run by the
    /// Python environment beside it.
    pub(crate) crawlee: &'a Path,
    pub(crate) scrapy: &'a Path,
    /// Chrome for Testing's folder, which every tool of the headless
    /// browser scenario runs.
    pub(crate) browser: &'a Path,
}

/// What a run was asked for.
pub(crate) struct Options {
    /// Trials of each measured cell.
    pub(crate) trials: u32,
    /// Whether the growth cell runs.
    pub(crate) growth: bool,
    /// Gives Spider the site of this many pages instead, which the
    /// same-work check refuses: the check shown failing.
    pub(crate) spider_pages: Option<u64>,
    /// The scenarios whose cells run.
    pub(crate) scenarios: Vec<Scenario>,
}

/// Whether each timed trial waits for a quiet machine: only in a run of the
/// matrix's full [`super::trials::TRIALS`], so a quicker try, which its
/// report flags as short, never sleeps.
pub(crate) fn waits_for_quiet(options: &Options) -> bool {
    options.trials >= super::trials::TRIALS
}

/// The measured cells of the scenarios a run asked for.
fn measured(options: &Options) -> Vec<(&'static Cell, Config)> {
    matrix::measured()
        .filter(|(cell, _)| options.scenarios.contains(&cell.scenario))
        .collect()
}

/// `SplitMix64`, for the trials' order: nothing here needs more.
fn next(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

/// `runs` runs of each cell of `cells`, shuffled by `state` (Fisher–Yates).
fn order(
    cells: &[(&'static Cell, Config)],
    runs: u32,
    state: &mut u64,
) -> Vec<(&'static Cell, Config, u32)> {
    let mut order: Vec<_> = cells
        .iter()
        .flat_map(|&(cell, config)| (1..=runs).map(move |number| (cell, config, number)))
        .collect();
    for last in (1..order.len()).rev() {
        let pick = usize::try_from(next(state) % (last as u64 + 1))
            .expect("invariant: an index below the trial count fits a usize");
        order.swap(last, pick);
    }
    order
}

/// The machine: OS and version, CPU, threads, and its load now.
#[expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous: it asks the OS's own commands and files with blocking calls (STYLE.md)"
)]
fn machine() -> Machine {
    let read = |program: &str, arguments: &[&str]| {
        Command::new(program)
            .args(arguments)
            .output()
            .ok()
            .filter(|output| output.status.success())
            .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned())
            .unwrap_or_default()
    };
    let (os, cpu) = if cfg!(target_os = "macos") {
        (
            format!("macOS {}", read("sw_vers", &["-productVersion"])),
            read("sysctl", &["-n", "machdep.cpu.brand_string"]),
        )
    } else {
        let cpu = std::fs::read_to_string("/proc/cpuinfo")
            .ok()
            .and_then(|text| {
                text.lines()
                    .find_map(|line| line.strip_prefix("model name"))
                    .map(|rest| rest.trim_start_matches([' ', '\t', ':']).to_owned())
            })
            .unwrap_or_default();
        (
            format!("{} {}", std::env::consts::OS, read("uname", &["-r"])),
            cpu,
        )
    };
    let load = load();
    Machine {
        class: "local-controlled".to_owned(),
        os,
        arch: std::env::consts::ARCH.to_owned(),
        cpu,
        threads: std::thread::available_parallelism().map_or(0, std::num::NonZero::get),
        load,
    }
}

/// The 1-minute load average now, where the OS gives one.
#[expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous: it reads /proc with a blocking call (STYLE.md)"
)]
fn load() -> Option<f64> {
    std::fs::read_to_string("/proc/loadavg")
        .ok()
        .and_then(|text| text.split_whitespace().next()?.parse().ok())
}

/// Waits, up to [`QUIET_WAIT_SECONDS`], for the 1-minute load to fall under
/// [`QUIET_LOAD`], so a timed trial starts on a quiet machine, a browser
/// trial's own load included; the load it starts at, where the OS gives
/// one.
#[expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous: a run waits for a quiet machine by sleeping between reads of its load (STYLE.md)"
)]
fn quiet() -> Option<f64> {
    const POLL_SECONDS: u64 = 10;
    let mut waited = 0;
    loop {
        let now = load()?;
        if now < QUIET_LOAD || waited >= QUIET_WAIT_SECONDS {
            if waited > 0 {
                output::line(&format!(
                    "Waited {waited} s for the 1-minute load to fall under {QUIET_LOAD}: {now:.2}."
                ));
            }
            return Some(now);
        }
        std::thread::sleep(std::time::Duration::from_secs(POLL_SECONDS));
        waited += POLL_SECONDS;
    }
}

/// The seed of the trials' order, drawn from the wall clock: any seed will
/// do, and the one drawn is recorded with the trials.
#[expect(
    clippy::disallowed_methods,
    reason = "xtask has no Clock or Entropy handle (ADR 0059): a one-off comparison draws its recorded order seed from the wall clock"
)]
fn drawn_seed() -> u64 {
    let now = jiff::Timestamp::now();
    now.as_second().unsigned_abs() ^ u64::from(now.subsec_nanosecond().unsigned_abs()) << 32
}

/// One trial of `cell` against a fresh Proving ground, through `xtask
/// compare trial`, which prints its figures as one JSON line. `mode` and
/// `output` name a counted trial's tool and file; `watch`, what the
/// comparison stops once [`stopped`] says why.
#[expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous: a trial runs as a blocking child process (STYLE.md)"
)]
fn trial<T: serde::de::DeserializeOwned>(
    executables: &Executables<'_>,
    cell: &Cell,
    config: Config,
    pages: u64,
    mode: (&str, Option<&Path>),
    watch: Watch,
) -> Result<Outcome<T>> {
    let executable = match config {
        Config::Foxhound { .. } => executables.foxhound,
        Config::Spider { chrome: false } => executables.spider,
        Config::Spider { chrome: true } => executables.spider_chrome,
        Config::Crawlee => executables.crawlee,
        Config::Scrapy => executables.scrapy,
    };
    let serving = serve(executables.proving_ground)?;
    let probe = std::env::current_exe().context("finding xtask's own executable")?;
    let mut command = Command::new(probe);
    command
        .args(["compare", "trial", "--scenario", cell.scenario.name()])
        .args(["--cell", cell.name, "--executable"])
        .arg(executable)
        .args(["--address", &serving.address, "--pages", &pages.to_string()])
        .args(["--concurrency", &CONCURRENCY.to_string(), "--mode", mode.0])
        .arg("--browser")
        .arg(executables.browser);
    if let Some(output) = mode.1 {
        command.arg("--output").arg(output);
    }
    // A file rather than a pipe: a watched trial is waited for by polling,
    // with no reader of a pipe beside it.
    let mut printed = tempfile::tempfile().context("creating the trial's output file")?;
    let mut child = command
        .stdout(
            printed
                .try_clone()
                .context("sharing the trial's output file")?,
        )
        .stderr(Stdio::inherit())
        .spawn()
        .context("running xtask compare trial")?;
    if let Some(reason) = watched(&mut child, &serving.address, watch)? {
        return Ok(Outcome::Stopped(reason));
    }
    let status = child.wait().context("waiting for xtask compare trial")?;
    drop(serving);
    if !status.success() {
        bail!(
            "{}'s trial in scenario {} failed",
            cell.name,
            cell.scenario.name()
        );
    }
    let mut figures = Vec::new();
    printed
        .seek(SeekFrom::Start(0))
        .and_then(|_| printed.read_to_end(&mut figures))
        .context("reading the trial's figures")?;
    serde_json::from_slice(&figures)
        .map(Outcome::Done)
        .with_context(|| {
            format!(
                "parsing the trial's figures: {}",
                String::from_utf8_lossy(&figures)
            )
        })
}

/// A trial's figures, or why the comparison stopped it.
pub(crate) enum Outcome<T> {
    Done(T),
    Stopped(String),
}

/// How long a watched trial may go with the Proving ground receiving no
/// request before it is stopped as stalled.
pub(crate) const STALL_SECONDS: u64 = 120;
/// The longest a counted run may take.
pub(crate) const COUNTED_BOUND_SECONDS: u64 = 1800;
/// The longest a timed trial of the [`SITE_PAGES`]-page site may take: a
/// browser trial took at most about 260 s on a loaded machine (#1903).
pub(crate) const TIMED_BOUND_SECONDS: u64 = 600;

/// Which trial the comparison watches, stopping it once its crawl stalls
/// or reaches its bound.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Watch {
    /// Not watched: a Callgrind run, whose slowdown would need another
    /// bound, and whose scenario 1 crawls finish.
    None,
    /// The `n`-th run of a cell under strace.
    Counted(u32),
    /// The `n`-th timed trial of a cell, of a site of so many pages.
    Timed(u32, u64),
}

/// Why the watched trial, `seconds` into its crawl and `quiet` seconds
/// since the Proving ground's count of requests last grew, at `requests`,
/// is stopped; none while it goes on.
pub(crate) fn stopped(watch: Watch, seconds: u64, quiet: u64, requests: u64) -> Option<String> {
    let (named, what, bound) = match watch {
        Watch::None => return None,
        Watch::Counted(number) => (
            format!("under `strace -f`, run {number}"),
            "a counted run",
            COUNTED_BOUND_SECONDS,
        ),
        Watch::Timed(number, pages) => (
            format!("timed trial {number}"),
            "a timed trial",
            TIMED_BOUND_SECONDS * pages.div_ceil(SITE_PAGES).max(1),
        ),
    };
    if quiet >= STALL_SECONDS {
        return Some(format!(
            "{named} stopped after {seconds} s: the Proving ground received no request for {quiet} s after {requests} requests"
        ));
    }
    (seconds >= bound).then(|| {
        format!(
            "{named} stopped after {seconds} s, the longest {what} may take, at {requests} requests"
        )
    })
}

/// A timed trial once run again when stopped: [`retried`].
pub(crate) enum Retried<T> {
    /// Its figures, and why its first attempt stopped when they are its
    /// retry's.
    Done {
        measured: T,
        retried: Option<String>,
    },
    /// Stopped twice: both reasons.
    Stopped(String),
}

/// Runs `attempt`, and once more when the comparison stopped it.
pub(crate) fn retried<T>(mut attempt: impl FnMut() -> Result<Outcome<T>>) -> Result<Retried<T>> {
    let first = match attempt()? {
        Outcome::Done(measured) => {
            return Ok(Retried::Done {
                measured,
                retried: None,
            });
        }
        Outcome::Stopped(reason) => reason,
    };
    Ok(match attempt()? {
        Outcome::Done(measured) => Retried::Done {
            measured,
            retried: Some(first),
        },
        Outcome::Stopped(second) => {
            Retried::Stopped(format!("{second}; its first attempt: {first}"))
        }
    })
}

/// Waits for the trial `child` on the Proving ground at `address` to exit;
/// when it is watched, reads its count of requests each second, and once
/// [`stopped`] says why, kills the trial's processes, each by its own ID,
/// and returns the reason.
#[expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous: a watched trial is watched by sleeping between reads of its Proving ground's request count (STYLE.md)"
)]
fn watched(child: &mut Child, address: &str, watch: Watch) -> Result<Option<String>> {
    if matches!(watch, Watch::None) {
        return Ok(None);
    }
    let started = trial::now();
    let (mut requests, mut grew) = (0, started);
    loop {
        if child
            .try_wait()
            .context("waiting for xtask compare trial")?
            .is_some()
        {
            return Ok(None);
        }
        std::thread::sleep(Duration::from_secs(1));
        let count = trial::request_count(address)?;
        if count != requests {
            (requests, grew) = (count, trial::now());
        }
        let reason = stopped(
            watch,
            started.elapsed().as_secs(),
            grew.elapsed().as_secs(),
            requests,
        );
        if let Some(reason) = reason {
            // Every process of the trial's tree, gathered before any dies
            // and leaves its children to another parent.
            for pid in sampler::tree(child.id()) {
                // One may have exited meanwhile.
                let _ = trial::kill(pid);
            }
            child.wait().context("waiting for the stopped trial")?;
            return Ok(Some(reason));
        }
    }
}

/// The pages of the site `config`'s tool crawls.
fn pages_of(config: Config, pages: u64, spider_pages: Option<u64>) -> u64 {
    match config {
        Config::Spider { .. } => spider_pages.unwrap_or(pages),
        Config::Foxhound { .. } | Config::Crawlee | Config::Scrapy => pages,
    }
}

/// Runs the timed trials of each measured cell over the site of `pages`,
/// each watched (see [`stopped`]): a stopped trial runs once more, and a
/// cell whose trial is stopped twice is not measured, its later trials not
/// run. Returns the trials and the cells stopped.
fn timed(
    executables: &Executables<'_>,
    options: &Options,
    pages: u64,
    state: &mut u64,
    pass: &str,
) -> Result<(Vec<Trial>, Vec<StoppedRun>)> {
    let cells = measured(options);
    let mut trials = Vec::new();
    let mut stopped: Vec<StoppedRun> = Vec::new();
    for (cell, config, number) in order(&cells, options.trials, state) {
        if stopped
            .iter()
            .any(|run| run.scenario == cell.scenario && run.cell == cell.name)
        {
            continue;
        }
        let site_pages = pages_of(config, pages, options.spider_pages);
        let mut started_at = None;
        let outcome = retried(|| {
            started_at = if waits_for_quiet(options) {
                quiet()
            } else {
                load()
            };
            let outcome = trial(
                executables,
                cell,
                config,
                site_pages,
                ("timed", None),
                Watch::Timed(number, site_pages),
            )?;
            if let Outcome::Stopped(reason) = &outcome {
                output::line(&format!(
                    "{pass}{} {} trial {number}: {reason}.",
                    cell.scenario.name(),
                    cell.name,
                ));
            }
            Ok(outcome)
        })?;
        let (measured, retried): (Measured, _) = match outcome {
            Retried::Done { measured, retried } => (measured, retried),
            Retried::Stopped(reason) => {
                output::line(&format!(
                    "{pass}{} {} trial {number} was stopped twice; the cell is not measured.",
                    cell.scenario.name(),
                    cell.name,
                ));
                stopped.push(StoppedRun {
                    scenario: cell.scenario,
                    cell: cell.name.to_owned(),
                    tool: config.tool(),
                    number,
                    reason,
                });
                continue;
            }
        };
        output::line(&format!(
            "{pass}{} {} trial {number}: {} pages in {:.3} s, peak {} bytes",
            cell.scenario.name(),
            cell.name,
            measured.work.pages,
            measured.seconds,
            measured.peak_bytes,
        ));
        trials.push(Trial {
            scenario: cell.scenario,
            cell: cell.name.to_owned(),
            tool: config.tool(),
            number,
            site_pages,
            load: started_at,
            retried,
            measured,
        });
    }
    // A stopped cell's trials before its stop are no figure either.
    trials.retain(|trial| {
        !stopped
            .iter()
            .any(|run| run.scenario == trial.scenario && run.cell == trial.cell)
    });
    Ok((trials, stopped))
}

/// Runs a counted cell: `runs` runs of each of `cells` under `mode`. Under
/// strace (`watched`), a run whose crawl stalls is stopped (see
/// [`stopped`]): its cell is not measured, and its later runs do not run.
fn counted(
    executables: &Executables<'_>,
    options: &Options,
    cells: &[(&'static Cell, Config)],
    runs: u32,
    (mode, watched): (&str, bool),
    state: &mut u64,
) -> Result<CountedCell> {
    let folder = tempfile::tempdir().context("creating the counts' folder")?;
    let mut done = Vec::new();
    let mut stopped: Vec<StoppedRun> = Vec::new();
    for (cell, config, number) in order(cells, runs, state) {
        if stopped
            .iter()
            .any(|run| run.scenario == cell.scenario && run.cell == cell.name)
        {
            continue;
        }
        let out = folder.path().join(format!(
            "{}-{}-{number}.out",
            cell.scenario.name(),
            cell.name
        ));
        let pages = pages_of(config, SITE_PAGES, options.spider_pages);
        let outcome = trial(
            executables,
            cell,
            config,
            pages,
            (mode, Some(&out)),
            if watched {
                Watch::Counted(number)
            } else {
                Watch::None
            },
        )?;
        let counted: Counted = match outcome {
            Outcome::Done(counted) => counted,
            Outcome::Stopped(reason) => {
                output::line(&format!(
                    "{} {} under {mode} {number}: {reason}; the cell is not measured.",
                    cell.scenario.name(),
                    cell.name,
                ));
                stopped.push(StoppedRun {
                    scenario: cell.scenario,
                    cell: cell.name.to_owned(),
                    tool: config.tool(),
                    number,
                    reason,
                });
                continue;
            }
        };
        output::line(&format!(
            "{} {} under {mode} {number}: {} pages, {} counted, {} syncs",
            cell.scenario.name(),
            cell.name,
            counted.work.pages,
            counted.count,
            counted.syncs
        ));
        done.push(CountedRun {
            scenario: cell.scenario,
            cell: cell.name.to_owned(),
            tool: config.tool(),
            number,
            count: counted.count,
            syncs: counted.syncs,
            work: counted.work,
        });
    }
    Ok(CountedCell::Measured {
        trials: runs,
        runs: done,
        stopped,
    })
}

/// Runs every declared trial and returns the trials file's contents.
pub(crate) fn run(
    executables: &Executables<'_>,
    commit: String,
    options: &Options,
) -> Result<Comparison> {
    let machine = machine();
    let order_seed = drawn_seed();
    let mut state = order_seed;
    let (trials, stopped) = timed(executables, options, SITE_PAGES, &mut state, "")?;
    let measured = measured(options);
    let syscalls = match strace_missing() {
        Some(reason) => {
            output::line(&format!("The system-call cell does not run: {reason}."));
            CountedCell::NotRun { reason }
        }
        None => counted(
            executables,
            options,
            &measured,
            options.trials,
            ("syscalls", true),
            &mut state,
        )?,
    };
    let growth = if options.growth {
        let (trials_run, stopped) =
            timed(executables, options, GROWTH_PAGES, &mut state, "growth ")?;
        GrowthCell::Measured {
            trials: options.trials,
            site_pages: GROWTH_PAGES,
            trials_run,
            stopped,
        }
    } else {
        GrowthCell::NotRun {
            reason: format!(
                "the growth cell runs with `--growth`: each measured cell crawls the {GROWTH_PAGES}-page site {} times, hours at `run_state.durability: full`",
                options.trials
            ),
        }
    };
    // ADR 0140's row compares Foxhound's instructions with Spider's.
    let scenario_1: Vec<_> = measured
        .iter()
        .copied()
        .filter(|(cell, config)| {
            cell.scenario == Scenario::Raw && INSTRUCTION_TOOLS.contains(&config.tool())
        })
        .collect();
    let instructions = match callgrind_missing() {
        _ if scenario_1.is_empty() => CountedCell::NotRun {
            reason: "the run asked for no scenario 1 cell".to_owned(),
        },
        Some(reason) => {
            output::line(&format!("The Callgrind cell does not run: {reason}."));
            CountedCell::NotRun { reason }
        }
        None => counted(
            executables,
            options,
            &scenario_1,
            INSTRUCTION_RUNS,
            ("callgrind", false),
            &mut state,
        )?,
    };
    Ok(Comparison {
        declared: Declared {
            trials: options.trials,
            statistic: STATISTIC.to_owned(),
            concurrency: CONCURRENCY,
            site: format!("/sites/{SITE_PAGES}/1"),
            spider: SPIDER_VERSION.to_owned(),
            crawlee: CRAWLEE_VERSION.to_owned(),
            scrapy: SCRAPY_VERSION.to_owned(),
            playwright: PLAYWRIGHT_VERSION.to_owned(),
            scrapy_playwright: SCRAPY_PLAYWRIGHT_VERSION.to_owned(),
            browser: super::trial::CHROME_FOR_TESTING_BUILD.to_owned(),
            scenarios: options.scenarios.clone(),
        },
        machine,
        commit,
        order_seed,
        trials,
        stopped,
        syscalls,
        growth,
        instructions,
    })
}

#[cfg(test)]
mod tests;
