use std::hint::black_box;

use super::{peak_resident_bytes, private_resident_bytes_of};

const MIB: u64 = 1024 * 1024;

/// The peak holds memory the process touched and freed since: after as
/// many bytes as the peak so far, and 64 MiB more, are written and dropped,
/// it is at least 64 MiB above where it was, whatever the process held
/// when it was first reached.
#[test]
fn the_peak_keeps_memory_touched_and_freed_since() {
    let before = peak_resident_bytes().unwrap();
    let touched = vec![1_u8; usize::try_from(before + 64 * MIB).unwrap()];
    drop(black_box(touched));
    let after = peak_resident_bytes().unwrap();

    assert!(
        after >= before + 64 * MIB,
        "the peak moved from {before} to {after} bytes"
    );
}

/// A process's private memory resident now, read by its PID, holds what
/// it touched and keeps, and lets go of what it freed, unlike its peak.
#[test]
fn the_private_memory_of_a_process_holds_what_it_keeps_and_not_what_it_freed() {
    let before = private_resident_bytes_of(std::process::id()).unwrap();
    let touched = vec![1_u8; usize::try_from(256 * MIB).unwrap()];
    let held = private_resident_bytes_of(std::process::id()).unwrap();
    drop(black_box(touched));
    let after = private_resident_bytes_of(std::process::id()).unwrap();

    assert!(
        held >= before + 240 * MIB && after + 128 * MIB <= held,
        "private memory went from {before} to {held} bytes holding 256 MiB, then {after} once they were freed"
    );
}
