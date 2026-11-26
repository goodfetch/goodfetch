//! A page's permit waits on the paused test `Clock` (ADR 0059): what a
//! deadline leaves out of its count, without real time.
#![expect(
    clippy::disallowed_methods,
    reason = "tests read the test Clock (ADR 0059)"
)]

use std::time::Duration;

use goodfetch_engine::clock::Clock;

use super::PermitWaits;

/// Runs `test` on a paused clock.
fn on_paused_clock(test: impl AsyncFnOnce(Clock)) {
    goodfetch_test_support::runtime().block_on(async {
        test(Clock::paused(std::time::UNIX_EPOCH)).await;
    });
}

const LIMIT: Duration = Duration::from_secs(1);

/// With no wait under way, a deadline passes at its limit.
#[test]
fn with_no_wait_a_deadline_passes_at_its_limit() {
    on_paused_clock(async |clock| {
        let waits = PermitWaits::new(clock);
        let start = clock.now();

        let outcome = waits
            .within(LIMIT, clock.sleep(Duration::from_secs(5)))
            .await;

        assert_eq!(
            (outcome.is_err(), clock.now().duration_since(start)),
            (true, LIMIT)
        );
    });
}

/// The time a wait is under way counts against no deadline: a deadline of
/// 1 s with a wait of 3 s under way passes 4 s after it started, and work
/// that ends within that answers.
#[test]
fn a_wait_under_way_moves_the_deadline_by_as_long() {
    on_paused_clock(async |clock| {
        let waits = PermitWaits::new(clock);
        let start = clock.now();
        let waiting = waits.begin();
        let ending = async {
            clock.sleep(Duration::from_secs(3)).await;
            drop(waiting);
        };

        let (answered, ()) = tokio::join!(
            waits.within(LIMIT, async {
                clock.sleep(Duration::from_millis(3_500)).await;
                "answered"
            }),
            ending
        );
        let passed = waits
            .within(LIMIT, clock.sleep(Duration::from_secs(5)))
            .await;

        assert_eq!(
            (
                answered.ok(),
                passed.is_err(),
                clock.now().duration_since(start)
            ),
            (Some("answered"), true, Duration::from_millis(3_500) + LIMIT)
        );
    });
}

/// While a wait is under way, no deadline passes, however long it lasts.
#[test]
fn no_deadline_passes_while_a_wait_is_under_way() {
    on_paused_clock(async |clock| {
        let waits = PermitWaits::new(clock);
        let waiting = waits.begin();

        let answered = waits
            .within(LIMIT, async {
                clock.sleep(Duration::from_secs(600)).await;
                "answered"
            })
            .await;
        drop(waiting);

        assert_eq!(answered.ok(), Some("answered"));
    });
}
