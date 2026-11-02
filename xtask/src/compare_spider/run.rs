//! `run`: the declared trials of both tools in a random order, each against
//! a fresh Proving ground whose observations then say what the trial did,
//! then the Callgrind cell's runs of each tool in the same way, written to
//! the trials file.

use std::path::Path;
use std::process::{Command, Stdio};

use anyhow::{Context as _, Result, bail};
use serde_json::Value;

use crate::output;
use crate::rss::crawl::{HOST, Serving, serve};

use super::same_work::{self, Work};
use super::trial::{callgrind_missing, instructions_of};
use super::trials::{
    CONCURRENCY, Comparison, Declared, INSTRUCTION_RUNS, INSTRUCTION_SCENARIO, InstructionCell,
    InstructionRun, Machine, SITE_PAGES, SPIDER_VERSION, STATISTIC, TRIALS, Tool, Trial,
};

/// The executables a run takes.
pub(crate) struct Executables<'a> {
    pub(crate) proving_ground: &'a Path,
    pub(crate) foxhound: &'a Path,
    pub(crate) spider: &'a Path,
}

/// `SplitMix64`, for the trials' order: nothing here needs more.
fn next(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

/// Each tool's trials, shuffled by `seed` (Fisher–Yates).
fn order(seed: u64) -> Vec<(Tool, u32)> {
    let mut order: Vec<(Tool, u32)> = Tool::ALL
        .iter()
        .flat_map(|&tool| (1..=TRIALS).map(move |number| (tool, number)))
        .collect();
    let mut state = seed;
    for last in (1..order.len()).rev() {
        let pick = usize::try_from(next(&mut state) % (last as u64 + 1))
            .expect("invariant: an index below the trial count fits a usize");
        order.swap(last, pick);
    }
    order
}

/// The machine: OS and version, CPU, threads.
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
    Machine {
        class: "local-controlled".to_owned(),
        os,
        arch: std::env::consts::ARCH.to_owned(),
        cpu,
        threads: std::thread::available_parallelism().map_or(0, std::num::NonZero::get),
    }
}

/// Runs one trial through `xtask compare-spider trial` and returns its
/// seconds and peak. With `callgrind`, the tool runs under Callgrind, which
/// writes its output there.
#[expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous: a trial runs as a blocking child process (STYLE.md)"
)]
fn trial(
    tool: Tool,
    executable: &Path,
    address: &str,
    pages: u64,
    callgrind: Option<&Path>,
) -> Result<(f64, u64)> {
    let probe = std::env::current_exe().context("finding xtask's own executable")?;
    let mut command = Command::new(probe);
    command.args(["compare-spider", "trial"]);
    if let Some(output) = callgrind {
        command.arg("--callgrind").arg(output);
    }
    let output = command
        .args(["--tool", tool.name(), "--executable"])
        .arg(executable)
        .args(["--address", address, "--pages", &pages.to_string()])
        .args(["--concurrency", &CONCURRENCY.to_string()])
        .stderr(Stdio::inherit())
        .output()
        .context("running xtask compare-spider trial")?;
    if !output.status.success() {
        bail!("{}'s trial failed", tool.name());
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let field = |name: &str| {
        stdout
            .split_whitespace()
            .find_map(|word| word.strip_prefix(name))
            .with_context(|| format!("the trial printed no {name}"))
    };
    Ok((
        field("seconds=")?.parse().context("parsing the seconds")?,
        field("peak-bytes=")?.parse().context("parsing the peak")?,
    ))
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

/// What the Proving ground `serving` saw a trial do on the site of `pages`.
fn observed(serving: &Serving, pages: u64) -> Result<Work> {
    let observations: Value = ureq::get(&format!("http://{}/observations", serving.address))
        .call()
        .context("reading the Proving ground's observations")?
        .body_mut()
        .with_config()
        .limit(u64::MAX)
        .read_json()
        .context("parsing the Proving ground's observations")?;
    // Another site's pages are no pages of this one: give its own
    // prefix, so the check sees what that trial really fetched.
    same_work::work_of(&observations, HOST, &format!("/sites/{pages}/"))
}

/// Runs the Callgrind cell, when this host can.
#[expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous: it reads Callgrind's output with a blocking call (STYLE.md)"
)]
fn instruction_cell(
    executables: &Executables<'_>,
    spider_pages: Option<u64>,
) -> Result<InstructionCell> {
    if let Some(reason) = callgrind_missing() {
        output::line(&format!("The Callgrind cell does not run: {reason}."));
        return Ok(InstructionCell::NotRun { reason });
    }
    let folder = tempfile::tempdir().context("creating Callgrind's folder")?;
    let mut runs = Vec::new();
    for tool in Tool::ALL {
        for number in 1..=INSTRUCTION_RUNS {
            let (executable, pages) = side(executables, tool, spider_pages);
            let out = folder.path().join(format!("{}-{number}.out", tool.name()));
            let serving = serve(executables.proving_ground)?;
            trial(tool, executable, &serving.address, pages, Some(&out))?;
            let work = observed(&serving, pages)?;
            drop(serving);
            let text = std::fs::read_to_string(&out)
                .with_context(|| format!("reading {}", out.display()))?;
            let instructions = instructions_of(&text)?;
            output::line(&format!(
                "{} under Callgrind {number}: {} pages, {instructions} instructions",
                tool.name(),
                work.pages.len()
            ));
            runs.push(InstructionRun {
                tool,
                number,
                instructions,
                pages: work.pages.into_iter().collect(),
                requests: work.requests,
                bytes: work.bytes,
            });
        }
    }
    Ok(InstructionCell::Measured {
        scenario: INSTRUCTION_SCENARIO.to_owned(),
        trials: INSTRUCTION_RUNS,
        runs,
    })
}

/// `tool`'s executable and the pages of the site it crawls.
fn side<'a>(
    executables: &Executables<'a>,
    tool: Tool,
    spider_pages: Option<u64>,
) -> (&'a Path, u64) {
    match tool {
        Tool::Foxhound => (executables.foxhound, SITE_PAGES),
        Tool::Spider => (executables.spider, spider_pages.unwrap_or(SITE_PAGES)),
    }
}

/// Runs every declared trial and returns the trials file's contents.
/// `spider_pages` gives Spider a site of another size, which the same-work
/// check must refuse.
pub(crate) fn run(
    executables: &Executables<'_>,
    commit: String,
    spider_pages: Option<u64>,
) -> Result<Comparison> {
    let order_seed = drawn_seed();
    let site = format!("/sites/{SITE_PAGES}/");
    let mut trials = Vec::new();
    for (tool, number) in order(order_seed) {
        let (executable, pages) = side(executables, tool, spider_pages);
        let serving = serve(executables.proving_ground)?;
        let (seconds, peak_bytes) = trial(tool, executable, &serving.address, pages, None)?;
        let work = observed(&serving, pages)?;
        drop(serving);
        output::line(&format!(
            "{} trial {number}: {} pages in {seconds:.3} s, peak {peak_bytes} bytes",
            tool.name(),
            work.pages.len()
        ));
        trials.push(Trial {
            tool,
            number,
            seconds,
            peak_bytes,
            pages: work.pages.into_iter().collect(),
            requests: work.requests,
            bytes: work.bytes,
        });
    }
    Ok(Comparison {
        declared: Declared {
            trials: TRIALS,
            statistic: STATISTIC.to_owned(),
            concurrency: CONCURRENCY,
            site: format!("{site}1"),
            spider: SPIDER_VERSION.to_owned(),
        },
        machine: machine(),
        commit,
        order_seed,
        trials,
        instructions: instruction_cell(executables, spider_pages)?,
    })
}
