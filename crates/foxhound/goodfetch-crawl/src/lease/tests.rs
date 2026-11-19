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
use goodfetch_engine::deadline::Deadline;
use goodfetch_engine::lease::{LeaseCheck as _, LeaseEnded};
use goodfetch_protocol::connection::WaitedFor;
use goodfetch_protocol::foxhound::crawl::{ATTEMPT_ENDED, TIMEOUT_HANDLER};
use goodfetch_protocol::jsonrpc::RequestId;
use goodfetch_protocol::settings::TIMEOUT_ACTION;
use proptest::prelude::*;
use serde_json::{Value, json};
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

/// A page call made inside a crawl handler ends at the handler's own end,
/// its `timeout.handler` (ADR 0080), not at its own 30 s `timeout.action`,
/// and its `deadline_exceeded` names the handler call as `ended_by`.
///
/// Covers: story:11-41, story:11-43
#[test]
fn a_handler_page_call_ends_at_the_handlers_timeout_and_names_it() {
    on_paused_clock(async |leases, clock| {
        let start = clock.now();
        let lease = leases.grant("1", 1, DEFAULT_LEASE);
        lease.handler_sent(RequestId::Number(4));
        let handler = lease
            .handler(&Deadline::declared(&TIMEOUT_HANDLER))
            .expect("a handler call was sent");
        clock.sleep(Duration::from_secs(55)).await;

        let ended = handler
            .bound(
                &clock,
                Deadline::declared(&TIMEOUT_ACTION).within(
                    &clock,
                    WaitedFor::BROWSER_ANSWER,
                    std::future::pending::<()>(),
                ),
                || Some("a navigation under way on page `p-1`".to_owned()),
            )
            .await;

        let error = ended.expect_err("the handler's end came first");
        assert_eq!(
            (
                clock.now().duration_since(start),
                error.code(),
                error.details().cloned().map(Value::Object)
            ),
            (
                Duration::from_secs(60),
                "deadline_exceeded",
                Some(json!({
                    "limit": { "key": "timeout.handler", "value": 60_000, "layer": "default" },
                    "elapsed": 5_000,
                    "waited_for": "answer",
                    "ended_by": { "operation": "crawl_request.handle", "call_id": 4 },
                    "observed": "a navigation under way on page `p-1`",
                }))
            )
        );
    });
}

/// Before the handler call is sent, and in a code-free crawl, which sends
/// none, nothing encloses the attempt's page calls.
///
/// Covers: story:11-41
#[test]
fn no_handler_call_sent_encloses_nothing() {
    on_paused_clock(async |leases, _clock| {
        let lease = leases.grant("1", 1, DEFAULT_LEASE);

        assert!(
            lease
                .handler(&Deadline::declared(&TIMEOUT_HANDLER))
                .is_none()
        );
    });
}

/// A handler's wait for a navigation-gate permit renews its lease (ADR
/// 0093), and the renewal moves the handler's end later, so the wait does
/// not count toward `timeout.handler`.
///
/// Covers: story:11-41
#[test]
fn a_renewal_during_a_permit_wait_moves_the_handlers_end_later() {
    on_paused_clock(async |leases, clock| {
        let lease = leases.grant("1", 1, DEFAULT_LEASE);
        lease.handler_sent(RequestId::Number(4));
        let handler = lease
            .handler(&Deadline::declared(&TIMEOUT_HANDLER))
            .expect("a handler call was sent");
        let waiting_for_a_permit = async {
            clock.sleep(Duration::from_secs(50)).await;
            lease.renew(DEFAULT_LEASE);
        };
        let answered = async {
            clock.sleep(Duration::from_secs(100)).await;
            Ok("clicked")
        };

        let ((), answer) = tokio::join!(
            waiting_for_a_permit,
            handler.bound(&clock, answered, || None)
        );

        assert_eq!(answer.ok(), Some("clicked"));
    });
}
