//! What the governor reads of the machine at each sample (ADR 0040): the
//! Memory allowance, the host's own memory pressure, the footprint of the
//! engine and its children and of each browser, the engine's CPU time and
//! the CPUs it may use. A value, never a global: Linux's readings come from
//! a folder laid out as `/proc`, the OS's own by default and a test's
//! otherwise, so every rule is checked on any OS; macOS's and Windows' come
//! from their C calls. Where Linux keeps the engine's limits, its cgroup, is
//! found once, as the governor starts ([`System::locate`]); each sample
//! reads each thing once (ADR 0140's E4).

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use goodfetch_counted_work::CountedWork;
use goodfetch_protocol::governor::{AllowanceSource, GovernorLevel};

use super::storage::{Disks, FilesystemSource};

pub(crate) use linux::Cgroup;

mod cpus;
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(windows)]
mod windows;

/// Where the governor reads the machine, and where it reads the
/// filesystems the Runs write to (ADR 0122).
#[derive(Clone, Debug)]
pub struct System {
    source: Source,
    pub(crate) disks: Disks,
    /// What its counted reads count on: the governor's, once it samples.
    work: CountedWork,
}

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
    /// `governor.host_memory`: whether the host's own memory is judged.
    pub(crate) host_memory: bool,
}

/// What one reading found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SystemReading {
    pub(crate) allowance: u64,
    pub(crate) allowance_source: AllowanceSource,
    pub(crate) used: u64,
    pub(crate) host: GovernorLevel,
    /// The engine's CPU time since it started.
    pub(crate) cpu_time: Duration,
    /// The CPUs the engine may use: its affinity, within its cgroup's CPU
    /// quota, as `available_parallelism` counts them; at least one.
    pub(crate) parallelism: usize,
    /// The memory of each browser asked for, in its order, as
    /// [`footprint_of`] counts it.
    pub(crate) browsers: Vec<u64>,
}

impl System {
    /// The OS the engine runs on, which only `main` and the engine's own
    /// default use.
    pub fn os() -> Self {
        Self {
            source: Source::Os,
            disks: Disks::Os,
            work: CountedWork::default(),
        }
    }

    /// A folder laid out as Linux's `/proc`, whose `self` is the engine,
    /// and whose `self/mountinfo` names where the cgroup file system is.
    pub fn linux_files(proc: PathBuf) -> Self {
        Self {
            source: Source::LinuxFiles(proc),
            disks: Disks::Os,
            work: CountedWork::default(),
        }
    }

    /// The same, reading the filesystems the Runs write to through
    /// `source` in place of the OS's own calls, as it reads the machine
    /// through [`Self::linux_files`]'s folder.
    #[must_use]
    pub fn with_filesystems(mut self, source: Arc<dyn FilesystemSource>) -> Self {
        self.disks = Disks::Chosen(source);
        self
    }

    /// The same, its reads counted on `work` (ADR 0140's E4).
    #[must_use]
    pub(crate) fn counting(mut self, work: CountedWork) -> Self {
        self.work = work;
        self
    }

    /// Finds where the engine's limits are: on Linux, its cgroup, from
    /// `self/cgroup` and `self/mountinfo`; nothing elsewhere. Once, as the
    /// governor starts. Blocking.
    pub(crate) fn locate(&self) -> Cgroup {
        match &self.source {
            Source::LinuxFiles(proc) => linux::Proc {
                folder: proc,
                work: &self.work,
            }
            .cgroup(),
            Source::Os => os_cgroup(&self.work),
        }
    }

    /// Reads the machine now, its limits where `cgroup` says, and the
    /// memory of each process of `browsers` and every descendant. Blocking.
    pub(crate) fn read(&self, cgroup: &Cgroup, inputs: &Inputs, browsers: &[u32]) -> SystemReading {
        let found = match &self.source {
            Source::LinuxFiles(proc) => linux_files(proc, &self.work, cgroup, inputs, browsers),
            Source::Os => os(&self.work, cgroup, inputs, browsers),
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
            host: if inputs.host_memory {
                found.host
            } else {
                GovernorLevel::Normal
            },
            cpu_time: found.cpu_time,
            parallelism: found.parallelism,
            browsers: found.browsers,
        }
    }
}

/// The memory of the process `process` and every descendant, in bytes,
/// as the governor counts the engine's: PSS on Linux, the platform's own
/// footprint elsewhere; nothing for a process that is gone. Blocking.
pub fn footprint_of(process: u32) -> u64 {
    #[cfg(target_os = "linux")]
    {
        // Uncounted: the pool counts its walk where it asks for one.
        linux::Proc {
            folder: Path::new("/proc"),
            work: &CountedWork::default(),
        }
        .footprint_from(&process.to_string())
    }
    #[cfg(target_os = "macos")]
    {
        macos::footprint_from(process)
    }
    #[cfg(windows)]
    {
        windows::footprint_from(process)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
    {
        let _ = process;
        0
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
    parallelism: usize,
    browsers: Vec<u64>,
}

fn linux_files(
    proc: &Path,
    work: &CountedWork,
    cgroup: &Cgroup,
    inputs: &Inputs,
    browsers: &[u32],
) -> Found {
    let proc = linux::Proc { folder: proc, work };
    let meminfo = proc.meminfo();
    let (used, browsers) = proc.footprints(browsers);
    let cpus = cpus::allowed();
    Found {
        cgroup: cgroup.memory_limit(),
        job: None,
        physical: meminfo.total,
        used,
        host: proc.host(meminfo, inputs.pressure, inputs.critical),
        cpu_time: proc.cpu_time(),
        parallelism: cgroup.cpu_quota().map_or(cpus, |quota| {
            cpus.min(usize::try_from(quota).unwrap_or(usize::MAX))
        }),
        browsers,
    }
}

#[cfg(target_os = "linux")]
fn os_cgroup(work: &CountedWork) -> Cgroup {
    linux::Proc {
        folder: Path::new("/proc"),
        work,
    }
    .cgroup()
}

#[cfg(not(target_os = "linux"))]
fn os_cgroup(work: &CountedWork) -> Cgroup {
    let _ = work;
    Cgroup::default()
}

#[cfg(target_os = "linux")]
fn os(work: &CountedWork, cgroup: &Cgroup, inputs: &Inputs, browsers: &[u32]) -> Found {
    linux_files(Path::new("/proc"), work, cgroup, inputs, browsers)
}

#[cfg(target_os = "macos")]
fn os(work: &CountedWork, cgroup: &Cgroup, inputs: &Inputs, browsers: &[u32]) -> Found {
    let _ = (work, cgroup, inputs);
    Found {
        cgroup: None,
        job: None,
        physical: macos::physical(),
        used: macos::footprint(),
        host: macos::host(),
        cpu_time: macos::cpu_time(),
        parallelism: cpus::allowed(),
        browsers: browsers
            .iter()
            .map(|process| footprint_of(*process))
            .collect(),
    }
}

#[cfg(windows)]
fn os(work: &CountedWork, cgroup: &Cgroup, inputs: &Inputs, browsers: &[u32]) -> Found {
    let _ = (work, cgroup);
    Found {
        cgroup: None,
        job: windows::job_limit(),
        physical: windows::physical(),
        used: windows::footprint(),
        host: windows::host(inputs.pressure, inputs.critical),
        cpu_time: windows::cpu_time(),
        parallelism: cpus::allowed(),
        browsers: browsers
            .iter()
            .map(|process| footprint_of(*process))
            .collect(),
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
fn os(work: &CountedWork, cgroup: &Cgroup, inputs: &Inputs, browsers: &[u32]) -> Found {
    let _ = (work, cgroup, inputs);
    Found {
        cgroup: None,
        job: None,
        physical: None,
        used: 0,
        host: GovernorLevel::Normal,
        cpu_time: Duration::ZERO,
        parallelism: cpus::allowed(),
        browsers: vec![0; browsers.len()],
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
