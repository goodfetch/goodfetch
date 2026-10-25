//! The race's delay as a function of its Label on a domain's browser-fetch
//! latencies (ADR 0040).
//!
//! Covers: story:6-263

use std::time::Duration;

use rstest::rstest;

use super::{FIRST_DELAY, KEPT, Latencies, median};

fn ms(milliseconds: u64) -> Duration {
    Duration::from_millis(milliseconds)
}

/// The median by nearest rank is one of the samples: the middle one of an
/// odd count, the lower middle one of an even count; none of no sample.
#[rstest]
#[case::none(&[], None)]
#[case::one(&[700], Some(700))]
#[case::odd(&[900, 100, 300], Some(300))]
#[case::even(&[400, 100, 300, 200], Some(200))]
#[case::repeated(&[50, 50, 900], Some(50))]
fn the_median_is_the_middle_sample_by_nearest_rank(
    #[case] samples: &[u64],
    #[case] expected: Option<u64>,
) {
    assert_eq!(median(samples.iter().copied().map(ms)), expected.map(ms));
}

/// A Label on a domain with no browser-fetch latency yet waits 500 ms
/// before its navigation; then the median of its own latencies, never
/// another Label's or another domain's.
#[test]
fn the_delay_is_500_ms_before_any_sample_then_the_median_of_the_labels_own_on_its_domain() {
    let mut latencies = Latencies::default();
    let before = latencies.delay("list", "a.test");
    latencies.record("list", "a.test", ms(80));
    latencies.record("list", "a.test", ms(1_000));
    latencies.record("list", "a.test", ms(120));
    latencies.record("other", "a.test", ms(5));
    latencies.record("list", "b.test", ms(7));

    assert_eq!(
        (
            before,
            latencies.delay("list", "a.test"),
            latencies.delay("other", "a.test"),
            latencies.delay("list", "b.test"),
            latencies.delay("list", "c.test"),
        ),
        (FIRST_DELAY, ms(120), ms(5), ms(7), ms(500)),
    );
}

/// Only the latest [`KEPT`] latencies count: older ones drop out.
#[test]
fn only_the_latest_latencies_count() {
    let mut latencies = Latencies::default();
    for _ in 0..KEPT {
        latencies.record("list", "a.test", ms(10));
    }
    for _ in 0..=KEPT / 2 {
        latencies.record("list", "a.test", ms(900));
    }

    assert_eq!(latencies.delay("list", "a.test"), ms(900));
}
