//! macOS's readings, through its C calls (ADR 0040): physical memory and
//! the kernel's memory-pressure level from `sysctl`, the physical footprint
//! of the engine and every descendant from `proc_pid_rusage`, the platform's
//! own measure of what a process costs, and the engine's CPU time from
//! `getrusage`. No cgroup and no Job object limit a macOS process. System
//! calls that Miri cannot run: the tests on a real macOS cover them
//! (STYLE.md, `unsafe`).
#![expect(
    unsafe_code,
    reason = "macOS gives memory pressure, footprints and CPU time only through its C calls (ADR 0040)"
)]

use std::ffi::{CStr, c_int, c_void};
use std::time::Duration;

use goodfetch_protocol::governor::GovernorLevel;

/// Physical memory, in bytes.
pub(super) fn physical() -> Option<u64> {
    sysctl::<u64>(c"hw.memsize")
}

/// The kernel's memory-pressure level: 2 warns, 4 is critical.
pub(super) fn host() -> GovernorLevel {
    match sysctl::<c_int>(c"kern.memorystatus_vm_pressure_level") {
        Some(4) => GovernorLevel::Critical,
        Some(2) => GovernorLevel::Pressure,
        _ => GovernorLevel::Normal,
    }
}

/// The physical footprint of the engine and every descendant, in bytes; a
/// process that ended meanwhile counts nothing.
pub(super) fn footprint() -> u64 {
    footprint_from(std::process::id())
}

/// The physical footprint of `root` and every descendant, in bytes.
pub(super) fn footprint_from(root: u32) -> u64 {
    let mut found = vec![root];
    let mut total = 0;
    while let Some(process) = found.pop() {
        let Ok(pid) = c_int::try_from(process) else {
            continue;
        };
        total += phys_footprint(pid);
        found.extend(children(pid));
    }
    total
}

/// The engine's user and system CPU time.
pub(super) fn cpu_time() -> Duration {
    let mut usage = std::mem::MaybeUninit::<libc::rusage>::zeroed();
    // SAFETY: `usage` is a writable `rusage` the call fills.
    let done = unsafe { libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) };
    if done != 0 {
        return Duration::ZERO;
    }
    // SAFETY: the call succeeded, so it filled `usage`.
    let usage = unsafe { usage.assume_init() };
    let time = |value: libc::timeval| {
        Duration::from_secs(u64::try_from(value.tv_sec).unwrap_or(0))
            + Duration::from_micros(u64::try_from(value.tv_usec).unwrap_or(0))
    };
    time(usage.ru_utime) + time(usage.ru_stime)
}

fn phys_footprint(pid: c_int) -> u64 {
    let mut info = std::mem::MaybeUninit::<libc::rusage_info_v2>::zeroed();
    // SAFETY: `info` is a writable `rusage_info_v2`, the flavour asked for.
    let done = unsafe {
        libc::proc_pid_rusage(
            pid,
            libc::RUSAGE_INFO_V2,
            info.as_mut_ptr().cast::<libc::rusage_info_t>(),
        )
    };
    if done != 0 {
        return 0;
    }
    // SAFETY: the call succeeded, so it filled `info`.
    unsafe { info.assume_init() }.ri_phys_footprint
}

fn children(pid: c_int) -> Vec<u32> {
    let mut pids = vec![0 as libc::pid_t; 256];
    loop {
        let Ok(size) = c_int::try_from(pids.len() * size_of::<libc::pid_t>()) else {
            return Vec::new();
        };
        // SAFETY: `pids` holds `size` writable bytes.
        let count =
            unsafe { libc::proc_listchildpids(pid, pids.as_mut_ptr().cast::<c_void>(), size) };
        let Ok(count) = usize::try_from(count) else {
            return Vec::new();
        };
        if count < pids.len() {
            return pids[..count]
                .iter()
                .filter_map(|child| u32::try_from(*child).ok())
                .collect();
        }
        pids.resize(pids.len() * 2, 0);
    }
}

fn sysctl<T: Copy + Default>(name: &CStr) -> Option<T> {
    let mut value = T::default();
    let mut size = size_of::<T>();
    // SAFETY: `value` holds `size` writable bytes, and `name` ends with a
    // NUL.
    let done = unsafe {
        libc::sysctlbyname(
            name.as_ptr(),
            (&raw mut value).cast::<c_void>(),
            &raw mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    (done == 0 && size == size_of::<T>()).then_some(value)
}
