//! Eviction order as the pure function it is: the least recently used
//! entries leave first, the fewest that bring the bytes held within the
//! cap, a body several entries share counted once and freed with the last
//! of them; and the index's times, whose text sorts as they do.

use std::collections::BTreeMap;
use std::time::{Duration, SystemTime};

use proptest::prelude::*;
use rstest::rstest;

use super::{Weighed, evictions, index_time};

/// The bytes `entries` hold, each body once.
fn held(entries: &[(u8, u64)]) -> u64 {
    let bodies: BTreeMap<u8, u64> = entries.iter().copied().collect();
    bodies.values().sum()
}

/// `entries`, (body, size) in least recently used order, weighed as the
/// index counts their bodies' holders.
fn weighed(entries: &[(u8, u64)]) -> Vec<Weighed> {
    let mut holders: BTreeMap<u8, u64> = BTreeMap::new();
    for (body, _) in entries {
        *holders.entry(*body).or_default() += 1;
    }
    entries
        .iter()
        .map(|(body, size)| Weighed {
            body: body.to_string(),
            size: *size,
            holders: holders[body],
        })
        .collect()
}

/// Entries of up to 8 bodies, each body of one size.
fn entries() -> impl Strategy<Value = Vec<(u8, u64)>> {
    (
        prop::collection::vec(1_u64..1_000, 8),
        prop::collection::vec(0_u8..8, 0..24),
    )
        .prop_map(|(sizes, bodies)| {
            bodies
                .into_iter()
                .map(|body| (body, sizes[usize::from(body)]))
                .collect()
        })
}

proptest! {
    /// The entries left hold the cap at most, unless none is left, and
    /// keeping one more of those that left would pass it.
    #[test]
    fn the_fewest_least_recently_used_leave_to_bring_the_bytes_within_the_cap(
        entries in entries(),
        cap in 0_u64..4_000,
    ) {
        let taken = evictions(held(&entries), cap, &weighed(&entries));

        let kept = &entries[taken..];
        prop_assert!(held(kept) <= cap || kept.is_empty());
        if taken > 0 {
            prop_assert!(held(&entries[taken - 1..]) > cap);
        }
    }

    /// Read a page at a time, as the cache reads its oldest entries, the
    /// same entries leave as when read whole.
    #[test]
    fn eviction_read_a_page_at_a_time_takes_the_same_entries(
        entries in entries(),
        cap in 0_u64..4_000,
        page in 1_usize..6,
    ) {
        let whole = evictions(held(&entries), cap, &weighed(&entries));

        let mut taken = 0;
        loop {
            let left = &entries[taken..];
            let read = &weighed(left)[..page.min(left.len())];
            let more = evictions(held(left), cap, read);
            taken += more;
            if more < read.len() || read.is_empty() {
                break;
            }
        }
        prop_assert_eq!(taken, whole);
    }

    /// The index's times sort as text as they do in time, to the
    /// millisecond, whatever their fraction of a second.
    #[test]
    fn index_times_sort_as_their_times(
        first in 0_u64..4_102_444_800_000,
        second in 0_u64..4_102_444_800_000,
    ) {
        let time = |millis| SystemTime::UNIX_EPOCH + Duration::from_millis(millis);

        prop_assert_eq!(
            index_time(time(first)).cmp(&index_time(time(second))),
            first.cmp(&second)
        );
    }
}

#[rstest]
#[case::within_the_cap_none(&[(0, 100), (1, 100)], 200, 0)]
#[case::one_over_the_oldest(&[(0, 100), (1, 100), (2, 100)], 200, 1)]
#[case::a_shared_body_frees_with_its_last_holder(&[(0, 100), (0, 100), (1, 100)], 100, 2)]
#[case::a_body_over_the_cap_leaves_everything(&[(0, 100), (1, 500)], 400, 2)]
#[case::nothing_held(&[], 0, 0)]
fn eviction_takes_the_oldest_first(
    #[case] entries: &[(u8, u64)],
    #[case] cap: u64,
    #[case] taken: usize,
) {
    assert_eq!(evictions(held(entries), cap, &weighed(entries)), taken);
}
