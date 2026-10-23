//! The `limits` family on `limits.test`: one list of 24 items behind a rule
//! per task, each rule keyed by the address a request came from, as a site
//! sees it:
//!
//! - `limits/1`, a rate limit: each address may make 10 requests, then gets
//!   `429` with `Retry-After: 1` and a pause of that second; a request in the
//!   pause is a `retry_after_ignored` finding, answered `429` again, and the
//!   third flags the address, refused with `403` from then on.
//! - `limits/2`, a quota: each address may fetch 10 pages; the next request
//!   flags it, so a complete crawl needs more than one address.
//! - `limits/3`, a slow site: each page is held 50 ms before it is sent, and
//!   at most 2 requests may be in flight at once on the task's host, from
//!   every address together; one more is answered `503` with
//!   `Retry-After: 1` and is an `over_in_flight_cap` finding.
//!
//! The rules read "now" as a value, so each is a pure function of an
//! address's state and the time.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::net::IpAddr;
use std::sync::Arc;
use std::time::Duration;

use hyper::StatusCode;

use super::{Page, Visit, crawl_task, document, name, numbered};
use crate::sync::Mutex;
use crate::task::{Finding, Record, Task};

pub(super) const HOST: &str = "limits.test";
pub(super) const FAMILY: &str = "limits";

/// The generator place of the family's values.
const FAMILY_PLACE: u64 = 23;

const ITEMS: u64 = 24;
/// The requests an address may make before a pause, for `limits/1`.
const BUDGET: u32 = 10;
/// The pause a `429` asks for, in seconds.
const PAUSE_SECONDS: u64 = 1;
/// The requests in a pause that flag an address.
const IGNORED_BEFORE_FLAG: u32 = 3;
/// The pages an address may fetch, for `limits/2`.
const QUOTA: u32 = 10;
/// The requests that may be in flight at once on a host, for `limits/3`.
const IN_FLIGHT_CAP: usize = 2;
/// How long `limits/3` holds each page.
const HOLD: Duration = Duration::from_millis(50);
/// The most addresses kept per `serve` (STYLE.md: every buffer is bounded);
/// a later one is judged from a fresh state each time.
const ADDRESS_LIMIT: usize = 4096;

/// What one address did on one task's host.
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
struct Address {
    /// Requests served since its last pause.
    served: u32,
    /// When its pause ends, in microseconds since `serve` started.
    pause_until: Option<u64>,
    /// Requests it made in a pause.
    ignored: u32,
    flagged: bool,
}

/// What the family keeps: each address's state per task host, and the
/// requests in flight per host.
#[derive(Default)]
pub(super) struct State {
    addresses: Mutex<BTreeMap<(String, IpAddr), Address>>,
    in_flight: Arc<Mutex<BTreeMap<String, usize>>>,
}

/// What a rule makes of one request.
#[derive(Debug, PartialEq, Eq)]
enum Admission {
    Serve,
    /// `429`, with the seconds left to wait; `ignored` when the request came
    /// in a pause.
    Limited {
        retry_after: u64,
        ignored: bool,
    },
    Refused,
}

/// `limits/1`'s rule at `now` (microseconds).
fn rate_limit(address: &mut Address, now: u64) -> Admission {
    if address.flagged {
        return Admission::Refused;
    }
    if let Some(until) = address.pause_until {
        if now < until {
            address.ignored += 1;
            address.flagged = address.ignored >= IGNORED_BEFORE_FLAG;
            return Admission::Limited {
                retry_after: (until - now).div_ceil(1_000_000),
                ignored: true,
            };
        }
        address.pause_until = None;
        address.served = 0;
    }
    if address.served >= BUDGET {
        address.pause_until = Some(now + PAUSE_SECONDS * 1_000_000);
        return Admission::Limited {
            retry_after: PAUSE_SECONDS,
            ignored: false,
        };
    }
    address.served += 1;
    Admission::Serve
}

/// `limits/2`'s rule.
fn quota(address: &mut Address) -> Admission {
    if address.flagged || address.served >= QUOTA {
        address.flagged = true;
        return Admission::Refused;
    }
    address.served += 1;
    Admission::Serve
}

/// A held answer: its request stays in flight on its host until the server
/// has waited and dropped it.
pub(crate) struct Held {
    delay: Duration,
    /// The request's place in flight, when its host counts them.
    _slot: Option<Slot>,
}

impl Held {
    /// A hold of `delay` on a host that counts no request in flight.
    pub(crate) fn after(delay: Duration) -> Self {
        Self { delay, _slot: None }
    }

    /// Waits the hold, then frees the request's place in flight.
    pub(crate) async fn release(self) {
        tokio::time::sleep(self.delay).await;
    }
}

/// A request's place among those in flight on its host.
struct Slot {
    in_flight: Arc<Mutex<BTreeMap<String, usize>>>,
    host: String,
}

impl Drop for Slot {
    fn drop(&mut self) {
        let mut in_flight = self.in_flight.lock();
        if let Some(count) = in_flight.get_mut(&self.host) {
            *count = count.saturating_sub(1);
        }
    }
}

impl State {
    /// Applies `rule` to the request's address on its host.
    fn admit(
        &self,
        host: &str,
        peer: IpAddr,
        rule: impl FnOnce(&mut Address) -> Admission,
    ) -> Admission {
        let mut addresses = self.addresses.lock();
        let key = (host.to_owned(), peer);
        if !addresses.contains_key(&key) && addresses.len() >= ADDRESS_LIMIT {
            return rule(&mut Address::default());
        }
        rule(addresses.entry(key).or_default())
    }

    /// A place in flight on `host`, if it has fewer than its cap.
    fn slot(&self, host: &str) -> Option<Slot> {
        let mut in_flight = self.in_flight.lock();
        let count = in_flight.entry(host.to_owned()).or_default();
        (*count < IN_FLIGHT_CAP).then(|| {
            *count += 1;
            Slot {
                in_flight: Arc::clone(&self.in_flight),
                host: host.to_owned(),
            }
        })
    }
}

fn item_id(number: u64) -> String {
    format!("L-{number:04}")
}

fn item_name(seed: u64, number: u64) -> String {
    name(seed, &[FAMILY_PLACE, number])
}

pub(super) fn respond(visit: &Visit<'_>, state: &State) -> Page {
    let request = visit.request;
    let host = request.host;
    let ordinal = host.split('.').next().unwrap_or_default();
    let admission = match ordinal {
        "1" => state.admit(host, request.peer, |address| rate_limit(address, visit.now)),
        "2" => state.admit(host, request.peer, quota),
        _ => Admission::Serve,
    };
    match admission {
        Admission::Refused => return Page::blocked(),
        Admission::Limited {
            retry_after,
            ignored,
        } => {
            if ignored {
                visit.find(Finding::RetryAfterIgnored {
                    target: request.target(),
                });
            }
            return Page::html(
                StatusCode::TOO_MANY_REQUESTS,
                document("Too many requests", "<h1>Too many requests</h1>\n"),
            )
            .header("retry-after", retry_after.to_string());
        }
        Admission::Serve => {}
    }
    let Some(html) = page(visit.seed, request.path) else {
        return Page::not_found();
    };
    if ordinal != "3" {
        return Page::ok(html);
    }
    let Some(slot) = state.slot(host) else {
        visit.find(Finding::OverInFlightCap {
            target: request.target(),
        });
        return Page::html(
            StatusCode::SERVICE_UNAVAILABLE,
            document(
                "Busy",
                "<h1>Busy</h1>\n<p>Too many requests in flight.</p>\n",
            ),
        )
        .header("retry-after", "1".to_owned());
    };
    Page {
        held: Some(Held {
            delay: HOLD,
            _slot: Some(slot),
        }),
        ..Page::ok(html)
    }
}

fn page(seed: u64, path: &str) -> Option<String> {
    if path == "/" {
        let mut body = String::from("<main>\n<h1>Items</h1>\n<ul class=\"items\">\n");
        for number in 1..=ITEMS {
            writeln!(
                body,
                "<li><a href=\"/items/{}\">{}</a></li>",
                item_id(number),
                item_name(seed, number)
            )
            .expect("invariant: writing to a String succeeds");
        }
        body.push_str("</ul>\n</main>\n");
        return Some(document("Items", &body));
    }
    let number = numbered(path.strip_prefix("/items/")?, "L-", 4, ITEMS)?;
    let (id, name) = (item_id(number), item_name(seed, number));
    Some(document(
        &name,
        &format!(
            "<main>\n<article class=\"item\" data-id=\"{id}\">\n<h1 class=\"name\">{name}</h1>\n</article>\n<a href=\"/\">All items</a>\n</main>\n"
        ),
    ))
}

pub(super) fn tasks(seed: u64, port: Option<u16>) -> Vec<Task> {
    let records = || -> Vec<Record> {
        (1..=ITEMS)
            .map(|number| {
                Record::from([("id", item_id(number)), ("name", item_name(seed, number))])
            })
            .collect()
    };
    vec![
        crawl_task(
            (HOST, FAMILY),
            1,
            port,
            "/",
            "Crawl the items from the list and return one Record per item, with the fields `id` \
                and `name`. Each address may make 10 requests, then gets `429 Too Many Requests` \
                with `Retry-After`; a request before that time has passed is ignored advice, and \
                an address that ignores it three times is flagged and refused from then on.",
            records(),
        ),
        crawl_task(
            (HOST, FAMILY),
            2,
            port,
            "/",
            "Crawl the items from the list and return one Record per item, with the fields `id` \
                and `name`. Each address may fetch 10 pages; the site flags an address that asks \
                for more and refuses it with `403 Forbidden` from then on, so the crawl needs \
                more than one address.",
            records(),
        ),
        crawl_task(
            (HOST, FAMILY),
            3,
            port,
            "/",
            "Crawl the items from the list and return one Record per item, with the fields `id` \
                and `name`. The site takes 50 milliseconds to answer each page and allows 2 \
                requests in flight at once; one more is answered `503 Service Unavailable` with \
                `Retry-After`.",
            records(),
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECOND: u64 = 1_000_000;

    #[test]
    fn a_rate_limit_pauses_an_address_after_its_budget_for_its_retry_after() {
        let mut address = Address::default();
        let served: Vec<Admission> = (0..BUDGET).map(|_| rate_limit(&mut address, 0)).collect();

        assert!(
            served
                .iter()
                .all(|admission| *admission == Admission::Serve)
        );
        assert_eq!(
            rate_limit(&mut address, 5),
            Admission::Limited {
                retry_after: 1,
                ignored: false
            }
        );
        assert_eq!(
            rate_limit(&mut address, SECOND / 2),
            Admission::Limited {
                retry_after: 1,
                ignored: true
            }
        );
        assert_eq!(rate_limit(&mut address, SECOND + 5), Admission::Serve);
    }

    #[test]
    fn a_third_request_in_a_pause_flags_the_address_for_good() {
        let mut address = Address {
            served: BUDGET,
            ..Address::default()
        };
        rate_limit(&mut address, 0);
        for at in 1..=3 {
            rate_limit(&mut address, at);
        }

        assert!(address.flagged);
        assert_eq!(rate_limit(&mut address, 10 * SECOND), Admission::Refused);
    }

    #[test]
    fn a_quota_refuses_an_address_past_its_pages_from_then_on() {
        let mut address = Address::default();
        let served = (0..QUOTA)
            .filter(|_| quota(&mut address) == Admission::Serve)
            .count();

        assert_eq!(served, 10);
        assert_eq!(quota(&mut address), Admission::Refused);
        assert!(address.flagged);
    }
}
