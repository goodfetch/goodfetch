//! Resource limits on macOS (ADR 0033, issue 867): sets `RLIMIT_CPU` and
//! `RLIMIT_NOFILE` via `seatbelt::set_rlimit`. Memory is not enforced via an
//! rlimit on macOS because an unprivileged process cannot set a per-process
//! physical-footprint cap; the engine's Resource governor (ADR 0040) samples
//! each Plugin instance's footprint instead.

use std::io;

use goodfetch_protocol::plugin::Limits;
use serde::{Deserialize, Serialize};

use super::seatbelt;

/// A resource limit the macOS launcher sets as an rlimit.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Resource {
    /// `cpu_time`, as `RLIMIT_CPU` (soft = `cpu_time`, hard = `cpu_time + 1`).
    CpuTime,
    /// `open_files`, as `RLIMIT_NOFILE` (soft and hard = `open_files`).
    OpenFiles,
}

impl Resource {
    const ALL: [Self; 2] = [Self::CpuTime, Self::OpenFiles];

    fn apply(self, limits: &Limits) -> io::Result<()> {
        match self {
            Self::CpuTime => seatbelt::set_rlimit(
                libc::RLIMIT_CPU,
                limits.cpu_time,
                limits.cpu_time.saturating_add(1),
            ),
            Self::OpenFiles => {
                let open_files = u64::from(limits.open_files);
                seatbelt::set_rlimit(libc::RLIMIT_NOFILE, open_files, open_files)
            }
        }
    }

    /// Sets this resource's soft and hard rlimit to `value` (used by the
    /// confinement probe executable's `Raise` oracle).
    ///
    /// # Errors
    ///
    /// When the kernel refuses the new limit (e.g. `EPERM` past the hard limit).
    pub fn set_both(self, value: u64) -> io::Result<()> {
        let number = match self {
            Self::CpuTime => libc::RLIMIT_CPU,
            Self::OpenFiles => libc::RLIMIT_NOFILE,
        };
        seatbelt::set_rlimit(number, value, value)
    }
}

/// Sets each macOS rlimit of `limits` on the calling process.
///
/// # Errors
///
/// When `setrlimit` fails (e.g. requested limit exceeds the current hard limit).
pub fn limit_self(limits: &Limits) -> io::Result<()> {
    Resource::ALL
        .into_iter()
        .try_for_each(|resource| resource.apply(limits))
}
