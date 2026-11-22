#![expect(
    clippy::disallowed_methods,
    reason = "tests run on the test Clock (ADR 0059)"
)]

use std::sync::Arc;
use std::time::{Duration, UNIX_EPOCH};

use goodfetch_protocol::run::{RunId, StopReason};

use super::{Parts, RunIdentity, StopAsker};
use crate::clock::Clock;
use crate::metrics::Metrics;
use crate::tasks::{BlockingPolicy, TaskOwner};

/// An identity and what asks it to stop, made on the runtime its paused
/// clock runs on.
fn identity() -> (RunIdentity, StopAsker) {
    RunIdentity::new(Parts {
        id: RunId::parse("identity").unwrap(),
        folder: "runs/identity".into(),
        clock: Clock::paused(UNIX_EPOCH),
        settings: None,
        metrics: Metrics::new(&opentelemetry::global::meter("run-identity-tests")),
        span: tracing::Span::none(),
        owner: Arc::new(TaskOwner::root(BlockingPolicy::Inline)),
    })
}

/// A stop asked at a safe point is read by every clone of the identity,
/// one taken before the ask included, and the first reason asked stays.
#[test]
fn a_stop_asked_at_a_safe_point_is_read_by_every_clone_with_its_first_reason() {
    let (stopping, woken, later) = goodfetch_test_support::runtime().block_on(async {
        let (identity, asker) = identity();
        let earlier = identity.clone();
        let (woken, ()) = tokio::join!(earlier.stopping(), async {
            tokio::task::yield_now().await;
            asker.at_safe_point(StopReason::Timeout);
            asker.clone().at_safe_point(StopReason::Pause);
        });
        (earlier.is_stopping(), woken, identity.stopping().await)
    });

    assert!(stopping);
    assert_eq!((woken, later), (StopReason::Timeout, StopReason::Timeout));
}

/// A stop kept at once wakes nothing that waits for a safe point, and a
/// later read finds its reason.
#[test]
fn a_stop_kept_at_once_wakes_no_waiter_and_a_later_read_finds_it() {
    let (woken, later) = goodfetch_test_support::runtime().block_on(async {
        let (identity, asker) = identity();
        let waiting = async {
            tokio::join!(identity.stopping(), async {
                tokio::task::yield_now().await;
                asker.at_once(StopReason::Migrate);
            })
        };
        let woken = tokio::time::timeout(Duration::from_secs(1), waiting).await;
        (woken.is_ok(), identity.stopping().await)
    });

    assert_eq!((woken, later), (false, StopReason::Migrate));
}

/// No stop is asked of a new identity.
#[test]
fn a_new_identity_is_not_stopping() {
    let stopping = goodfetch_test_support::runtime().block_on(async {
        let (identity, _asker) = identity();
        identity.is_stopping()
    });

    assert!(!stopping);
}
