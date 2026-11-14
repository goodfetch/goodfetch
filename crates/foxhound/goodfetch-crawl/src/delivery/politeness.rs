//! What an attempt's ending tells its domain's Politeness (ADRs 0021,
//! 0086 and 0118): the backoff, the 429's halving and the time a
//! `Retry-After` asked for, and the requests a site's time holds further
//! away than the Run waits, which end `failed` unsent.

use std::time::{Duration, SystemTime};

use goodfetch_engine::admission::{Admission, Admitted, Ended};
use goodfetch_engine::retry_after::{self, RetryAfter};
use goodfetch_engine::runs::Run;
use goodfetch_frontier::backoff_classes::{Class, Met, class};
use goodfetch_protocol::block::BlockCause;
use goodfetch_protocol::foxhound::crawl::RESPONSE_UNAVAILABLE;
use goodfetch_protocol::page::PageResponse;
use serde_json::{Map, Value};
use tokio::time::Instant;

use super::attempt::Answer;
use super::failure::{RETRY_AFTER, Why};
use crate::run_state::{StateError, Stored};
use crate::work_items::Failure;

/// The `fix` of a request a site's `Retry-After` holds further away than
/// the Run waits (ADR 0118).
pub(super) const RETRY_AFTER_FIX: &str = "The site asked for no request to its domain before `retry_after`, further away than `politeness.retry_after.max`: re-admit the request with `run.retry_failed` once the site is back, or raise `politeness.retry_after.max` for the crawl to wait that long.";

/// The header a proxy names its own errors in (RFC 9209), lower-cased as a
/// response's headers are.
const PROXY_STATUS: &str = "proxy-status";

/// What an attempt's ending says of its domain (ADRs 0021, 0053, 0086 and
/// 0118; the `backoff_classes` rule): what its handler's failure says
/// ([`failure_ending`]), else what the site's answer to the request's own
/// document, `document`, says, the engine having fetched it on any tier,
/// whatever the handler made of the page: a 429 halves the domain's
/// concurrency and backs it off, a 502, 503 or 504 backs it off, each
/// until the time its `Retry-After` names when that is later, and a 500 or
/// any other status says nothing. An answer the Run's proxy gave itself
/// (`proxied`: its Sessions send through one, which forwards an `http:`
/// URL itself) backs no domain off: its limits are the proxy's (ADR
/// 0053). Times are read at `wall` when the monotonic clock read `now`.
pub(super) fn domain_ending(
    answer: &Answer,
    document: Option<&PageResponse>,
    proxied: bool,
    now: Instant,
    wall: SystemTime,
) -> Ended {
    match failure_ending(answer, now, wall) {
        Ended::Done => document.map_or(Ended::Done, |response| {
            document_ending(response, proxied, now, wall)
        }),
        ended => ended,
    }
}

/// What an attempt's failure says of its domain: a fetch that failed
/// because the site refused or timed out the connection, or reset it with
/// HTTP/2's `ENHANCE_YOUR_CALM` (`navigation_failed` naming that network
/// error), and a handler's failure that says the site did not answer in
/// time (a `deadline_exceeded` whose `limit` is the response's first byte
/// or the navigation), back the domain off (ADRs 0086 and 0118); one that
/// says the site refused for too many requests (`blocked` for a
/// `rate_limit`, a 429) halves its concurrency (ADR 0021); one that says
/// the site was failing for now (`response_unavailable` for a 502, 503 or
/// 504) backs it off (ADR 0086). The time a failure's `retry_after` names
/// holds the domain until then (ADR 0118). A proxy's own refusal backs no
/// domain off.
fn failure_ending(answer: &Answer, now: Instant, wall: SystemTime) -> Ended {
    let failure = match answer {
        Answer::Failed(failure) => failure,
        // robots.txt is the whole origin's file: one that answers 429 or
        // 5xx, or no answer, backs off the domain as a 503 does (ADR 0086).
        // A hop's on another domain key says nothing of this request's.
        Answer::RobotsTxtUnreachable(_) => return Ended::BackOff(None),
        _ => return Ended::Done,
    };
    let asked = || failure.retry_after().map(|asked| at(asked, now, wall));
    match failure.why() {
        Why::Blocked(BlockCause::RateLimit) => Ended::TooManyRequests(asked()),
        Why::Unavailable(Some(status)) if class(site_response(*status)) == Class::BackOff => {
            Ended::BackOff(asked())
        }
        Why::Unreachable | Why::TimedOut => Ended::BackOff(None),
        Why::Blocked(_)
        | Why::Unavailable(_)
        | Why::PageCheckFailed { .. }
        | Why::Proxy
        | Why::Handler
        | Why::OutcomeNotWritten
        | Why::Panicked
        | Why::Other => Ended::Done,
    }
}

/// The site's own response of `status`, as the backoff classes read it.
fn site_response(status: u16) -> Met<'static> {
    Met::Response {
        status,
        proxy_status: None,
        forwarded: false,
    }
}

/// What the answer `response` to a request's own document says of its
/// domain, by the backoff classes, with the time its `Retry-After` names,
/// measured against its own `Date` when it has one (ADR 0118).
fn document_ending(
    response: &PageResponse,
    proxied: bool,
    now: Instant,
    wall: SystemTime,
) -> Ended {
    let header = |name: &str| response.headers.get(name).map(String::as_str);
    let met = Met::Response {
        status: response.status,
        proxy_status: header(PROXY_STATUS),
        forwarded: proxied && response.url.starts_with("http:"),
    };
    let asked = || match retry_after::read(header("retry-after")?, header("date"), wall) {
        RetryAfter::Until(until) => Some(at(until, now, wall)),
        RetryAfter::NoWait | RetryAfter::Unparsable => None,
    };
    match class(met) {
        Class::TooManyRequests => Ended::TooManyRequests(asked()),
        Class::BackOff => Ended::BackOff(asked()),
        Class::Done | Class::ProxyRefusal => Ended::Done,
    }
}

/// The monotonic moment of the wall time `until`, the monotonic clock
/// reading `now` at the wall time `wall`.
fn at(until: SystemTime, now: Instant, wall: SystemTime) -> Instant {
    now + until.duration_since(wall).unwrap_or_default()
}

/// The time the `retry_after` of the handler's failure `answer` names when
/// it is further away than the Run waits, `longest_wait`, from `asked`, the
/// moment the site asked: when its handler request went out, never when
/// the failure arrived, so the time the handler and the engine took to
/// report it never shortens the site's wait (ADR 0118). `None` for any
/// other answer, and when the Run waits for nothing.
pub(super) fn held_past_bound(
    answer: &Answer,
    asked: SystemTime,
    longest_wait: Option<Duration>,
) -> Option<SystemTime> {
    let longest_wait = longest_wait?;
    let Answer::Failed(failure) = answer else {
        return None;
    };
    failure.retry_after().filter(|until| {
        until
            .duration_since(asked)
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
    stored: &Stored,
    admission: &mut Admission,
    refused: Vec<(Admitted, SystemTime)>,
) -> Result<(), StateError> {
    let unsent = refused
        .iter()
        .map(|(admitted, until)| {
            let mut details = Map::new();
            details.insert("status".to_owned(), Value::Null);
            details.insert(
                RETRY_AFTER.to_owned(),
                Value::from(retry_after::detail(*until)),
            );
            details.insert("fix".to_owned(), Value::from(RETRY_AFTER_FIX));
            let failure = Failure {
                code: RESPONSE_UNAVAILABLE.name.to_owned(),
                message: "The site asked for no request to its domain before a time further away than `politeness.retry_after.max`; nothing was sent.".to_owned(),
                details,
            };
            (admitted.id.clone(), failure)
        })
        .collect();
    stored.end_unsent(unsent, run.point_in_time()).await?;
    let now = run.clock().now();
    for (admitted, _) in refused {
        run.record_outcome(RESPONSE_UNAVAILABLE.name);
        admission
            .ended(&admitted.id, &admitted.key, now, Ended::Done)
            .await;
    }
    Ok(())
}

#[cfg(test)]
mod tests;
