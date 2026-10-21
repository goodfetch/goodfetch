//! The writer lock's interface (spec #213's Seam 4): a reader's test that
//! never keeps the lock and creates nothing, and the writer's backoff, a
//! pure function of the attempt and its jitter.
use std::time::Duration;

use goodfetch_test_support::TestEnv;
use proptest::prelude::*;
use rstest::rstest;

use super::{LOCK_FILE, LockError, RETRIES, Writer, backoff, take_lock, test_writer};

/// Covers: story:3-46
#[test]
fn a_readers_test_leaves_a_free_lock_free_for_a_writer() {
    let env = TestEnv::new();
    let folder = env.folder().join("run");
    drop(take_lock(&folder).ok().unwrap());

    let found = test_writer(&folder).unwrap();

    assert_eq!((found, take_lock(&folder).is_ok()), (Writer::None, true));
}

/// Covers: story:3-46
#[test]
fn a_readers_test_finds_a_held_lock_live() {
    let env = TestEnv::new();
    let folder = env.folder().join("run");
    let _writer = take_lock(&folder).ok().unwrap();

    let found = test_writer(&folder).unwrap();

    assert_eq!(
        (found, matches!(take_lock(&folder), Err(LockError::Held))),
        (Writer::Live, true)
    );
}

/// Covers: story:3-46
#[test]
fn a_readers_test_of_a_folder_without_a_lock_file_creates_none() {
    let env = TestEnv::new();
    let folder = env.folder().join("run");

    let found = test_writer(&folder).unwrap();

    assert_eq!(
        (found, folder.join(LOCK_FILE).exists()),
        (Writer::None, false)
    );
}

/// Covers: story:3-46
#[rstest]
#[case::first_at_middle_jitter(1, 500, Duration::from_micros(32_258))]
#[case::last_at_middle_jitter(5, 500, Duration::from_micros(516_129))]
#[case::last_at_least_jitter(5, 0, Duration::from_micros(387_096))]
#[case::last_at_most_jitter(5, 1_000, Duration::from_micros(645_161))]
#[case::jitter_wraps(5, 1_001, Duration::from_micros(387_096))]
fn a_retry_waits_by_its_attempt_and_jitter(
    #[case] attempt: u32,
    #[case] jitter: u64,
    #[case] wait: Duration,
) {
    assert_eq!(backoff(attempt, jitter), wait);
}

proptest! {
    /// The five retries wait about a second in all, between three quarters
    /// of one and five quarters (less what rounding down drops), whatever
    /// the jitter.
    /// Covers: story:3-46
    #[test]
    fn the_retries_wait_about_a_second_in_all(jitters in prop::array::uniform5(any::<u64>())) {
        let total: Duration = (1..=RETRIES)
            .zip(jitters)
            .map(|(attempt, jitter)| backoff(attempt, jitter))
            .sum();

        prop_assert!(
            (Duration::from_millis(749)..=Duration::from_millis(1_250)).contains(&total),
            "{total:?}"
        );
    }

    /// Each retry waits longer than the one before it, whatever either's
    /// jitter.
    /// Covers: story:3-46
    #[test]
    fn each_retry_waits_longer_than_the_one_before(
        attempt in 1..RETRIES,
        before in any::<u64>(),
        after in any::<u64>(),
    ) {
        prop_assert!(backoff(attempt + 1, after) > backoff(attempt, before));
    }
}
