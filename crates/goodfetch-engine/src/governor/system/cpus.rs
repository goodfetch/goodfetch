//! The CPUs the engine may run on now: on Linux, its affinity mask, read
//! with one `sched_getaffinity` call and no file; elsewhere, and should
//! that call fail, `available_parallelism`. The cgroup's CPU quota, which
//! `available_parallelism` reads too, is the `linux` module's, from the
//! folders the governor finds once as it starts (ADR 0140's E4).
#![cfg_attr(
    target_os = "linux",
    expect(
        unsafe_code,
        reason = "sched_getaffinity is a C call, as std's available_parallelism makes it"
    )
)]

/// The CPUs the engine may run on now, at least one.
pub(super) fn allowed() -> usize {
    #[cfg(target_os = "linux")]
    if let Some(count) = affinity() {
        return count;
    }
    std::thread::available_parallelism().map_or(1, usize::from)
}

/// The CPUs in the engine's affinity mask, none when the call fails, as it
/// does on a machine with more CPUs than one mask holds.
#[cfg(target_os = "linux")]
fn affinity() -> Option<usize> {
    // SAFETY: `cpu_set_t` is a plain bit array, valid all zeros.
    let mut set: libc::cpu_set_t = unsafe { std::mem::zeroed() };
    // SAFETY: the pointer and size describe `set`, which outlives the call;
    // pid 0 is the calling thread.
    let answer =
        unsafe { libc::sched_getaffinity(0, std::mem::size_of::<libc::cpu_set_t>(), &raw mut set) };
    if answer != 0 {
        return None;
    }
    // SAFETY: `set` is the mask the kernel just filled.
    let count = unsafe { libc::CPU_COUNT(&set) };
    usize::try_from(count).ok().filter(|count| *count > 0)
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_engine_may_run_on_at_least_the_cpus_std_counts() {
        let std = std::thread::available_parallelism().map_or(1, usize::from);
        assert!(super::allowed() >= std);
    }
}
