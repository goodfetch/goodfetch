//! Resource-limit sampling and enforcement for a running process Plugin
//! instance (ADR 0033, issue 837): checks memory, CPU time, process count and
//! open files against the approved manifest's [`Limits`], killing the instance
//! with `plugin_limit_exceeded` under the crash rules when any limit is
//! exceeded.

use std::fmt;

use goodfetch_protocol::plugin::{Limits, PluginResourceLimit};

/// One resource-usage snapshot of a Plugin instance's process group.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ResourceSample {
    /// Total resident memory across the instance's process group, in bytes.
    pub memory_bytes: u64,
    /// Total CPU time (user + system) consumed across the process group, in
    /// whole seconds.
    pub cpu_time_secs: u64,
    /// Number of live OS processes in the instance's process group.
    pub processes: u32,
    /// Total open file descriptors held across the instance's process group.
    pub open_files: u32,
}

/// Checks `sample` against `limits`, returning the first exceeded resource and
/// its configured limit value, if any.
///
/// `open_files` triggers at `>= limits.open_files` because Linux sets
/// `RLIMIT_NOFILE` to `(open_files, open_files)`, so a process that fills all
/// descriptors hits `EMFILE` at `open_files` and can never report
/// `open_files + 1`. `cpu_time` triggers at `>= limits.cpu_time` matching
/// `RLIMIT_CPU`'s soft limit.
#[must_use]
pub fn exceeded(limits: &Limits, sample: &ResourceSample) -> Option<(PluginResourceLimit, u64)> {
    if sample.memory_bytes > limits.memory {
        return Some((PluginResourceLimit::Memory, limits.memory));
    }
    if limits.cpu_time > 0 && sample.cpu_time_secs >= limits.cpu_time {
        return Some((PluginResourceLimit::CpuTime, limits.cpu_time));
    }
    if sample.processes > limits.processes {
        return Some((PluginResourceLimit::Processes, u64::from(limits.processes)));
    }
    if limits.open_files > 0 && sample.open_files >= limits.open_files {
        return Some((PluginResourceLimit::OpenFiles, u64::from(limits.open_files)));
    }
    None
}

/// The configured limit value for `resource` in `limits`.
#[must_use]
pub const fn limit_value(limits: &Limits, resource: PluginResourceLimit) -> u64 {
    match resource {
        PluginResourceLimit::Memory => limits.memory,
        PluginResourceLimit::CpuTime => limits.cpu_time,
        PluginResourceLimit::Processes => limits.processes as u64,
        PluginResourceLimit::OpenFiles => limits.open_files as u64,
    }
}

/// Samples a Plugin instance's resource usage for limit enforcement.
pub trait ResourceMonitor: fmt::Debug + Send + Sync {
    /// Returns the current resource usage sample, or `None` if the process is
    /// already gone or unreadable.
    fn sample(&self) -> Option<ResourceSample>;
}

/// Linux `/proc` process-group resource monitor for a confined Plugin process.
#[cfg(target_os = "linux")]
#[derive(Debug)]
pub struct ProcGroupMonitor {
    root_pid: u32,
    pgid: i32,
}

#[cfg(target_os = "linux")]
impl ProcGroupMonitor {
    /// Monitors the process group rooted at `root_pid` (whose PGID equals
    /// `root_pid` because [`crate::children::Children`] starts every Plugin in
    /// its own process group).
    #[must_use]
    pub fn for_pid(root_pid: u32) -> Option<Self> {
        let pgid = i32::try_from(root_pid).ok()?;
        Some(Self { root_pid, pgid })
    }
}

#[cfg(target_os = "linux")]
impl ResourceMonitor for ProcGroupMonitor {
    fn sample(&self) -> Option<ResourceSample> {
        sample_proc_group(self.root_pid, self.pgid)
    }
}

/// Reads `/proc` for all live processes belonging to `pgid`, summing memory,
/// CPU time, process count, and open file descriptors.
#[cfg(target_os = "linux")]
#[expect(
    clippy::disallowed_methods,
    reason = "reads virtual `/proc` kernel files on the monitoring path"
)]
pub(crate) fn sample_proc_group(root_pid: u32, group_id: i32) -> Option<ResourceSample> {
    let ticks_per_sec = clock_ticks_per_sec();
    let mut pids = Vec::new();
    if let Ok(entries) = std::fs::read_dir("/proc") {
        for entry in entries.flatten() {
            let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            let Ok(pid) = name.parse::<u32>() else {
                continue;
            };
            if let Some((proc_pgid, _)) = read_stat(pid)
                && proc_pgid == group_id
            {
                pids.push(pid);
            }
        }
    }
    if pids.is_empty() && read_stat(root_pid).is_some() {
        pids.push(root_pid);
    }
    if pids.is_empty() {
        return None;
    }
    let mut sample = ResourceSample::default();
    let mut total_ticks: u64 = 0;
    for pid in pids {
        let Some((_, ticks)) = read_stat(pid) else {
            continue;
        };
        sample.processes = sample.processes.saturating_add(1);
        total_ticks = total_ticks.saturating_add(ticks);
        sample.memory_bytes = sample.memory_bytes.saturating_add(read_rss_bytes(pid));
        sample.open_files = sample.open_files.saturating_add(read_fd_count(pid));
    }
    if sample.processes == 0 {
        return None;
    }
    sample.cpu_time_secs = total_ticks / ticks_per_sec.max(1);
    Some(sample)
}

#[cfg(target_os = "linux")]
#[expect(
    clippy::disallowed_methods,
    reason = "reads `/proc/<pid>/stat` virtual kernel file"
)]
fn read_stat(pid: u32) -> Option<(i32, u64)> {
    let text = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let after_comm = text.rsplit_once(") ")?.1;
    let mut fields = after_comm.split_whitespace();
    // Fields after `(comm)`: 0=state, 1=ppid, 2=pgrp, ..., 11=utime, 12=stime.
    let pgrp: i32 = fields.nth(2)?.parse().ok()?;
    let utime: u64 = fields.nth(8)?.parse().ok()?;
    let stime: u64 = fields.next()?.parse().ok()?;
    Some((pgrp, utime.saturating_add(stime)))
}

#[cfg(target_os = "linux")]
#[expect(
    clippy::disallowed_methods,
    reason = "reads `/proc/<pid>/status` virtual kernel file"
)]
fn read_rss_bytes(pid: u32) -> u64 {
    let Ok(text) = std::fs::read_to_string(format!("/proc/{pid}/status")) else {
        return 0;
    };
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("VmRSS:") {
            let kib = rest
                .split_whitespace()
                .next()
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(0);
            return kib.saturating_mul(1024);
        }
    }
    0
}

#[cfg(target_os = "linux")]
#[expect(
    clippy::disallowed_methods,
    reason = "reads `/proc/<pid>/fd` virtual kernel directory"
)]
fn read_fd_count(pid: u32) -> u32 {
    let Ok(entries) = std::fs::read_dir(format!("/proc/{pid}/fd")) else {
        return 0;
    };
    u32::try_from(entries.count()).unwrap_or(u32::MAX)
}

#[cfg(target_os = "linux")]
#[expect(unsafe_code, reason = "`sysconf(_SC_CLK_TCK)` is a POSIX C call")]
fn clock_ticks_per_sec() -> u64 {
    // SAFETY: `sysconf(_SC_CLK_TCK)` reads a kernel constant with no pointers.
    let ticks = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
    u64::try_from(ticks).ok().filter(|&v| v > 0).unwrap_or(100)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exceeded_reports_each_resource_limit_in_turn() {
        let limits = Limits {
            memory: 1024,
            cpu_time: 10,
            processes: 2,
            open_files: 16,
            concurrency: 4,
        };
        let within = ResourceSample {
            memory_bytes: 1024,
            cpu_time_secs: 9,
            processes: 2,
            open_files: 15,
        };
        assert_eq!(exceeded(&limits, &within), None);
        assert_eq!(
            exceeded(
                &limits,
                &ResourceSample {
                    memory_bytes: 1025,
                    ..within
                }
            ),
            Some((PluginResourceLimit::Memory, 1024))
        );
        assert_eq!(
            exceeded(
                &limits,
                &ResourceSample {
                    cpu_time_secs: 10,
                    ..within
                }
            ),
            Some((PluginResourceLimit::CpuTime, 10))
        );
        assert_eq!(
            exceeded(
                &limits,
                &ResourceSample {
                    processes: 3,
                    ..within
                }
            ),
            Some((PluginResourceLimit::Processes, 2))
        );
        assert_eq!(
            exceeded(
                &limits,
                &ResourceSample {
                    open_files: 16,
                    ..within
                }
            ),
            Some((PluginResourceLimit::OpenFiles, 16))
        );
    }
}
