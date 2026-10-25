use std::hint::black_box;

use super::peak_resident_bytes;

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
