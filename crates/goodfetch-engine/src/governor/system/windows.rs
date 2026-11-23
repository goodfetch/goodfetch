//! Windows' readings, through its C calls (ADR 0040): the memory limit of
//! the Job object the engine runs in, physical memory and the memory in use,
//! the low-memory notification, the private bytes of the engine and every
//! descendant, the platform's own measure of what a process costs, and the
//! engine's CPU time. System calls that Miri cannot run: the tests on a real
//! Windows cover them (STYLE.md, `unsafe`).
#![expect(
    unsafe_code,
    reason = "Windows gives Job object limits, memory notifications, footprints and CPU time only through its C calls (ADR 0040)"
)]

use std::collections::BTreeMap;
use std::time::Duration;

use goodfetch_protocol::governor::GovernorLevel;
use windows_sys::Win32::Foundation::{CloseHandle, FALSE, FILETIME, HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
};
use windows_sys::Win32::System::JobObjects::{
    JOB_OBJECT_LIMIT_JOB_MEMORY, JOB_OBJECT_LIMIT_PROCESS_MEMORY,
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
    QueryInformationJobObject,
};
use windows_sys::Win32::System::Memory::{
    CreateMemoryResourceNotification, LowMemoryResourceNotification,
    QueryMemoryResourceNotification,
};
use windows_sys::Win32::System::ProcessStatus::{
    GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX,
};
use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
use windows_sys::Win32::System::Threading::{
    GetCurrentProcess, GetCurrentProcessId, GetProcessTimes, OpenProcess,
    PROCESS_QUERY_LIMITED_INFORMATION,
};

use super::readings;

/// The memory limit of the Job object the engine runs in, per process or
/// for the whole job, whichever is tighter.
pub(super) fn job_limit() -> Option<u64> {
    // SAFETY: an all-zero JOBOBJECT_EXTENDED_LIMIT_INFORMATION is valid.
    let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
    let size = u32::try_from(size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>()).ok()?;
    // SAFETY: a null handle asks for the job of the calling process, and
    // `limits` holds `size` writable bytes.
    let done = unsafe {
        QueryInformationJobObject(
            std::ptr::null_mut(),
            JobObjectExtendedLimitInformation,
            (&raw mut limits).cast(),
            size,
            std::ptr::null_mut(),
        )
    };
    if done == FALSE {
        return None;
    }
    let flags = limits.BasicLimitInformation.LimitFlags;
    let process =
        (flags & JOB_OBJECT_LIMIT_PROCESS_MEMORY != 0).then_some(limits.ProcessMemoryLimit);
    let job = (flags & JOB_OBJECT_LIMIT_JOB_MEMORY != 0).then_some(limits.JobMemoryLimit);
    [process, job]
        .into_iter()
        .flatten()
        .filter_map(|limit| u64::try_from(limit).ok())
        .min()
}

fn memory_status() -> Option<MEMORYSTATUSEX> {
    // SAFETY: an all-zero MEMORYSTATUSEX is valid.
    let mut status: MEMORYSTATUSEX = unsafe { std::mem::zeroed() };
    status.dwLength = u32::try_from(size_of::<MEMORYSTATUSEX>()).ok()?;
    // SAFETY: `status` is a writable MEMORYSTATUSEX whose length is set.
    let done = unsafe { GlobalMemoryStatusEx(&raw mut status) };
    (done != FALSE).then_some(status)
}

/// Physical memory, in bytes.
pub(super) fn physical() -> Option<u64> {
    memory_status().map(|status| status.ullTotalPhys)
}

/// The host's own memory pressure, from the memory in use and the
/// low-memory notification, as [`readings::windows_host`] judges them.
pub(super) fn host(pressure: u64, critical: u64) -> GovernorLevel {
    let load = memory_status().map_or(0, |status| u64::from(status.dwMemoryLoad));
    readings::windows_host(load, low_memory(), pressure, critical)
}

fn low_memory() -> bool {
    // SAFETY: no pointer argument.
    let notification = unsafe { CreateMemoryResourceNotification(LowMemoryResourceNotification) };
    if notification.is_null() {
        return false;
    }
    let mut low = FALSE;
    // SAFETY: `notification` is the handle just made, and `low` is a
    // writable BOOL.
    let done = unsafe { QueryMemoryResourceNotification(notification, &raw mut low) };
    close(notification);
    done != FALSE && low != FALSE
}

/// The private bytes of the engine and every descendant; a process that
/// ended meanwhile counts nothing.
pub(super) fn footprint() -> u64 {
    // SAFETY: no argument.
    footprint_from(unsafe { GetCurrentProcessId() })
}

/// The private bytes of `root` and every descendant.
pub(super) fn footprint_from(root: u32) -> u64 {
    tree(root).into_iter().map(private_bytes).sum()
}

/// `root` and every descendant, from one snapshot of every process's
/// parent.
pub(super) fn tree(root: u32) -> Vec<u32> {
    let parents = parents();
    readings::tree(root, |process| {
        parents
            .iter()
            .filter(|(_, parent)| **parent == process)
            .map(|(child, _)| *child)
            .collect()
    })
}

/// Each process's parent, from a snapshot of every process.
fn parents() -> BTreeMap<u32, u32> {
    let mut parents = BTreeMap::new();
    // SAFETY: no pointer argument.
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if snapshot == INVALID_HANDLE_VALUE {
        return parents;
    }
    // SAFETY: an all-zero PROCESSENTRY32W is valid.
    let mut entry: PROCESSENTRY32W = unsafe { std::mem::zeroed() };
    entry.dwSize = u32::try_from(size_of::<PROCESSENTRY32W>()).unwrap_or(0);
    // SAFETY: `snapshot` is the handle just made, and `entry` a writable
    // PROCESSENTRY32W whose size is set.
    let mut more = unsafe { Process32FirstW(snapshot, &raw mut entry) } != FALSE;
    while more {
        parents.insert(entry.th32ProcessID, entry.th32ParentProcessID);
        // SAFETY: as for the first.
        more = unsafe { Process32NextW(snapshot, &raw mut entry) } != FALSE;
    }
    close(snapshot);
    parents
}

fn private_bytes(process: u32) -> u64 {
    // SAFETY: no pointer argument.
    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, FALSE, process) };
    if handle.is_null() {
        return 0;
    }
    // SAFETY: an all-zero PROCESS_MEMORY_COUNTERS_EX is valid.
    let mut counters: PROCESS_MEMORY_COUNTERS_EX = unsafe { std::mem::zeroed() };
    let size = u32::try_from(size_of::<PROCESS_MEMORY_COUNTERS_EX>()).unwrap_or(0);
    // SAFETY: `handle` is the process just opened, and `counters` holds
    // `size` writable bytes, a PROCESS_MEMORY_COUNTERS_EX starting with a
    // PROCESS_MEMORY_COUNTERS.
    let done = unsafe {
        GetProcessMemoryInfo(
            handle,
            (&raw mut counters).cast::<PROCESS_MEMORY_COUNTERS>(),
            size,
        )
    };
    close(handle);
    if done == FALSE {
        return 0;
    }
    u64::try_from(counters.PrivateUsage).unwrap_or(0)
}

/// The engine's user and kernel CPU time.
pub(super) fn cpu_time() -> Duration {
    let zero = FILETIME {
        dwLowDateTime: 0,
        dwHighDateTime: 0,
    };
    let (mut created, mut exited, mut kernel, mut user) = (zero, zero, zero, zero);
    // SAFETY: the current process's pseudo-handle, and four writable
    // FILETIMEs.
    let done = unsafe {
        GetProcessTimes(
            GetCurrentProcess(),
            &raw mut created,
            &raw mut exited,
            &raw mut kernel,
            &raw mut user,
        )
    };
    if done == FALSE {
        return Duration::ZERO;
    }
    let hundreds =
        |time: FILETIME| (u64::from(time.dwHighDateTime) << 32) | u64::from(time.dwLowDateTime);
    Duration::from_nanos((hundreds(kernel) + hundreds(user)).saturating_mul(100))
}

fn close(handle: HANDLE) {
    // SAFETY: `handle` is one this module opened and closes once.
    unsafe {
        CloseHandle(handle);
    }
}

#[cfg(test)]
mod tests {
    use goodfetch_protocol::governor::AllowanceSource;
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, SetInformationJobObject,
    };

    use super::super::{Inputs, System};
    use super::*;

    /// A Job object's memory limit tighter than half of physical memory,
    /// with no setting, is the Memory allowance (#2357). Nextest runs each
    /// test in its own process, so the limit holds this test alone.
    ///
    /// Covers: story:3-56
    #[test]
    fn a_job_objects_memory_limit_tighter_than_the_others_is_the_allowance() {
        let limit = physical().unwrap() / 4;
        // SAFETY: no pointer argument but nulls, which ask for an unnamed
        // job with default security.
        let job = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        assert!(!job.is_null());
        // SAFETY: an all-zero JOBOBJECT_EXTENDED_LIMIT_INFORMATION is valid.
        let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_PROCESS_MEMORY;
        limits.ProcessMemoryLimit = usize::try_from(limit).unwrap();
        let size = u32::try_from(size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>()).unwrap();
        // SAFETY: `job` is the handle just made, and `limits` holds `size`
        // readable bytes of the class named.
        let set = unsafe {
            SetInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                (&raw const limits).cast(),
                size,
            )
        };
        // SAFETY: `job` is the handle just made, and the current process's
        // pseudo-handle needs no closing.
        let assigned = unsafe { AssignProcessToJobObject(job, GetCurrentProcess()) };
        let system = System::os();
        let inputs = Inputs {
            memory_allowance: None,
            pressure: 85,
            critical: 95,
            host_memory: false,
        };

        let reading = system.read(&system.locate(), &inputs, &[]);
        close(job);

        assert_eq!(
            (
                set != FALSE,
                assigned != FALSE,
                reading.allowance,
                reading.allowance_source
            ),
            (true, true, limit, AllowanceSource::JobObject)
        );
    }
}
