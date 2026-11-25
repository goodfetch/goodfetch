//! What a run reads of the machine it measures on: its name, OS, CPU,
//! threads and memory, its 1-minute load, and the quiet check a timed trial
//! and a run under strace wait to see pass before they start: the cores the
//! run may use idle, no CPU or memory pressure, and nothing left of the
//! previous trial; the 1-minute load where the OS gives no pressure files.

use std::collections::BTreeSet;
use std::process::Command;
use std::time::{Duration, Instant};

use anyhow::{Result, bail};

use crate::output;

use super::super::trials::{
    Cores, Machine, QUIET_SAMPLE_SECONDS, QUIET_WAIT_SECONDS, QuietReading,
};

/// The machine: the name given it, OS and version, CPU, threads, memory,
/// and its load now.
#[expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous: it asks the OS's own commands and files with blocking calls (STYLE.md)"
)]
pub(super) fn machine(name: Option<String>) -> Machine {
    let read = |program: &str, arguments: &[&str]| {
        Command::new(program)
            .args(arguments)
            .output()
            .ok()
            .filter(|output| output.status.success())
            .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned())
            .unwrap_or_default()
    };
    let (os, cpu, memory_bytes) = if cfg!(target_os = "macos") {
        (
            format!("macOS {}", read("sw_vers", &["-productVersion"])),
            read("sysctl", &["-n", "machdep.cpu.brand_string"]),
            read("sysctl", &["-n", "hw.memsize"]).parse().ok(),
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
        // `MemTotal:       65843412 kB`, in KiB.
        let memory_bytes = std::fs::read_to_string("/proc/meminfo")
            .ok()
            .and_then(|text| {
                text.lines()
                    .find_map(|line| line.strip_prefix("MemTotal:"))?
                    .split_whitespace()
                    .next()?
                    .parse::<u64>()
                    .ok()
            })
            .map(|kib| kib * 1024);
        (
            format!("{} {}", std::env::consts::OS, read("uname", &["-r"])),
            cpu,
            memory_bytes,
        )
    };
    let load = load();
    Machine {
        class: "local-controlled".to_owned(),
        name,
        os,
        arch: std::env::consts::ARCH.to_owned(),
        cpu,
        threads: std::thread::available_parallelism().map_or(0, std::num::NonZero::get),
        memory_bytes,
        load,
    }
}

/// The 1-minute load average now, where the OS gives one.
#[expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous: it reads /proc with a blocking call (STYLE.md)"
)]
pub(super) fn load() -> Option<f64> {
    std::fs::read_to_string("/proc/loadavg")
        .ok()
        .and_then(|text| text.split_whitespace().next()?.parse().ok())
}

/// The seconds between the end of one reading of the quiet check and the
/// next, while a run waits for the check to pass.
const POLL_SECONDS: u64 = 10;

/// How long a run waits for the quiet check to pass.
#[derive(Clone, Copy, Debug)]
enum Limit {
    /// Up to these seconds, then it starts anyway, what the check read
    /// recorded: a run given no time to keep.
    Within(u64),
    /// Up to these seconds, then it stops, naming the failed reading: a run
    /// given the time its waits must end by (`--quiet-until`).
    By(u64),
}

/// Waits for the quiet check to pass before a timed trial, so it starts on a
/// quiet machine, a browser trial's own processes gone, or before a run
/// under strace; gives what the check read when it started, or none where
/// the OS gives neither pressure files nor a load. `load` is the 1-minute
/// load the machine must be under where it gives no pressure files:
/// [`QUIET_LOAD`](super::super::trials::QUIET_LOAD) before a timed trial,
/// [`STRACE_QUIET_LOAD`](super::super::trials::STRACE_QUIET_LOAD) before a
/// run under strace.
///
/// Without `until`, it waits up to [`QUIET_WAIT_SECONDS`], then starts
/// anyway. Given `until`, the time the run's waits must end by (the quiet
/// window's end), it waits up to it, then fails, naming the reading the
/// machine failed, so that no trial starts on a machine the check fails.
#[expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous: a run waits for a quiet machine by sleeping between readings (STYLE.md)"
)]
pub(super) fn quiet(load: f64, until: Option<jiff::Timestamp>) -> Result<Option<QuietReading>> {
    let limit = until.map_or(Limit::Within(QUIET_WAIT_SECONDS), |until| {
        Limit::By(seconds_left(until, jiff::Timestamp::now()))
    });
    let started = Instant::now();
    waited(
        load,
        limit,
        reading,
        |seconds| std::thread::sleep(Duration::from_secs(seconds)),
        || started.elapsed().as_secs(),
    )
}

/// The whole seconds from `now` to `until`, none once it has passed.
fn seconds_left(until: jiff::Timestamp, now: jiff::Timestamp) -> u64 {
    u64::try_from(until.as_second() - now.as_second()).unwrap_or(0)
}

/// [`quiet`]'s wait: the check read with `read`, each pause between two
/// readings slept with `sleep`, given its seconds, and the seconds since the
/// wait began told by `clock`.
fn waited(
    load: f64,
    limit: Limit,
    mut read: impl FnMut() -> Option<QuietReading>,
    mut sleep: impl FnMut(u64),
    clock: impl Fn() -> u64,
) -> Result<Option<QuietReading>> {
    let began = clock();
    let mut paused = false;
    loop {
        let Some(reading) = read() else {
            return Ok(None);
        };
        let waited = clock() - began;
        let Some(failed) = reading.failed(load) else {
            if paused {
                output::line(&format!(
                    "Waited {waited} s for the machine to pass the quiet check ({}).",
                    reading.check(load)
                ));
            }
            return Ok(Some(reading));
        };
        match limit {
            Limit::Within(seconds) if waited >= seconds => {
                output::line(&format!(
                    "Starting after {waited} s on a machine that is not quiet: {failed}."
                ));
                return Ok(Some(reading));
            }
            Limit::By(seconds) if waited >= seconds => {
                bail!("the machine was not quiet by the end given (`--quiet-until`): {failed}")
            }
            Limit::Within(_) | Limit::By(_) => {}
        }
        sleep(POLL_SECONDS);
        paused = true;
    }
}

/// The quiet check's reading now: of the run's cores where the OS gives
/// pressure files, which takes [`QUIET_SAMPLE_SECONDS`], else the 1-minute
/// load, where it gives one.
fn reading() -> Option<QuietReading> {
    cores()
        .map(QuietReading::Cores)
        .or_else(|| load().map(QuietReading::Load))
}

/// The readings of the cores the run may use, where the OS gives pressure
/// files and `/proc`: Linux with PSI.
#[expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous: it reads /proc with blocking calls and sleeps through the sample (STYLE.md)"
)]
fn cores() -> Option<Cores> {
    let read = |path: &str| std::fs::read_to_string(path).ok();
    // Read first, so a kernel without PSI takes no sample.
    pressure(&read("/proc/pressure/cpu")?, "some")?;
    let allowed = allowed_cores(&read("/proc/self/status")?)?;
    let before = read("/proc/stat")?;
    std::thread::sleep(Duration::from_secs(QUIET_SAMPLE_SECONDS));
    let after = read("/proc/stat")?;
    Some(Cores {
        least_idle: least_idle(&before, &after, &allowed)?,
        cpu_pressure: pressure(&read("/proc/pressure/cpu")?, "some")?,
        memory_pressure: pressure(&read("/proc/pressure/memory")?, "full")?,
        leftovers: descendants(&processes(), std::process::id()),
    })
}

/// The cores a process may run on, from the `Cpus_allowed_list` of its
/// `/proc/<pid>/status`, such as `0-3,8,10-11`.
fn allowed_cores(status: &str) -> Option<BTreeSet<usize>> {
    let list = status
        .lines()
        .find_map(|line| line.strip_prefix("Cpus_allowed_list:"))?
        .trim();
    let mut cores = BTreeSet::new();
    for part in list.split(',') {
        match part.split_once('-') {
            Some((low, high)) => cores.extend(low.parse::<usize>().ok()?..=high.parse().ok()?),
            None => {
                cores.insert(part.parse().ok()?);
            }
        }
    }
    Some(cores)
}

/// The share of the time between two readings of `/proc/stat` that the
/// busiest of `cores` spent idle, from 0 to 1; none when `/proc/stat` lists
/// none of them.
fn least_idle(before: &str, after: &str, cores: &BTreeSet<usize>) -> Option<f64> {
    // `cpuN user nice system idle iowait irq softirq steal guest guest_nice`,
    // in ticks; guest time is counted in user and nice already.
    let ticks = |text: &str| -> Vec<(usize, u64, u64)> {
        text.lines()
            .filter_map(|line| {
                let mut fields = line.split_whitespace();
                let core = fields.next()?.strip_prefix("cpu")?.parse().ok()?;
                let ticks: Vec<u64> = fields
                    .take(8)
                    .map_while(|field| field.parse().ok())
                    .collect();
                let idle = *ticks.get(3)?;
                Some((core, idle, ticks.iter().sum()))
            })
            .filter(|(core, _, _)| cores.contains(core))
            .collect()
    };
    let before = ticks(before);
    ticks(after)
        .into_iter()
        .filter_map(|(core, idle, total)| {
            let (_, idle_before, total_before) = before.iter().find(|(c, _, _)| *c == core)?;
            let elapsed = total
                .checked_sub(*total_before)
                .filter(|ticks| *ticks > 0)?;
            Some(share(idle.saturating_sub(*idle_before), elapsed))
        })
        .reduce(f64::min)
}

/// `part` over `whole`, as a float.
#[expect(
    clippy::cast_precision_loss,
    reason = "tick counts of a few seconds fit a float's mantissa"
)]
fn share(part: u64, whole: u64) -> f64 {
    part as f64 / whole as f64
}

/// `avg10` of the `kind` line (`some` or `full`) of a pressure file of
/// `/proc/pressure/`, in percent.
fn pressure(text: &str, kind: &str) -> Option<f64> {
    text.lines()
        .find_map(|line| line.strip_prefix(kind)?.strip_prefix(' '))?
        .split_whitespace()
        .find_map(|field| field.strip_prefix("avg10="))?
        .parse()
        .ok()
}

/// Each process `/proc` lists now: its id, its parent's and its state.
#[expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous: it reads /proc with blocking calls (STYLE.md)"
)]
fn processes() -> Vec<(u32, u32, char)> {
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return Vec::new();
    };
    entries
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let pid = entry.file_name().to_str()?.parse().ok()?;
            let stat = std::fs::read_to_string(entry.path().join("stat")).ok()?;
            // `pid (comm) state ppid …`, where comm may hold spaces and
            // parentheses: the fields follow the last `)`.
            let mut fields = stat.get(stat.rfind(')')? + 1..)?.split_whitespace();
            let state = fields.next()?.chars().next()?;
            let parent = fields.next()?.parse().ok()?;
            Some((pid, parent, state))
        })
        .collect()
}

/// How many of `processes` descend from `root`, at any depth, and still
/// run: an exited process the OS lists until it is reaped (`Z`) does not.
fn descendants(processes: &[(u32, u32, char)], root: u32) -> usize {
    let mut found = 0;
    let mut parents = vec![root];
    while let Some(parent) = parents.pop() {
        for &(pid, _, state) in processes.iter().filter(|(_, of, _)| *of == parent) {
            if state != 'Z' {
                found += 1;
            }
            parents.push(pid);
        }
    }
    found
}

#[cfg(test)]
mod tests;
