//! The free-space sweep's rules, with no clock and no file system: when a
//! file's times prove no build read it since a moment under `relatime`, and
//! the order and the stop of what goes under the line.

use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use rstest::rstest;

use super::{Candidate, Swept, Times, least_read_first};

/// A moment, `hours` after an arbitrary origin.
fn at(hours: u64) -> SystemTime {
    SystemTime::UNIX_EPOCH + Duration::from_hours(1_000 + hours)
}

fn times(accessed: u64, modified: u64, changed: u64) -> Times {
    Times {
        accessed: at(accessed),
        modified: at(modified),
        changed: at(changed),
    }
}

#[rstest]
// Re-armed at 10 and unread since: unread since any moment after 10.
#[case::re_armed_and_unread_since(times(5, 1, 10), 12, true)]
// Read at 11, after its change at 10.
#[case::read_after_its_last_change(times(11, 1, 10), 12, false)]
// Re-armed at 13, after the moment: a read at 12 may be unrecorded.
#[case::re_armed_after_the_moment(times(5, 1, 13), 12, false)]
// Written at 13 by a build: changed, and in use, after the moment.
#[case::written_after_the_moment(times(5, 13, 13), 12, false)]
// Read and never re-armed: `relatime` records a read a day late at most.
#[case::read_more_than_a_day_before(times(10, 1, 2), 40, true)]
#[case::read_within_the_day_before(times(20, 1, 2), 40, false)]
fn a_file_is_unread_since_a_moment_only_when_its_times_prove_it(
    #[case] file: Times,
    #[case] since: u64,
    #[case] unread: bool,
) {
    assert_eq!(file.unread_since(at(since)), unread);
}

#[rstest]
#[case::read_since_its_last_change(times(11, 1, 10), true)]
#[case::unread_since_its_last_change(times(5, 1, 10), false)]
fn a_file_is_re_armed_when_a_read_since_its_last_change_left_it(
    #[case] file: Times,
    #[case] rearm: bool,
) {
    assert_eq!(file.needs_rearm(), rearm);
}

fn candidate(last: u64, target: usize, name: &str) -> Candidate {
    Candidate {
        last: at(last),
        target,
        paths: vec![PathBuf::from(name)],
        unit: None,
    }
}

/// Least recently read first, and only while its target's file system is
/// under the line: each removal here frees 10 bytes of a file system with 75
/// free and a line of 100, so three go, and the other target, over its line,
/// loses nothing.
#[test]
fn the_least_recently_read_go_first_until_their_file_system_is_over_the_line() {
    let mut candidates = vec![
        candidate(4, 0, "fourth"),
        candidate(1, 0, "first"),
        candidate(2, 1, "elsewhere"),
        candidate(3, 0, "third"),
        candidate(2, 0, "second"),
    ];
    let mut swept = [Swept::default(), Swept::default()];
    let mut taken = Vec::new();

    least_read_first(
        &mut candidates,
        |target, swept| Ok(target == 0 && 75 + swept[0].bytes < 100),
        |candidate, swept| {
            swept[candidate.target].bytes += 10;
            taken.push(candidate.paths[0].clone());
            Ok(())
        },
        &mut swept,
    )
    .unwrap();

    assert_eq!(
        taken,
        ["first", "second", "third"].map(PathBuf::from).to_vec()
    );
}
