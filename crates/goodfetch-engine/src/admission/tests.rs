//! A Run's rate cap through its interface (spec #213's Seam 4): a pure
//! function of the monotonic readings the tests give it.
#![expect(
    clippy::disallowed_methods,
    reason = "a pure rule's readings may start from any moment (ADR 0059)"
)]

use std::time::Duration;

use proptest::prelude::*;
use tokio::time::Instant;

use super::RateCap;

/// Offers `cap` a request at each of `offsets` from `start`, in order, and
/// answers the offsets it admitted.
fn admitted(cap: &mut RateCap, start: Instant, offsets: &[Duration]) -> Vec<Duration> {
    offsets
        .iter()
        .filter(|offset| cap.admit(start + **offset).is_ok())
        .copied()
        .collect()
}

/// Covers: story:3-51
#[test]
fn sixty_a_minute_admit_one_request_a_second() {
    let start = Instant::now();
    let mut cap = RateCap::per_minute(60);
    let offsets: Vec<Duration> = (0..=6)
        .map(|tenth| Duration::from_millis(tenth * 500))
        .collect();

    let admitted = admitted(&mut cap, start, &offsets);

    assert_eq!(
        admitted,
        [0, 1_000, 2_000, 3_000].map(Duration::from_millis)
    );
}

/// Covers: story:3-51
#[test]
fn a_refused_request_learns_when_the_cap_admits_again() {
    let start = Instant::now();
    let mut cap = RateCap::per_minute(4);
    cap.admit(start).unwrap();

    let refused = cap.admit(start + Duration::from_secs(1));

    assert_eq!(refused, Err(Some(start + Duration::from_secs(15))));
}

/// Covers: story:3-51
#[test]
fn a_cap_of_zero_admits_nothing() {
    let start = Instant::now();
    let mut cap = RateCap::per_minute(0);

    assert_eq!((cap.admit(start), cap.ready_at(start)), (Err(None), None));
}

/// Covers: story:3-51
#[test]
fn a_tightened_cap_spaces_the_next_admission_by_its_own_interval() {
    let start = Instant::now();
    let mut cap = RateCap::per_minute(60);
    cap.admit(start).unwrap();

    let mut tightened = cap.set_to(6);

    assert_eq!(
        (
            tightened.admit(start + Duration::from_secs(1)).is_ok(),
            tightened.admit(start + Duration::from_secs(10)).is_ok(),
        ),
        (true, false)
    );
}

proptest! {
    /// However requests arrive, no minute admits more than the cap, and a
    /// request offered once the cap is ready is admitted.
    ///
    /// Covers: story:3-51
    #[test]
    fn no_minute_admits_more_than_the_cap(
        per_minute in 1_u64..=600,
        gaps in proptest::collection::vec(0_u64..=2_000, 1..400),
    ) {
        let start = Instant::now();
        let mut cap = RateCap::per_minute(per_minute);
        let mut at = start;
        let mut admitted = Vec::new();
        for gap in gaps {
            at += Duration::from_millis(gap);
            let ready = cap.ready_at(at).unwrap();
            let outcome = cap.admit(at);
            prop_assert_eq!(outcome.is_ok(), ready <= at);
            if outcome.is_ok() {
                admitted.push(at);
            }
        }
        let limit = usize::try_from(per_minute).unwrap();
        for (index, first) in admitted.iter().enumerate() {
            let in_minute = admitted[index..]
                .iter()
                .take_while(|moment| **moment < *first + Duration::from_mins(1))
                .count();
            prop_assert!(in_minute <= limit, "{in_minute} admitted in a minute under a cap of {per_minute}");
        }
    }
}
