//! What the governor reads of the machine at each sample (ADR 0040): the
//! Memory allowance, the host's own memory pressure, the footprint of the
//! engine and its children, and the engine's CPU time. A value, never a
//! global: Linux's readings come from a folder laid out as `/proc`, the OS's
//! own by default and a test's otherwise, so every rule is checked on any
//! OS; macOS's and Windows' come from their C calls.

use std::path::{Path, PathBuf};
use std::time::Duration;

use goodfetch_protocol::governor::{AllowanceSource, GovernorLevel};

mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(windows)]
mod windows;

/// Where the governor reads the machine.
#[derive(Clone, Debug)]
pub struct System(Source);

#[derive(Clone, Debug)]
enum Source {
    /// The OS the engine runs on.
    Os,
    /// A folder laid out as Linux's `/proc`.
    LinuxFiles(PathBuf),
}

/// What a reading needs beside the machine.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Inputs {
    /// `governor.memory_allowance`.
    pub(crate) memory_allowance: Option<u64>,
    /// `governor.pressure`, in percent, which the host's memory in use is
    /// judged against too.
    pub(crate) pressure: u64,
    /// `governor.critical`, in percent.
    pub(crate) critical: u64,
}

/// What one reading found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SystemReading {
    pub(crate) allowance: u64,
    pub(crate) allowance_source: AllowanceSource,
    pub(crate) used: u64,
    pub(crate) host: GovernorLevel,
    /// The engine's CPU time since it started.
    pub(crate) cpu_time: Duration,
}

impl System {
    /// The OS the engine runs on, which only `main` and the engine's own
    /// default use.
    pub fn os() -> Self {
        Self(Source::Os)
    }

    /// A folder laid out as Linux's `/proc`, whose `self` is the engine,
    /// and whose `self/mountinfo` names where the cgroup file system is.
    pub fn linux_files(proc: PathBuf) -> Self {
        Self(Source::LinuxFiles(proc))
    }

    /// Reads the machine now. Blocking.
    pub(crate) fn read(&self, inputs: &Inputs) -> SystemReading {
        let found = match &self.0 {
            Source::LinuxFiles(proc) => linux_files(proc, inputs),
            Source::Os => os(inputs),
        };
        let (allowance, allowance_source) = tightest(
            found.cgroup,
            found.job,
            inputs.memory_allowance,
            found.physical,
        );
        SystemReading {
            allowance,
            allowance_source,
            used: found.used,
            host: found.host,
            cpu_time: found.cpu_time,
        }
    }
}

/// What a platform found, before the allowance is chosen.
struct Found {
    cgroup: Option<u64>,
    job: Option<u64>,
    physical: Option<u64>,
    used: u64,
    host: GovernorLevel,
    cpu_time: Duration,
}

fn linux_files(proc: &Path, inputs: &Inputs) -> Found {
    let proc = linux::Proc(proc);
    Found {
        cgroup: proc.cgroup_limit(),
        job: None,
        physical: proc.physical(),
        used: proc.footprint(),
        host: proc.host(inputs.pressure, inputs.critical),
        cpu_time: proc.cpu_time(),
    }
}

#[cfg(target_os = "linux")]
fn os(inputs: &Inputs) -> Found {
    linux_files(Path::new("/proc"), inputs)
}

#[cfg(target_os = "macos")]
fn os(inputs: &Inputs) -> Found {
    let _ = inputs;
    Found {
        cgroup: None,
        job: None,
        physical: macos::physical(),
        used: macos::footprint(),
        host: macos::host(),
        cpu_time: macos::cpu_time(),
    }
}

#[cfg(windows)]
fn os(inputs: &Inputs) -> Found {
    Found {
        cgroup: None,
        job: windows::job_limit(),
        physical: windows::physical(),
        used: windows::footprint(),
        host: windows::host(inputs.pressure, inputs.critical),
        cpu_time: windows::cpu_time(),
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
fn os(inputs: &Inputs) -> Found {
    let _ = inputs;
    Found {
        cgroup: None,
        job: None,
        physical: None,
        used: 0,
        host: GovernorLevel::Normal,
        cpu_time: Duration::ZERO,
    }
}

/// The tightest of the cgroup's limit, the Job object's, the setting and
/// half of physical memory; with none of them, nothing is allowed, and the
/// engine runs at its floors.
fn tightest(
    cgroup: Option<u64>,
    job: Option<u64>,
    setting: Option<u64>,
    physical: Option<u64>,
) -> (u64, AllowanceSource) {
    [
        (cgroup, AllowanceSource::Cgroup),
        (job, AllowanceSource::JobObject),
        (setting, AllowanceSource::Setting),
        (
            physical.map(|physical| physical / 2),
            AllowanceSource::Physical,
        ),
    ]
    .into_iter()
    .filter_map(|(limit, source)| limit.map(|limit| (limit, source)))
    .min_by_key(|(limit, _)| *limit)
    .unwrap_or((0, AllowanceSource::Physical))
}

#[cfg(test)]
mod tests;
