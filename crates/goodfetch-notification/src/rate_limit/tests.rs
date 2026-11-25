//! The rate limit through its interface, on the paused `Clock`: concurrent
//! senders to one destination go five at once, then one a second, while
//! another destination keeps its own turns, and a quiet spell gives the
//! burst back.
#![expect(
    clippy::disallowed_methods,
    reason = "tests run on the test Clock (ADR 0059)"
)]

use std::time::{Duration, UNIX_EPOCH};

use futures_util::future::join_all;
use goodfetch_engine::clock::Clock;

use super::{BURST, INTERVAL, RateLimit};

/// The seconds after the start at which each of `senders` concurrent
/// senders to `destination` takes its turn, in the order they asked.
async fn turns(limit: &RateLimit, clock: Clock, senders: &[&str]) -> Vec<(String, u64)> {
    let start = clock.now();
    join_all(senders.iter().map(|destination| async move {
        limit.wait_turn(destination).await;
        (
            (*destination).to_owned(),
            clock.now().duration_since(start).as_secs(),
        )
    }))
    .await
}

/// Covers: story:11-270
#[test]
fn concurrent_sends_to_one_destination_go_five_at_once_then_one_a_second() {
    goodfetch_test_support::runtime().block_on(async {
        let clock = Clock::paused(UNIX_EPOCH);
        let limit = RateLimit::new(clock);
        let mut senders = vec!["hooks.example"; 8];
        senders.push("other.example");

        let taken = turns(&limit, clock, &senders).await;

        let seconds = |name: &str| -> Vec<u64> {
            taken
                .iter()
                .filter(|(destination, _)| destination == name)
                .map(|(_, at)| *at)
                .collect()
        };
        assert_eq!(seconds("hooks.example"), [0, 0, 0, 0, 0, 1, 2, 3]);
        assert_eq!(seconds("other.example"), [0]);
    });
}

/// Covers: story:11-270
#[test]
fn a_quiet_spell_gives_the_burst_back() {
    goodfetch_test_support::runtime().block_on(async {
        let clock = Clock::paused(UNIX_EPOCH);
        let limit = RateLimit::new(clock);
        let burst = vec!["hooks.example"; usize::try_from(BURST).unwrap() + 1];
        turns(&limit, clock, &burst).await;

        clock.sleep(INTERVAL * BURST + Duration::from_secs(1)).await;
        let taken = turns(&limit, clock, &burst).await;

        let seconds: Vec<u64> = taken.into_iter().map(|(_, at)| at).collect();
        assert_eq!(seconds, [0, 0, 0, 0, 0, 1]);
    });
}
