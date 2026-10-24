//! What an attempt's ending tells its domain's Politeness (ADRs 0021,
//! 0086 and 0118): the backoff, the 429's halving and the time a
//! `Retry-After` asked for, and the requests a site's time holds further
//! away than the Run waits, which end `failed` unsent.

use std::time::{Duration, SystemTime};

use goodfetch_engine::admission::{Admission, Admitted, Ended};
use goodfetch_engine::retry_after;
use goodfetch_engine::run_state::RunStateError;
use goodfetch_engine::runs::Run;
use goodfetch_protocol::block::BlockCause;
use goodfetch_protocol::connection::DEADLINE_EXCEEDED;
use goodfetch_protocol::foxhound::crawl::RESPONSE_UNAVAILABLE;
use goodfetch_protocol::settings::{TIMEOUT_FIRST_BYTE, TIMEOUT_NAVIGATION};
use serde_json::{Map, Value};
use tokio::time::Instant;

use super::attempt::{Answer, block_cause};
use crate::work_items::{self, Failure, Unsuccessful};

/// The `fix` of a request a site's `Retry-After` holds further away than
/// the Run waits (ADR 0118).
pub(super) const RETRY_AFTER_FIX: &str = "The site asked for no request to its domain before `retry_after`, further away than `politeness.retry_after.max`: re-admit the request with `run.retry_failed` once the site is back, or raise `politeness.retry_after.max` for the crawl to wait that long.";

/// The detail naming the time a site's `Retry-After` asked for, as the
/// engine's `blocked` spells it on the wire (ADRs 0038 and 0118).
const RETRY_AFTER: &str = "retry_after";

/// The statuses whose response backs a domain off (ADR 0086).
const BACKS_OFF: [u64; 3] = [502, 503, 504];

/// What an attempt's answer says of its domain: a handler's failure that
/// says the site did not answer in time (a `deadline_exceeded` whose
/// `limit` is the response's first byte or the navigation) backs the domain
/// off like a refused or timed-out connection (ADR 0086); one that says the
/// site refused for too many requests (`blocked` for a `rate_limit`, a 429)
/// halves its concurrency (ADR 0021); one that says the site was failing
/// for now (`response_unavailable` for a 502, 503 or 504) backs it off
/// (ADR 0086). The time a failure's `retry_after` names, read at `wall` when
/// the monotonic clock read `now`, holds the domain until then (ADR 0118).
pub(super) fn domain_ending(answer: &Answer, now: Instant, wall: SystemTime) -> Ended {
    let Answer::Failed { code, details, .. } = answer else {
        return Ended::Done;
    };
    let asked = details
        .get(RETRY_AFTER)
        .and_then(retry_after::of_detail)
        .map(|asked| now + asked.duration_since(wall).unwrap_or_default());
    if block_cause(code, details) == Some(BlockCause::RateLimit) {
        return Ended::TooManyRequests(asked);
    }
    let unavailable = code == RESPONSE_UNAVAILABLE.name
        && details
            .get("status")
            .and_then(Value::as_u64)
            .is_some_and(|status| BACKS_OFF.contains(&status));
    if unavailable {
        return Ended::BackOff(asked);
    }
    let timed_out = code == DEADLINE_EXCEEDED.name
        && details
            .get("limit")
            .and_then(|limit| limit.get("key"))
            .and_then(Value::as_str)
            .is_some_and(|key| [TIMEOUT_FIRST_BYTE.name, TIMEOUT_NAVIGATION.name].contains(&key));
    if timed_out {
        Ended::BackOff(None)
    } else {
        Ended::Done
    }
}

/// The time a failure's `retry_after` names when it is further away from
/// `wall` than the Run waits, `longest_wait`; `None` when the Run waits for
/// nothing.
pub(super) fn held_past_bound(
    details: &Map<String, Value>,
    wall: SystemTime,
    longest_wait: Option<Duration>,
) -> Option<SystemTime> {
    let longest_wait = longest_wait?;
    details
        .get(RETRY_AFTER)
        .and_then(retry_after::of_detail)
        .filter(|asked| {
            asked
                .duration_since(wall)
                .is_ok_and(|wait| wait > longest_wait)
        })
}

/// Ends `failed` with `response_unavailable` each request admitted while
/// its domain's site asked for a time further away than the Run waits, with
/// no handler request sent: its status `null`, that time as `retry_after`
/// and a `fix` naming `politeness.retry_after.max` (ADR 0118). Its attempt
/// counts, so `run.failed_requests` lists it and `run.retry_failed`
/// re-admits it.
pub(super) async fn end_unsent(
    run: &Run,
    admission: &mut Admission,
    refused: Vec<(Admitted, SystemTime)>,
) -> Result<(), RunStateError> {
    let ended_at = run.point_in_time();
    let ending: Vec<(String, SystemTime)> = refused
        .iter()
        .map(|(admitted, until)| (admitted.id.clone(), *until))
        .collect();
    run.state()
        .write(move |connection| {
            for (id, until) in ending {
                for leased in work_items::lease(connection, std::slice::from_ref(&id))? {
                    let mut details = Map::new();
                    details.insert("status".to_owned(), Value::Null);
                    details.insert(
                        RETRY_AFTER.to_owned(),
                        Value::from(retry_after::detail(until)),
                    );
                    details.insert("fix".to_owned(), Value::from(RETRY_AFTER_FIX));
                    let failure = Failure {
                        code: RESPONSE_UNAVAILABLE.name.to_owned(),
                        message: "The site asked for no request to its domain before a time further away than `politeness.retry_after.max`; nothing was sent.".to_owned(),
                        details,
                    };
                    work_items::end(
                        connection,
                        &id,
                        leased.lease_epoch,
                        Unsuccessful::Failed(failure),
                        &ended_at,
                    )?;
                }
            }
            Ok(())
        })
        .await?;
    let now = run.clock().now();
    for (admitted, _) in refused {
        run.record_outcome(RESPONSE_UNAVAILABLE.name);
        admission
            .ended(&admitted.id, &admitted.key, now, Ended::Done)
            .await;
    }
    Ok(())
}
