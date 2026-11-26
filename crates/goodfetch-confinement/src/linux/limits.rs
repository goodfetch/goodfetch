//! A Plugin's resource limits as rlimits (ADR 0033), which its launcher
//! sets once confined, just before the Plugin starts, so that a namespace
//! mechanism's former self, which waits outside its namespaces, holds none
//! of them. An unprivileged process cannot raise a hard limit, in a user
//! namespace of its own or not.
//!
//! - `memory` is `RLIMIT_DATA`, soft and hard: the private writable memory
//!   the process maps. A runtime's reserved but inaccessible address space
//!   does not count, as it would against `RLIMIT_AS`.
//! - `cpu_time` is `RLIMIT_CPU`, soft, with a hard limit one second past
//!   it: at the soft limit the kernel sends `SIGXCPU`, which ends a Plugin
//!   that does not handle it, and at the hard limit `SIGKILL`.
//! - `open_files` is `RLIMIT_NOFILE`, soft and hard.
//! - `processes` has no rlimit: `RLIMIT_NPROC` counts every thread of the
//!   user, not one instance's processes, so it would stop a runtime's
//!   threads. The `none` level refuses every new process, which holds an
//!   instance to itself; a level that allows one needs the `pids`
//!   controller of a delegated cgroup (issue 3960).
//! - `concurrency` holds calls, which the engine counts.

use std::io;

use goodfetch_protocol::plugin::Limits;
use serde::{Deserialize, Serialize};

/// A limit the launcher sets as an rlimit.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Resource {
    /// `memory`, as `RLIMIT_DATA`.
    Memory,
    /// `cpu_time`, as `RLIMIT_CPU`.
    CpuTime,
    /// `open_files`, as `RLIMIT_NOFILE`.
    OpenFiles,
}

impl Resource {
    const ALL: [Self; 3] = [Self::Memory, Self::CpuTime, Self::OpenFiles];

    /// Its soft and hard limits under `limits`, in the rlimit's unit:
    /// bytes, seconds or files.
    fn limit(self, limits: &Limits) -> Rlimit64 {
        match self {
            Self::Memory => Rlimit64::both(limits.memory),
            Self::CpuTime => Rlimit64 {
                soft: limits.cpu_time,
                hard: limits.cpu_time.saturating_add(1),
            },
            Self::OpenFiles => Rlimit64::both(u64::from(limits.open_files)),
        }
    }

    /// Its rlimit's number, which `prlimit64` takes; glibc and musl type
    /// it differently, and both fit.
    fn number(self) -> i64 {
        i64::from(match self {
            Self::Memory => libc::RLIMIT_DATA,
            Self::CpuTime => libc::RLIMIT_CPU,
            Self::OpenFiles => libc::RLIMIT_NOFILE,
        })
    }
}

/// Sets each [`Resource`] of `limits` on the calling process.
///
/// # Errors
///
/// When the kernel refuses one: a value above a hard limit the process
/// already has.
pub fn limit_self(limits: &Limits) -> io::Result<()> {
    Resource::ALL
        .into_iter()
        .try_for_each(|resource| set(resource.number(), &resource.limit(limits)))
}

/// `prlimit64`'s limit pair.
#[repr(C)]
#[derive(Debug, PartialEq, Eq)]
struct Rlimit64 {
    soft: u64,
    hard: u64,
}

impl Rlimit64 {
    /// `value` as both the soft and the hard limit.
    fn both(value: u64) -> Self {
        Self {
            soft: value,
            hard: value,
        }
    }
}

/// Sets rlimit `number` of the calling process to `limit`, with
/// `prlimit64`, the call the seccomp lists allow.
#[expect(unsafe_code, reason = "setting an rlimit is a raw system call")]
fn set(number: i64, limit: &Rlimit64) -> io::Result<()> {
    // SAFETY: the kernel reads the pair from `limit`, which outlives the
    // call, and writes nothing, as the old limit's pointer is null.
    let result = unsafe {
        libc::syscall(
            libc::SYS_prlimit64,
            0_i64,
            number,
            std::ptr::from_ref(limit),
            std::ptr::null_mut::<Rlimit64>(),
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

#[cfg(test)]
mod tests;
