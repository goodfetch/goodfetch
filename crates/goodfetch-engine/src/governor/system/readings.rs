//! What macOS's and Windows' readings mean, apart from the C calls that
//! make them (ADR 0040), so each rule is checked on any OS, as Linux's are
//! through their `/proc` folder: the host's own pressure from each OS's
//! signal, and the tree of processes whose memory the footprint counts.

use std::collections::BTreeSet;

use goodfetch_protocol::governor::GovernorLevel;

/// macOS's kernel memory-pressure level, from
/// `kern.memorystatus_vm_pressure_level`: 2 warns, 4 is critical; any other
/// level, or none read, is normal.
#[cfg(any(target_os = "macos", test))]
pub(super) fn macos_host(level: Option<i32>) -> GovernorLevel {
    match level {
        Some(4) => GovernorLevel::Critical,
        Some(2) => GovernorLevel::Pressure,
        _ => GovernorLevel::Normal,
    }
}

/// Windows' own memory pressure: the low-memory notification is critical,
/// and the memory in use, `load` percent, past `pressure` or `critical`
/// percent counts too.
#[cfg(any(windows, test))]
pub(super) fn windows_host(
    load: u64,
    low_memory: bool,
    pressure: u64,
    critical: u64,
) -> GovernorLevel {
    let in_use = if load >= critical {
        GovernorLevel::Critical
    } else if load >= pressure {
        GovernorLevel::Pressure
    } else {
        GovernorLevel::Normal
    };
    if low_memory {
        GovernorLevel::Critical
    } else {
        in_use
    }
}

/// `root` and every process under it, as `children` lists each one's, each
/// once: a process ID the OS reused can make a process its own ancestor.
pub(super) fn tree(root: u32, mut children: impl FnMut(u32) -> Vec<u32>) -> Vec<u32> {
    let mut seen = BTreeSet::new();
    let mut found = vec![root];
    let mut tree = Vec::new();
    while let Some(process) = found.pop() {
        if seen.insert(process) {
            tree.push(process);
            found.extend(children(process));
        }
    }
    tree
}
