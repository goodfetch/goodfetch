//! The peak reader: a child's peak resident memory, as the OS records it.
//! On Linux and macOS, `getrusage`'s maximum resident set of the waited-for
//! children, so it is the child's own only when the process started no
//! other child: `xtask rss probe` runs each child in a process of its own.
//! On Windows, the child's peak working set, which `GetProcessMemoryInfo`
//! still reads after it exits, while its handle stays open. Beside the
//! peak, the executable's own file-backed resident pages, their highest
//! sample while it ran ([`super::executable`]), on Linux.
//!
//! System calls Miri cannot run; tests on each OS cover them (STYLE.md,
//! `unsafe`).
#![expect(
    unsafe_code,
    reason = "the OS's resident-memory figures are read through its C interface: getrusage, GetProcessMemoryInfo"
)]

use std::process::{Child, Command};

use anyhow::{Context as _, Result, bail};

use super::executable;

/// What [`of`] measured of one child.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Resident {
    /// The peak resident memory, in bytes.
    pub(crate) peak_bytes: u64,
    /// The executable's own file-backed resident pages, in bytes: the
    /// highest sample while it ran; none on an OS without `/proc`.
    pub(crate) executable_bytes: Option<u64>,
}

/// Starts `command`, lets `drive` run it, waits for it to exit and returns
/// its peak resident memory in bytes, with its executable's own pages. A child that exits with a failure is
/// an error, since what it measured is not the scenario's. When `drive`
/// fails, the child is killed first: its figure is no scenario's, and a
/// child such as a keep-alive Daemon never ends by itself.
pub(crate) fn of(
    command: &mut Command,
    drive: impl FnOnce(&mut Child) -> Result<()>,
) -> Result<Resident> {
    let mut child = command.spawn().context("starting the measured command")?;
    let sampler = executable::Sampler::start(child.id());
    let driven = drive(&mut child);
    if driven.is_err() {
        // A child that already exited cannot be killed; its wait follows.
        let _ = child.kill();
    }
    let status = child.wait().context("waiting for the measured command");
    let executable_bytes = sampler.finish();
    let status = status?;
    driven?;
    if !status.success() {
        bail!("the measured command exited with {status}");
    }
    Ok(Resident {
        peak_bytes: read(&child)?,
        executable_bytes,
    })
}

#[cfg(unix)]
fn read(_child: &Child) -> Result<u64> {
    // SAFETY: `rusage` is plain data, for which all zeroes is a valid value.
    let mut usage: libc::rusage = unsafe { std::mem::zeroed() };
    // SAFETY: `usage` is a valid, writable `rusage` that outlives the call.
    if unsafe { libc::getrusage(libc::RUSAGE_CHILDREN, &raw mut usage) } != 0 {
        return Err(std::io::Error::last_os_error()).context("reading getrusage");
    }
    let maximum = u64::try_from(usage.ru_maxrss).context("a resident set is never negative")?;
    // macOS counts the maximum resident set in bytes, Linux in KiB.
    Ok(if cfg!(target_os = "macos") {
        maximum
    } else {
        maximum * 1024
    })
}

#[cfg(windows)]
fn read(child: &Child) -> Result<u64> {
    use std::os::windows::io::AsRawHandle as _;

    use windows_sys::Win32::System::ProcessStatus::{
        GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS,
    };

    let mut counters = PROCESS_MEMORY_COUNTERS::default();
    let size = u32::try_from(std::mem::size_of::<PROCESS_MEMORY_COUNTERS>())
        .expect("invariant: the counters' size fits a u32");
    // SAFETY: the handle is the child's, open until `child` drops, and
    // `counters` is a valid, writable struct of the size passed.
    if unsafe { GetProcessMemoryInfo(child.as_raw_handle(), &raw mut counters, size) } == 0 {
        return Err(std::io::Error::last_os_error()).context("reading GetProcessMemoryInfo");
    }
    Ok(u64::try_from(counters.PeakWorkingSetSize).expect("invariant: a usize fits a u64"))
}
