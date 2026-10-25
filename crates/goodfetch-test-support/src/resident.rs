//! This process's peak resident memory, as the OS records it, for a test
//! that bounds growth by a ratio of two peaks taken in turn (ADR 0134, the
//! Frontier at scale). On Linux and macOS, `getrusage`'s maximum resident
//! set of the calling process; on Windows, its peak working set. nextest
//! runs each test in a process of its own, so the peak is the test's.
//!
//! System calls Miri cannot run; tests on each OS cover them (STYLE.md,
//! `unsafe`).
#![expect(
    unsafe_code,
    reason = "the OS's resident-memory figures are read through its C interface: getrusage, GetProcessMemoryInfo"
)]

use std::io;

/// This process's peak resident memory so far, in bytes.
///
/// # Errors
///
/// When the OS refuses to read it.
#[cfg(unix)]
pub fn peak_resident_bytes() -> io::Result<u64> {
    // SAFETY: `rusage` is plain data, for which all zeroes is a valid value.
    let mut usage: libc::rusage = unsafe { std::mem::zeroed() };
    // SAFETY: `usage` is a valid, writable `rusage` that outlives the call.
    if unsafe { libc::getrusage(libc::RUSAGE_SELF, &raw mut usage) } != 0 {
        return Err(io::Error::last_os_error());
    }
    let maximum = u64::try_from(usage.ru_maxrss).map_err(io::Error::other)?;
    // macOS counts the maximum resident set in bytes, Linux in KiB.
    Ok(if cfg!(target_os = "macos") {
        maximum
    } else {
        maximum * 1024
    })
}

/// This process's peak resident memory so far, in bytes.
///
/// # Errors
///
/// When the OS refuses to read it.
#[cfg(windows)]
pub fn peak_resident_bytes() -> io::Result<u64> {
    use windows_sys::Win32::System::ProcessStatus::{
        GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS,
    };
    use windows_sys::Win32::System::Threading::GetCurrentProcess;

    let mut counters = PROCESS_MEMORY_COUNTERS::default();
    let size = u32::try_from(size_of::<PROCESS_MEMORY_COUNTERS>()).map_err(io::Error::other)?;
    // SAFETY: the pseudo-handle of the current process needs no closing,
    // and `counters` is a valid, writable struct of the size passed.
    if unsafe { GetProcessMemoryInfo(GetCurrentProcess(), &raw mut counters, size) } == 0 {
        return Err(io::Error::last_os_error());
    }
    u64::try_from(counters.PeakWorkingSetSize).map_err(io::Error::other)
}

#[cfg(test)]
mod tests;
