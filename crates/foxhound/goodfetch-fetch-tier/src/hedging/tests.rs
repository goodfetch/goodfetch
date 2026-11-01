//! The redundant requests' rules through their interface (spec #213's
//! Seam 4, its module interfaces; pure, `proptest`): which requests are
//! repeated, a route's threshold and a Run's hedge budget.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use std::time::Duration;

use proptest::prelude::*;
use rstest::rstest;

use super::{Budget, Latencies, repeatable};

/// Only a GET or a HEAD from a work item that declares no Effect, and
/// never a Paid fetch, is hedged or coalesced (ADRs 0040, 0045 and 0106).
///
/// Covers: story:3-63, story:3-64
#[rstest]
#[case::get(http::Method::GET, false, false, true)]
#[case::head(http::Method::HEAD, false, false, true)]
#[case::post(http::Method::POST, false, false, false)]
#[case::put(http::Method::PUT, false, false, false)]
#[case::options(http::Method::OPTIONS, false, false, false)]
#[case::get_of_a_work_item_that_declares_an_effect(http::Method::GET, true, false, false)]
#[case::head_of_a_work_item_that_declares_an_effect(http::Method::HEAD, true, false, false)]
#[case::paid_get(http::Method::GET, false, true, false)]
fn only_a_get_or_head_that_runs_no_effect_and_pays_nothing_is_repeated(
    #[case] method: http::Method,
    #[case] declares_effect: bool,
    #[case] paid: bool,
    #[case] expected: bool,
) {
    assert_eq!(repeatable(&method, declares_effect, paid), expected);
}

fn milliseconds(latencies: impl IntoIterator<Item = u64>) -> Latencies {
    let mut recorded = Latencies::default();
    for latency in latencies {
        recorded.record("page@shop.test", Duration::from_millis(latency));
    }
    recorded
}

/// A route hedges nothing until twenty of its requests answered, then
/// after the 95th percentile of its latencies by nearest rank; another
/// route's latencies say nothing of it.
///
/// Covers: story:3-63
#[rstest]
#[case::nineteen(1..=19, "page@shop.test", None)]
#[case::twenty(1..=20, "page@shop.test", Some(19))]
#[case::a_hundred(1..=100, "page@shop.test", Some(95))]
#[case::another_route(1..=100, "page@other.test", None)]
fn a_route_hedges_after_the_95th_percentile_of_its_latencies(
    #[case] latencies: std::ops::RangeInclusive<u64>,
    #[case] route: &str,
    #[case] expected: Option<u64>,
) {
    let recorded = milliseconds(latencies);

    assert_eq!(
        recorded.threshold(route),
        expected.map(Duration::from_millis)
    );
}

/// Only a route's latest 128 latencies count: a site that slowed down is
/// hedged after its new latencies, not its old ones.
///
/// Covers: story:3-63
#[test]
fn a_route_s_threshold_follows_its_latest_latencies() {
    let recorded = milliseconds((0..200).map(|index| if index < 72 { 1_000 } else { 10 }));

    assert_eq!(
        recorded.threshold("page@shop.test"),
        Some(Duration::from_millis(10))
    );
}

/// At the default 5 %, a Run that sent 19 requests may send no hedge, and
/// one that sent 20 may send one, then none until it sent 40.
///
/// Covers: story:3-63, setting:fetch.hedge.max_percent
#[test]
fn a_run_hedges_at_most_five_percent_of_its_requests() {
    let mut budget = Budget::new(5);
    let mut spent = Vec::new();
    for sent in 1..=40 {
        budget.sent();
        if budget.spend() {
            spent.push(sent);
        }
    }

    assert_eq!(spent, [20, 40]);
}

proptest! {
    #![proptest_config(goodfetch_test_support::proptest_config())]

    /// Whatever the share and however requests and hedges interleave,
    /// the hedges spent never pass the share of the requests sent.
    ///
    /// Covers: story:3-63, setting:fetch.hedge.max_percent
    #[test]
    fn hedges_never_pass_their_share_of_the_requests(
        max_percent in 0_u64..=100,
        steps in proptest::collection::vec(any::<bool>(), 1..500),
    ) {
        let mut budget = Budget::new(max_percent);
        let (mut requests, mut hedges) = (0_u64, 0_u64);
        for sends in steps {
            if sends {
                budget.sent();
                requests += 1;
            } else if budget.spend() {
                hedges += 1;
            }
            prop_assert!(hedges * 100 <= max_percent * requests, "{hedges} hedges for {requests} requests at {max_percent} %");
        }
    }

    /// A route's threshold is one of its latest latencies, and at least
    /// 95 % of them are no longer.
    ///
    /// Covers: story:3-63
    #[test]
    fn a_threshold_is_a_latency_no_shorter_than_95_percent_of_them(
        latencies in proptest::collection::vec(0_u64..=5_000, 20..300),
    ) {
        let recorded = milliseconds(latencies.iter().copied());
        let latest = &latencies[latencies.len().saturating_sub(128)..];
        let threshold = recorded.threshold("page@shop.test").unwrap();
        let at_most = latest
            .iter()
            .filter(|latency| Duration::from_millis(**latency) <= threshold)
            .count();
        prop_assert!(latest.iter().any(|latency| Duration::from_millis(*latency) == threshold));
        prop_assert!(at_most * 100 >= latest.len() * 95, "{at_most} of {} at most {threshold:?}", latest.len());
    }
}
