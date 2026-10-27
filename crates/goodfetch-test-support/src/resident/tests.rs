use std::hint::black_box;

use super::{peak_resident_bytes, peak_resident_bytes_of};

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

/// Read by its PID, this process's peak holds memory touched and freed
/// since, as its own reading does.
#[test]
fn the_peak_of_a_process_named_by_its_pid_keeps_memory_touched_and_freed_since() {
    let before = peak_resident_bytes_of(std::process::id()).unwrap();
    let touched = vec![1_u8; usize::try_from(before + 64 * MIB).unwrap()];
    drop(black_box(touched));
    let after = peak_resident_bytes_of(std::process::id()).unwrap();

    assert!(
        after >= before + 64 * MIB,
        "the peak moved from {before} to {after} bytes"
    );
}

/// Another process's peak is its own: a child that only sleeps peaks far
/// below this process once this one has touched 256 MiB, yet above
/// nothing.
#[cfg(unix)]
#[test]
fn the_peak_of_another_process_is_its_own() {
    let touched = vec![1_u8; usize::try_from(256 * MIB).unwrap()];
    drop(black_box(touched));
    let mut child = std::process::Command::new("sleep")
        .arg("30")
        .spawn()
        .unwrap();
    std::thread::sleep(std::time::Duration::from_millis(100));

    let peak = peak_resident_bytes_of(child.id());
    child.kill().unwrap();
    child.wait().unwrap();

    let peak = peak.unwrap();
    assert!(
        (1..64 * MIB).contains(&peak),
        "a sleeping child peaked at {peak} bytes"
    );
}
