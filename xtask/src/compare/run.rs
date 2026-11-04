//! `run`: every measured cell's declared trials in one random order, each
//! against a fresh Proving ground, then the system-call cell's, the growth
//! cell's (when asked for) and the Callgrind cell's runs in the same way,
//! written to the trials file.

use std::path::Path;
use std::process::{Command, Stdio};

use anyhow::{Context as _, Result, bail};

use crate::output;
use crate::rss::crawl::serve;

use super::matrix::{self, Cell, Config, Scenario};
use super::trial::{Counted, callgrind_missing, strace_missing};
use super::trials::{
    CONCURRENCY, CRAWLEE_VERSION, Comparison, CountedCell, CountedRun, Declared, GROWTH_PAGES,
    GrowthCell, INSTRUCTION_RUNS, INSTRUCTION_TOOLS, Machine, Measured, PLAYWRIGHT_VERSION,
    QUIET_LOAD, QUIET_WAIT_SECONDS, SCRAPY_PLAYWRIGHT_VERSION, SCRAPY_VERSION, SITE_PAGES,
    SPIDER_VERSION, STATISTIC, Trial,
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
/// `output` name a counted trial's tool and file.
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
) -> Result<T> {
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
    let output = command
        .stderr(Stdio::inherit())
        .output()
        .context("running xtask compare trial")?;
    drop(serving);
    if !output.status.success() {
        bail!(
            "{}'s trial in scenario {} failed",
            cell.name,
            cell.scenario.name()
        );
    }
    serde_json::from_slice(&output.stdout).with_context(|| {
        format!(
            "parsing the trial's figures: {}",
            String::from_utf8_lossy(&output.stdout)
        )
    })
}

/// The pages of the site `config`'s tool crawls.
fn pages_of(config: Config, pages: u64, spider_pages: Option<u64>) -> u64 {
    match config {
        Config::Spider { .. } => spider_pages.unwrap_or(pages),
        Config::Foxhound { .. } | Config::Crawlee | Config::Scrapy => pages,
    }
}

/// Runs the timed trials of each measured cell over the site of `pages`.
fn timed(
    executables: &Executables<'_>,
    options: &Options,
    pages: u64,
    state: &mut u64,
    pass: &str,
) -> Result<Vec<Trial>> {
    let cells = measured(options);
    let mut trials = Vec::new();
    for (cell, config, number) in order(&cells, options.trials, state) {
        let site_pages = pages_of(config, pages, options.spider_pages);
        let load = if waits_for_quiet(options) {
            quiet()
        } else {
            load()
        };
        let measured: Measured = trial(executables, cell, config, site_pages, ("timed", None))?;
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
            load,
            measured,
        });
    }
    Ok(trials)
}

/// Runs a counted cell: `runs` runs of each of `cells` under `mode`.
fn counted(
    executables: &Executables<'_>,
    options: &Options,
    cells: &[(&'static Cell, Config)],
    runs: u32,
    mode: &str,
    state: &mut u64,
) -> Result<CountedCell> {
    let folder = tempfile::tempdir().context("creating the counts' folder")?;
    let mut done = Vec::new();
    for (cell, config, number) in order(cells, runs, state) {
        let out = folder.path().join(format!(
            "{}-{}-{number}.out",
            cell.scenario.name(),
            cell.name
        ));
        let pages = pages_of(config, SITE_PAGES, options.spider_pages);
        let counted: Counted = trial(executables, cell, config, pages, (mode, Some(&out)))?;
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
    let trials = timed(executables, options, SITE_PAGES, &mut state, "")?;
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
            "syscalls",
            &mut state,
        )?,
    };
    let growth = if options.growth {
        GrowthCell::Measured {
            trials: options.trials,
            site_pages: GROWTH_PAGES,
            trials_run: timed(executables, options, GROWTH_PAGES, &mut state, "growth ")?,
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
            "callgrind",
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
        syscalls,
        growth,
        instructions,
    })
}

#[cfg(test)]
mod tests;
