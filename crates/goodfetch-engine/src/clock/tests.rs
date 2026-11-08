//! The test `Clock`: its wall time starts where the test says and moves
//! with Tokio's paused clock, so it is only made on the runtime it runs on.
#![expect(
    clippy::disallowed_methods,
    reason = "tests run on the test Clock (ADR 0059)"
)]

use std::time::{Duration, SystemTime};

use super::Clock;

/// 2026-01-05T09:00:00Z.
fn start() -> SystemTime {
    SystemTime::UNIX_EPOCH + Duration::from_hours(491_001)
}

#[test]
fn a_paused_clocks_wall_time_starts_at_its_start_and_moves_with_the_paused_clock() {
    let walls = goodfetch_test_support::runtime().block_on(async {
        let clock = Clock::paused(start());
        let first = clock.wall();
        clock.sleep(Duration::from_secs(60)).await;
        (first, clock.wall())
    });

    assert_eq!(walls, (start(), start() + Duration::from_secs(60)));
}

/// Made before its runtime, a paused Clock would count the real time until
/// the runtime is built as wall time, a few milliseconds under load.
#[test]
#[should_panic(expected = "a paused Clock is made on the runtime it runs on")]
fn a_paused_clock_is_refused_off_a_runtime() {
    Clock::paused(start());
}

/// The Audit log, the Usage ledger and the known certificates write their
/// times at the clock's own precision.
#[test]
fn a_clocks_timestamp_is_its_wall_time_to_the_nanosecond() {
    let timestamp = goodfetch_test_support::runtime().block_on(async {
        Clock::paused(start() + Duration::from_nanos(1_500_000_001)).timestamp()
    });

    assert_eq!(timestamp.to_string(), "2026-01-05T09:00:01.500000001Z");
}

/// A Run's points in time and "Needs you" write theirs to the millisecond,
/// a half rounding up.
#[test]
fn a_clocks_point_in_time_is_its_wall_time_to_the_millisecond() {
    let points = goodfetch_test_support::runtime().block_on(async {
        let clock = Clock::paused(start() + Duration::from_micros(1_500));
        let now = clock.point_in_time();
        let after = clock.point_in_time_after(Duration::from_micros(499));
        (now, after)
    });

    assert_eq!(
        (points.0.as_str(), points.1.as_str()),
        ("2026-01-05T09:00:00.002Z", "2026-01-05T09:00:00.002Z")
    );
}
