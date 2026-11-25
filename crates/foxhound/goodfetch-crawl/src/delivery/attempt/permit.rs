//! The Permit of a request whose crawl runs a before-request Hook, taken
//! once those Hooks returned (#2905; the owner's decision of 2026-11-07 on
//! #1546): the attempt holds its lease while its Hooks run, then asks its
//! Run state for its domain's Permit, a second round trip a crawl with no
//! such Hook never makes, and sends nothing before it is granted.

use std::sync::OnceLock;

use goodfetch_engine::admission::{PermitAnswer, Wakes};
use goodfetch_engine::lease::PermitWait;
use goodfetch_protocol::foxhound::crawl::RESPONSE_UNAVAILABLE;

use super::super::failure::unsent;
use super::{Answer, AttemptFailure, On};
use crate::run_state::Asking;

/// How an attempt asks for its request's Permit once its before-request
/// Hooks returned, and whether it asked already: a run on another tier
/// asks no more.
pub(super) struct Permitting {
    asking: Asking,
    asked: OnceLock<()>,
}

impl Permitting {
    /// Asking through `asking`, not asked yet.
    pub(super) fn through(asking: Asking) -> Self {
        Self {
            asking,
            asked: OnceLock::new(),
        }
    }
}

impl On<'_> {
    /// Its request's Permit, asked once per attempt, when its crawl takes
    /// each Permit after the request's before-request Hooks: `None` once
    /// granted, its reserved send kept for the request's first send, or
    /// when it takes none here; else how the attempt ended meanwhile. While
    /// it waits its lease is held as through a navigation gate's permit
    /// wait (ADR 0093), and its request shows waiting for a Permit; a
    /// Permit another request gives back on its key asks again at once,
    /// else it asks again at the time the answer named. A site whose
    /// `Retry-After` reaches further than the Run waits fails it unsent
    /// (ADR 0118).
    pub(super) async fn permitted(&self) -> Option<Answer> {
        let Permitting { asking, asked } = self.permit?;
        if asked.set(()).is_err() {
            return None;
        }
        let (clock, leased, lease) = (self.run.identity.clock(), self.leased, self.lease);
        let (id, key) = (&leased.waiting.id, &leased.waiting.key);
        let mut waiting: Option<PermitWait> = None;
        loop {
            let answer = tokio::select! {
                biased;
                _ = lease.ended() => return Some(Answer::LeaseEnded),
                answer = asking.permit(id.clone(), key.clone()) => answer,
            };
            let (until, wakes) = match answer {
                Ok(PermitAnswer::Granted(send_at)) => {
                    self.hand_over.reserved_send.reserve(send_at);
                    return None;
                }
                Ok(PermitAnswer::Refused(until)) => {
                    let failure = unsent(until);
                    return Some(Answer::failed(AttemptFailure::entered(
                        failure.code,
                        RESPONSE_UNAVAILABLE.retryable,
                        failure.message,
                        failure.details,
                    )));
                }
                Ok(PermitAnswer::Wait { until, wakes }) => (until, wakes),
                // The Run state ended: the crawl is closing.
                Err(_) => return Some(Answer::LeaseEnded),
            };
            if waiting.is_none() {
                waiting = Some(self.hand_over.lease.permit_wait(&leased.crawl_request.url));
            }
            tokio::select! {
                biased;
                _ = lease.ended() => return Some(Answer::LeaseEnded),
                () = clock.sleep_until(until) => {}
                () = woken(wakes, key) => {}
            }
        }
    }
}

/// Returns once `wakes` names `key`, or fell behind; never without
/// `wakes`.
async fn woken(wakes: Option<Wakes>, key: &str) {
    let Some(mut wakes) = wakes else {
        return std::future::pending().await;
    };
    while let Some(woke) = wakes.next().await {
        if woke == key {
            return;
        }
    }
}
