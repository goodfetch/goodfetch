//! `run`: the declared trials of both tools in a random order, each against
//! a fresh Proving ground whose observations then say what the trial did,
//! written to the trials file.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs and process bans target async crates (STYLE.md)"
)]

use std::io::{BufRead as _, BufReader};
use std::path::Path;
use std::process::{Child, Command, Stdio};

use anyhow::{Context as _, Result, bail};
use serde::Deserialize;
use serde_json::Value;

use crate::output;

use super::same_work;
use super::trial::HOST;
use super::trials::{
    CONCURRENCY, Comparison, Declared, Machine, SITE_PAGES, SPIDER_VERSION, STATISTIC, TRIALS,
    Tool, Trial,
};

/// The Proving ground's seed.
const SEED: u64 = 1574;

/// The executables a run takes.
pub(crate) struct Executables<'a> {
    pub(crate) proving_ground: &'a Path,
    pub(crate) foxhound: &'a Path,
    pub(crate) spider: &'a Path,
}

/// A Proving ground serving for one trial, stopped when dropped.
struct Serving {
    child: Child,
    address: String,
}

impl Drop for Serving {
    fn drop(&mut self) {
        // A trial's Proving ground holds nothing worth a clean stop.
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[derive(Deserialize)]
struct ServeLine {
    address: String,
}

fn serve(proving_ground: &Path) -> Result<Serving> {
    let mut child = Command::new(proving_ground)
        .args(["serve", "--seed", &SEED.to_string(), "--copy", "local"])
        .args(["--address", "127.0.0.1:0", "--network-profile", "none"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .context("starting the Proving ground")?;
    let mut line = String::new();
    let stdout = child.stdout.take().context("its stdout is piped")?;
    BufReader::new(stdout)
        .read_line(&mut line)
        .context("reading the Proving ground's first line")?;
    // Stopped on drop from here, even when its line is not one.
    let mut serving = Serving {
        child,
        address: String::new(),
    };
    let first: ServeLine = serde_json::from_str(&line)
        .with_context(|| format!("the Proving ground's first line: {}", line.trim_end()))?;
    serving.address = first.address;
    Ok(serving)
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
/// seconds and peak.
fn trial(tool: Tool, executable: &Path, address: &str, pages: u64) -> Result<(f64, u64)> {
    let probe = std::env::current_exe().context("finding xtask's own executable")?;
    let output = Command::new(probe)
        .args([
            "compare-spider",
            "trial",
            "--tool",
            tool.name(),
            "--executable",
        ])
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

/// Runs every declared trial and returns the trials file's contents.
/// `spider_pages` gives Spider a site of another size, which the same-work
/// check must refuse.
pub(crate) fn run(
    executables: &Executables<'_>,
    commit: String,
    spider_pages: Option<u64>,
) -> Result<Comparison> {
    let now = jiff::Timestamp::now();
    // Any seed will do; the one drawn is recorded with the trials.
    let order_seed =
        now.as_second().unsigned_abs() ^ u64::from(now.subsec_nanosecond().unsigned_abs()) << 32;
    let site = format!("/sites/{SITE_PAGES}/");
    let mut trials = Vec::new();
    for (tool, number) in order(order_seed) {
        let (executable, pages) = match tool {
            Tool::Foxhound => (executables.foxhound, SITE_PAGES),
            Tool::Spider => (executables.spider, spider_pages.unwrap_or(SITE_PAGES)),
        };
        let serving = serve(executables.proving_ground)?;
        let (seconds, peak_bytes) = trial(tool, executable, &serving.address, pages)?;
        let observations: Value = ureq::get(&format!("http://{}/observations", serving.address))
            .call()
            .context("reading the Proving ground's observations")?
            .body_mut()
            .with_config()
            .limit(u64::MAX)
            .read_json()
            .context("parsing the Proving ground's observations")?;
        drop(serving);
        // Another site's pages are no pages of this one: give its own
        // prefix, so the check sees what that trial really fetched.
        let work = same_work::work_of(&observations, HOST, &format!("/sites/{pages}/"))?;
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
    })
}
