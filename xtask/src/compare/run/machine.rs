//! What a run reads of the machine it measures on: its name, OS, CPU,
//! threads and memory, and its 1-minute load, which a timed trial waits to
//! see fall before it starts.

use std::process::Command;

use crate::output;

use super::super::trials::{Machine, QUIET_LOAD, QUIET_WAIT_SECONDS};

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

/// Waits, up to [`QUIET_WAIT_SECONDS`], for the 1-minute load to fall under
/// [`QUIET_LOAD`], so a timed trial starts on a quiet machine, a browser
/// trial's own load included; the load it starts at, where the OS gives
/// one.
#[expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous: a run waits for a quiet machine by sleeping between reads of its load (STYLE.md)"
)]
pub(super) fn quiet() -> Option<f64> {
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
