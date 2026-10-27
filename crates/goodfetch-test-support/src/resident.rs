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

/// The peak resident memory so far of the running process `process`, such
/// as a Daemon the test started, in bytes: on Linux its `VmHWM`, on macOS
/// its lifetime maximum physical footprint, on Windows its peak working
/// set.
///
/// # Errors
///
/// When the process is gone or the OS refuses to read it.
#[cfg(target_os = "linux")]
#[expect(
    clippy::disallowed_methods,
    reason = "a test reads another process's status file outside any async worker"
)]
pub fn peak_resident_bytes_of(process: u32) -> io::Result<u64> {
    let status = std::fs::read_to_string(format!("/proc/{process}/status"))?;
    let kib = status
        .lines()
        .find_map(|line| line.strip_prefix("VmHWM:"))
        .and_then(|value| value.trim().strip_suffix("kB"))
        .and_then(|value| value.trim().parse::<u64>().ok())
        .ok_or_else(|| io::Error::other(format!("/proc/{process}/status names no VmHWM")))?;
    Ok(kib * 1024)
}

/// The peak resident memory so far of the running process `process`, such
/// as a Daemon the test started, in bytes: on Linux its `VmHWM`, on macOS
/// its lifetime maximum physical footprint, on Windows its peak working
/// set.
///
/// # Errors
///
/// When the process is gone or the OS refuses to read it.
#[cfg(target_os = "macos")]
pub fn peak_resident_bytes_of(process: u32) -> io::Result<u64> {
    let pid = libc::c_int::try_from(process).map_err(io::Error::other)?;
    // SAFETY: `rusage_info_v4` is plain data, for which all zeroes is a
    // valid value.
    let mut usage: libc::rusage_info_v4 = unsafe { std::mem::zeroed() };
    // SAFETY: `usage` is a valid, writable `rusage_info_v4`, the struct the
    // V4 flavour fills, and it outlives the call.
    let read = unsafe {
        libc::proc_pid_rusage(
            pid,
            libc::RUSAGE_INFO_V4,
            (&raw mut usage).cast::<libc::rusage_info_t>(),
        )
    };
    if read != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(usage.ri_lifetime_max_phys_footprint)
}

/// The peak resident memory so far of the running process `process`, such
/// as a Daemon the test started, in bytes: on Linux its `VmHWM`, on macOS
/// its lifetime maximum physical footprint, on Windows its peak working
/// set.
///
/// # Errors
///
/// When the process is gone or the OS refuses to read it.
#[cfg(windows)]
pub fn peak_resident_bytes_of(process: u32) -> io::Result<u64> {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::ProcessStatus::{
        GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS,
    };
    use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};

    // SAFETY: opening a process by its ID takes no pointer; a null handle
    // is checked below.
    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, process) };
    if handle.is_null() {
        return Err(io::Error::last_os_error());
    }
    let mut counters = PROCESS_MEMORY_COUNTERS::default();
    let size = u32::try_from(size_of::<PROCESS_MEMORY_COUNTERS>()).map_err(io::Error::other)?;
    // SAFETY: `handle` is open with the query right, and `counters` is a
    // valid, writable struct of the size passed.
    let read = unsafe { GetProcessMemoryInfo(handle, &raw mut counters, size) };
    let error = io::Error::last_os_error();
    // SAFETY: `handle` was opened above and is closed once.
    unsafe { CloseHandle(handle) };
    if read == 0 {
        return Err(error);
    }
    u64::try_from(counters.PeakWorkingSetSize).map_err(io::Error::other)
}

#[cfg(test)]
mod tests;
