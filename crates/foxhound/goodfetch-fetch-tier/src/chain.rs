//! A crawl request's redirect chain (ADR 0118; pitfalls U12 and U13): the
//! hops its fetch follows, every tier that fetches a document alike, as the
//! browser follows them.
//!
//! - **Redirects** follow Fetch's statuses and its method, body and header
//!   rules (the `redirects` module); any other 3xx fails with
//!   `response_not_ok`, and a status `crawl.handle_statuses` lists is the
//!   document, unfollowed.
//! - **A refresh stub** (the `refresh` module) to another URL, with a delay
//!   up to `fetch.refresh.max_delay`, is one more hop, a `GET` made at
//!   once; one with no URL or to the document's own is never followed.
//! - **At most `fetch.redirect.max` hops** after the first: one more fails
//!   with `too_many_redirects`, so a loop costs a site `fetch.redirect.max`
//!   + 1 requests.
//! - **A `Retry-After` on a 3xx** delays the next hop until then (RFC 9110
//!   §10.2.3); one further away than the crawl waits
//!   ([`goodfetch_frontier::longest_wait`]) fails with
//!   `response_unavailable` naming the 3xx's status and the time.
//! - **Each hop after the first goes through a [`HopGate`]**, which takes
//!   its permit, held until its answer's head arrived, or ends the request
//!   [skipped](Stop::Skipped). The first hop's permit is its admission's.
//!   The gate is told as each hop, the first included, is sent.
//! - **A hop after the first that the transport's egress check refuses**,
//!   its URL or the address it would connect to (ADRs 0017 and 0118), ends
//!   the request `skipped` with `domain_policy`: a redirect is no way
//!   around the policy, and its refusal is an outcome, not a failure.

use std::collections::BTreeSet;
use std::future::Future;
use std::pin::Pin;
use std::time::{Duration, SystemTime};

use goodfetch_engine::CallError;
use goodfetch_engine::clock::Clock;
use goodfetch_engine::retry_after::{self, RetryAfter};
use goodfetch_engine::settings::Resolved;
use goodfetch_page::page_value::Permit;
use goodfetch_protocol::egress::EGRESS_REFUSED;
use goodfetch_protocol::foxhound::crawl::{
    CRAWL_HANDLE_STATUSES, RESPONSE_NOT_OK, RESPONSE_UNAVAILABLE,
};
use goodfetch_protocol::foxhound::fetch::{FETCH_REDIRECT_MAX, FETCH_REFRESH_MAX_DELAY};
use goodfetch_protocol::foxhound::outcomes::SkipReason;
use goodfetch_protocol::page::{RedirectLimit, TOO_MANY_REDIRECTS, TooManyRedirectsDetails};
use serde_json::json;

use crate::driver::{Fetched, Hop};
use crate::redirects::{self, Next};
use crate::refresh;

/// What a crawl request's chain follows under: its crawl's settings.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Following {
    /// `fetch.redirect.max`: the hops followed after the first.
    pub max: u32,
    /// `fetch.refresh.max_delay`: `None` follows a refresh of any delay.
    pub refresh_max_delay: Option<Duration>,
    /// `crawl.handle_statuses`.
    pub handled: BTreeSet<u16>,
    /// The longest a site's `Retry-After` is waited
    /// ([`goodfetch_frontier::longest_wait`]); `None` waits for none.
    pub retry_after_max: Option<Duration>,
}

impl Following {
    /// What `settings` say.
    pub fn of(settings: &Resolved) -> Self {
        let handled = settings
            .value(CRAWL_HANDLE_STATUSES.name)
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|status| u16::try_from(status.as_u64()?).ok())
            .collect();
        Self {
            max: settings
                .count(FETCH_REDIRECT_MAX.name)
                .and_then(|max| u32::try_from(max).ok())
                .unwrap_or(19),
            refresh_max_delay: settings.duration(FETCH_REFRESH_MAX_DELAY.name),
            handled,
            retry_after_max: goodfetch_frontier::longest_wait(settings),
        }
    }
}

/// Why a request ends `skipped` at a hop, its handler never run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Skip {
    /// The outcome's reason.
    pub reason: SkipReason,
    /// What it says of the hop.
    pub message: String,
}

/// How a chain ends without a document.
#[derive(Debug)]
pub enum Stop {
    /// The attempt fails with this error.
    Failed(CallError),
    /// The request ends `skipped`.
    Skipped(Skip),
}

/// A boxed future a [`HopGate`] answers.
pub type Gating<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// What admits each hop of a chain after its first, and waits a site's
/// `Retry-After` out for it, holding the request's lease meanwhile.
pub trait HopGate: Send + Sync {
    /// Admits `hop`: its permit, released once dropped, or why it is not
    /// sent.
    fn admit<'a>(&'a self, hop: &'a Hop) -> Gating<'a, Result<Permit, Stop>>;

    /// Waits until `until` before `hop` is sent.
    fn wait_until<'a>(&'a self, hop: &'a Hop, until: SystemTime) -> Gating<'a, ()>;

    /// Records that `hop`, the first included, is sent now: its request's
    /// start, from which its domain's delay counts (ADR 0086), later than
    /// its admission by whatever ran between them.
    fn sending<'a>(&'a self, hop: &'a Hop) -> Gating<'a, ()>;

    /// Records that `hop`'s answer arrived `latency` after it was sent,
    /// for its domain's adaptive delay.
    fn answered<'a>(&'a self, hop: &'a Hop, latency: Duration) -> Gating<'a, ()>;

    /// Records that the chain ended on `hop`, one after its first, whose
    /// answer is its document.
    fn reached<'a>(&'a self, hop: &'a Hop) -> Gating<'a, ()>;
}

/// Follows the chain from `first` under `following`, each hop fetched with
/// `fetch` and each after the first admitted by `gate`, on `clock`, each
/// told to `gate` as it is sent and once its answer arrived; answers the
/// document the chain ends on.
///
/// # Errors
///
/// [`Stop::Failed`] with `fetch`'s error, `too_many_redirects`,
/// `response_not_ok` or `response_unavailable`; [`Stop::Skipped`] when
/// `gate` skips a hop, and with `domain_policy` when `fetch` fails a hop
/// after the first with `egress_refused`.
///
/// # Cancel safety
///
/// Dropping the future drops the hop in flight and its permit.
pub async fn follow<F, Fut>(
    first: Hop,
    following: &Following,
    gate: &dyn HopGate,
    clock: &Clock,
    mut fetch: F,
) -> Result<Fetched, Stop>
where
    F: FnMut(Hop) -> Fut,
    Fut: Future<Output = Result<Fetched, CallError>>,
{
    let first_url = first.url.to_string();
    let mut hops: Vec<String> = Vec::new();
    let mut hop = first;
    let mut permit: Option<Permit> = None;
    loop {
        gate.sending(&hop).await;
        let sent = clock.now();
        let fetched = fetch(hop.clone()).await.map_err(|error| {
            if hops.is_empty() || error.code() != EGRESS_REFUSED.name {
                return Stop::Failed(error);
            }
            Stop::Skipped(Skip {
                reason: SkipReason::DomainPolicy,
                message: format!(
                    "A redirect to `{}` reaches an address the Run's egress policy refuses: {}",
                    hop.url,
                    error.message()
                ),
            })
        })?;
        gate.answered(&hop, clock.now().duration_since(sent)).await;
        // The hop's answer arrived: its permit is released (ADR 0118).
        drop(permit.take());
        let Some(next) = next_hop(&hop, &fetched, following)? else {
            if !hops.is_empty() {
                gate.reached(&hop).await;
            }
            return Ok(fetched);
        };
        if hops.len() >= usize::try_from(following.max).unwrap_or(usize::MAX) {
            return Err(Stop::Failed(too_many_redirects(
                &first_url,
                hops,
                following.max,
            )));
        }
        if let Some(until) = retry_after(&fetched, following, clock)? {
            gate.wait_until(&next, until).await;
        }
        permit = Some(gate.admit(&next).await?);
        hops.push(next.url.to_string());
        hop = next;
    }
}

/// The hop `fetched`, the answer of `hop`, leads to: a redirect's, or a
/// refresh stub's within `fetch.refresh.max_delay`; `None` when it is the
/// chain's document.
fn next_hop(hop: &Hop, fetched: &Fetched, following: &Following) -> Result<Option<Hop>, Stop> {
    if following.handled.contains(&fetched.status) {
        return Ok(None);
    }
    let location = header(&fetched.headers, "location");
    match redirects::next(hop, fetched.status, location) {
        Next::Follow(next) => return Ok(Some(next)),
        Next::NotOk(reason) => return Err(Stop::Failed(not_ok(fetched, &reason))),
        Next::Answer => {}
    }
    let html = header(&fetched.headers, "content-type").is_none_or(|content_type| {
        let essence = content_type
            .split(';')
            .next()
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase();
        essence == "text/html" || essence == "application/xhtml+xml"
    });
    let Some(refresh) = refresh::directive(&fetched.url, &fetched.headers, &fetched.body, html)
    else {
        return Ok(None);
    };
    let within = following
        .refresh_max_delay
        .is_none_or(|max| refresh.delay <= max);
    if !within || refresh.reloads(&fetched.url) {
        return Ok(None);
    }
    let from = Hop {
        url: fetched.url.clone(),
        ..hop.clone()
    };
    Ok(Some(redirects::hop_to(&from, refresh.url, true)))
}

/// The value of the header `name` (lower-cased) among `headers`, if any.
fn header<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(header, _)| header == name)
        .map(|(_, value)| value.as_str())
}

/// The time a redirect's `Retry-After` asks its next hop to wait for, read
/// on `clock`; `None` for no wait, or when the crawl waits for none.
///
/// # Errors
///
/// `response_unavailable` with the redirect's status and the time, when it
/// is further away than the crawl waits.
fn retry_after(
    fetched: &Fetched,
    following: &Following,
    clock: &Clock,
) -> Result<Option<SystemTime>, Stop> {
    let (Some(value), Some(bound)) = (
        header(&fetched.headers, "retry-after"),
        following.retry_after_max,
    ) else {
        return Ok(None);
    };
    let now = clock.wall();
    let RetryAfter::Until(until) = retry_after::read(value, header(&fetched.headers, "date"), now)
    else {
        return Ok(None);
    };
    if until.duration_since(now).is_ok_and(|wait| wait > bound) {
        return Err(Stop::Failed(unavailable(fetched, until)));
    }
    Ok(Some(until))
}

/// The `fix` of a 3xx that is not followed.
const NOT_OK_FIX: &str =
    "List the status in `crawl.handle_statuses` for the handler to be given the response as it is.";

/// `response_not_ok` for `fetched`, a 3xx not followed for `reason`.
fn not_ok(fetched: &Fetched, reason: &str) -> CallError {
    let url = fetched.url.as_str();
    CallError::new(
        &RESPONSE_NOT_OK,
        format!(
            "{url} answered a {} that is not followed: {reason}.",
            fetched.status
        ),
        &json!({ "url": url, "status": fetched.status, "fix": NOT_OK_FIX }),
    )
}

/// `response_unavailable` for `fetched`, a redirect whose `Retry-After`
/// asks for `until`, further away than the crawl waits (ADR 0118).
fn unavailable(fetched: &Fetched, until: SystemTime) -> CallError {
    let url = fetched.url.as_str();
    CallError::new(
        &RESPONSE_UNAVAILABLE,
        format!(
            "{url} answered a {} whose `Retry-After` asks for no request before {}, further away than `politeness.retry_after.max`.",
            fetched.status,
            retry_after::detail(until)
        ),
        &json!({
            "url": url,
            "status": fetched.status,
            "retry_after": retry_after::detail(until),
        }),
    )
}

/// `too_many_redirects` for the chain from `url` past `max` redirects,
/// each of `hops` followed.
fn too_many_redirects(url: &str, hops: Vec<String>, max: u32) -> CallError {
    CallError::new(
        &TOO_MANY_REDIRECTS,
        format!(
            "The fetch of {url} was redirected again after {} redirects, past `fetch.redirect.max`'s {max}.",
            hops.len()
        ),
        &TooManyRedirectsDetails {
            url: url.to_owned(),
            hops,
            limit: RedirectLimit::Setting {
                name: FETCH_REDIRECT_MAX.name.to_owned(),
                count: max,
            },
        },
    )
}
