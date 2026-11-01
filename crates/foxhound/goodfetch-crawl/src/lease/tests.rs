//! A lease's rules as pure functions of a monotonic reading, with no clock;
//! the waiting loop of an attempt's lease in process on the paused `Clock`
//! with a default-length lease (ADR 0059); and the model-based test of
//! leases and commits on real SQLite (`model`).
#![expect(
    clippy::disallowed_methods,
    reason = "tests read the test Clock (ADR 0059)"
)]

mod model;

use std::time::Duration;

use goodfetch_engine::clock::Clock;
use goodfetch_engine::lease::{LeaseCheck as _, LeaseEnded};
use goodfetch_protocol::foxhound::crawl::ATTEMPT_ENDED;
use proptest::prelude::*;
use tokio::time::Instant;

use super::{Lease, Leases};

/// The lease of the settings' default 60 s handler: 60 s plus 10 s.
const DEFAULT_LEASE: Duration = Duration::from_secs(70);

/// Some monotonic moment, from the test `Clock`; the rules take every
/// reading as an argument.
fn origin() -> Instant {
    goodfetch_test_support::runtime().block_on(async { Clock::paused(std::time::UNIX_EPOCH).now() })
}

proptest! {
    #![proptest_config(goodfetch_test_support::proptest_config())]

    /// Covers: story:3-40
    #[test]
    fn a_lease_holds_until_its_length_has_passed(length in 1_u64..1_000_000, after in 0_u64..2_000_000) {
        let start = origin();
        let lease = Lease::granted(1, start, Duration::from_millis(length));

        prop_assert_eq!(lease.holds_at(start + Duration::from_millis(after)), after < length);
    }

    /// Covers: story:3-40
    #[test]
    fn a_renewal_never_shortens_a_lease(length in 1_u64..1_000_000, at in 0_u64..1_000_000, renewal in 0_u64..1_000_000) {
        let start = origin();
        let lease = Lease::granted(1, start, Duration::from_millis(length));

        let renewed = lease.renewed(start + Duration::from_millis(at), Duration::from_millis(renewal));

        prop_assert!(renewed.expires_at() >= lease.expires_at());
        prop_assert_eq!(renewed.epoch(), lease.epoch());
    }

    /// Covers: story:3-40
    #[test]
    fn a_renewal_never_revives_a_lease_that_ended(length in 1_u64..1_000_000, late in 0_u64..1_000_000, renewal in 1_u64..1_000_000) {
        let start = origin();
        let lease = Lease::granted(1, start, Duration::from_millis(length));
        let at = start + Duration::from_millis(length + late);

        let renewed = lease.renewed(at, Duration::from_millis(renewal));

        prop_assert!(!renewed.holds_at(at));
    }
}

/// Runs `test` on the paused clock with the leases of a Run `waiting`.
fn on_paused_clock(test: impl AsyncFnOnce(Leases, Clock)) {
    goodfetch_test_support::runtime().block_on(async {
        let clock = Clock::paused(std::time::UNIX_EPOCH);
        test(Leases::new(clock, "waiting", 1), clock).await;
    });
}

/// The lease check a handler's page passes before each command (#720,
/// #761) refuses once the lease ran out.
///
/// Covers: story:3-40, story:3-42
#[test]
fn a_default_lease_ends_seventy_seconds_after_its_grant() {
    on_paused_clock(async |leases, clock| {
        let start = clock.now();
        let lease = leases.grant("1", 1, DEFAULT_LEASE);

        let ended = lease.ended().await;

        assert_eq!(
            (ended, clock.now().duration_since(start), lease.check()),
            (LeaseEnded::Lost, DEFAULT_LEASE, Err(LeaseEnded::Lost))
        );
    });
}

/// Covers: story:3-40
#[test]
fn a_renewal_while_the_attempt_waits_lengthens_its_wait() {
    on_paused_clock(async |leases, clock| {
        let start = clock.now();
        let lease = leases.grant("1", 1, DEFAULT_LEASE);
        let renewing = async {
            clock.sleep(Duration::from_secs(60)).await;
            lease.renew(DEFAULT_LEASE);
        };

        let (ended, ()) = tokio::join!(lease.ended(), renewing);

        assert_eq!(
            (ended, clock.now().duration_since(start)),
            (LeaseEnded::Lost, Duration::from_secs(130))
        );
    });
}

/// The lease check refuses once the attempt released its lease.
///
/// Covers: story:3-40, story:3-42
#[test]
fn a_released_lease_ends_its_wait_and_lets_nothing_more_leave() {
    on_paused_clock(async |leases, clock| {
        let start = clock.now();
        let lease = leases.grant("1", 1, DEFAULT_LEASE);
        let releasing = async {
            clock.sleep(Duration::from_secs(5)).await;
            lease.release();
        };

        let (ended, ()) = tokio::join!(lease.ended(), releasing);

        assert_eq!(
            (ended, clock.now().duration_since(start), lease.check()),
            (
                LeaseEnded::Released,
                Duration::from_secs(5),
                Err(LeaseEnded::Released)
            )
        );
    });
}

/// A call whose page withheld a command once the lease ended raises
/// Foxhound's `attempt_ended`, which the page, Collie's, never names.
///
/// Covers: story:3-42
#[test]
fn a_command_withheld_after_the_lease_ended_raises_attempt_ended() {
    on_paused_clock(async |leases, _clock| {
        let lease = leases.grant("1", 1, DEFAULT_LEASE);
        lease.release();

        let error = lease.withheld(LeaseEnded::Released, "Page.navigate");

        assert_eq!(
            (error.code(), error.retryable()),
            (ATTEMPT_ENDED.name, ATTEMPT_ENDED.retryable)
        );
    });
}
