//! Slice sizing through its interface (spec #213's Seam 4): a pure function
//! of the Limit, what is left and the live runs, checked by named cases from
//! ADR 0041's rule and by properties over every input.
//!
//! Covers: story:3-69

use proptest::prelude::*;
use rstest::rstest;

use super::{SliceSizing, slice_size};

#[rstest]
#[case::one_percent_while_plenty_is_left(10_000, 10_000, 1, 100)]
#[case::the_remainder_over_twice_the_live_runs_once_it_is_smaller(10_000, 1_000, 10, 50)]
#[case::no_smaller_than_a_thousandth(10_000, 30, 1, 15)]
#[case::the_minimum_when_the_even_share_is_below_it(10_000, 100, 20, 10)]
#[case::the_whole_remainder_below_the_minimum(10_000, 8, 3, 8)]
#[case::one_unit_of_a_small_limit(50, 50, 1, 1)]
#[case::nothing_when_nothing_is_left(10_000, 0, 1, 0)]
fn a_slice_is_the_smaller_of_one_percent_and_the_remainder_over_twice_the_live_runs(
    #[case] amount: u64,
    #[case] left: u64,
    #[case] live_runs: u64,
    #[case] expected: u64,
) {
    assert_eq!(
        slice_size(SliceSizing::default(), amount, left, live_runs),
        expected
    );
}

fn limit_and_left() -> impl Strategy<Value = (u64, u64)> {
    (1_u64..10_000_000).prop_flat_map(|amount| (Just(amount), 0..=amount))
}

proptest! {
    #![proptest_config(goodfetch_test_support::proptest_config())]

    #[test]
    fn a_slice_never_takes_more_than_is_left(
        (amount, left) in limit_and_left(),
        live_runs in 1_u64..1_000,
    ) {
        prop_assert!(slice_size(SliceSizing::default(), amount, left, live_runs) <= left);
    }

    #[test]
    fn a_slice_is_empty_only_when_nothing_is_left(
        (amount, left) in limit_and_left(),
        live_runs in 1_u64..1_000,
    ) {
        let size = slice_size(SliceSizing::default(), amount, left, live_runs);
        prop_assert_eq!(size == 0, left == 0);
    }

    #[test]
    fn a_slice_never_exceeds_one_percent_unless_it_is_the_minimum(
        (amount, left) in limit_and_left(),
        live_runs in 1_u64..1_000,
    ) {
        let size = slice_size(SliceSizing::default(), amount, left, live_runs);
        let minimum = amount.div_ceil(1000).max(1);
        prop_assert!(size <= (amount / 100).max(minimum));
    }

    #[test]
    fn a_remainder_below_the_minimum_goes_whole_to_the_asker(
        amount in 1_000_u64..10_000_000,
        live_runs in 1_u64..1_000,
        below in 1_u64..1_000,
    ) {
        let minimum = amount.div_ceil(1000);
        let left = below % minimum;
        prop_assert_eq!(slice_size(SliceSizing::default(), amount, left, live_runs), left);
    }

    #[test]
    fn more_live_runs_never_get_a_bigger_slice(
        (amount, left) in limit_and_left(),
        live_runs in 1_u64..1_000,
    ) {
        let sizing = SliceSizing::default();
        prop_assert!(
            slice_size(sizing, amount, left, live_runs + 1) <= slice_size(sizing, amount, left, live_runs)
        );
    }
}
