//! The Crawl families (ADR 0025's Crawl cases, and ADR 0005's parameter that
//! looks like it has no effect), each served on its own host name and each
//! of its tasks on `<ordinal>.<family host>`, as every family is:
//!
//! - `statuses`: every HTTP error, 402 included, soft 404s, redirects and
//!   redirect loops;
//! - `traps`: crawler traps, duplicate URLs and honeypot links, a honeypot
//!   flagging the address that followed it;
//! - `limits`: per-address rate limits with `Retry-After`, flagged addresses
//!   and a cap of requests in flight;
//! - `parameters`: a parameter that seems to change nothing but separates
//!   distinct pages on some products;
//! - `scale`: millions of seeded pages, with sitemaps;
//! - `links`: relative links under a `<base href>` and after a redirect, and
//!   links only scripts follow (pitfalls U08 and U14);
//! - `pacing`: a host whose latency rises with the time step;
//! - `facets`: a faceted listing that grows combinatorially (ADR 0120);
//! - `fields`: Records that gain fields over time (pitfall D07);
//! - `sitemaps`: a sitemap index past the protocol's 50,000 entries, plain
//!   and gzip;
//! - `learning`: pages from which a Frontier learns a parameter with no
//!   effect, a trap and cursor pagination preferred to offset (ADR 0005),
//!   and equivalent forms on evidence (ADR 0120), each threshold met and
//!   one short of it;
//! - `scope`: a site with a `www` host and a subdomain beside a sister
//!   domain and an off-site host (pitfall U15), on host names of its own;
//! - `sitemap-seed`: a site whose sitemaps a sitemap seed reads, its
//!   robots.txt cross-submitting one on a partner host (pitfall U20), on
//!   host names of its own;
//! - `tiers`: static and script-rendered listings on one host, apart by
//!   their paths' first segment, from which a crawl learns its tiers;
//! - `comparison`: `just compare`'s multi-domain site and script-rendered
//!   site, the `scale` family's tree of pages on host names of their own
//!   under `localhost`.
//!
//! Every page and expected result is a pure function of the seed and the
//! page's place; what a family keeps between requests is what it saw a
//! crawl do, which it records as findings in the observations, for the
//! oracle.

mod comparison;
mod facets;
mod fields;
mod learning;
mod limits;
mod links;
mod pacing;
mod parameters;
mod scale;
mod scope;
mod sitemap_seed;
mod sitemaps;
mod statuses;
mod tiers;
mod traps;

use std::collections::BTreeMap;
use std::net::IpAddr;

use hyper::body::Bytes;
use hyper::{Method, StatusCode};

use crate::clock::Stopwatch;
use crate::exchange::Reply;
use crate::generator;
use crate::observation::Observations;
use crate::sync::Mutex;
use crate::task::{self, Entry, Expected, Finding, Kind, Record, Task};

pub(crate) use limits::Held;

/// The family host names, each with its family's name.
const FAMILIES: [(&str, &str); 14] = [
    (statuses::HOST, statuses::FAMILY),
    (traps::HOST, traps::FAMILY),
    (limits::HOST, limits::FAMILY),
    (parameters::HOST, parameters::FAMILY),
    (scale::HOST, scale::FAMILY),
    (links::HOST, links::FAMILY),
    (pacing::HOST, pacing::FAMILY),
    (facets::HOST, facets::FAMILY),
    (fields::HOST, fields::FAMILY),
    (sitemaps::HOST, sitemaps::FAMILY),
    (learning::HOST, learning::FAMILY),
    (learning::SHOP_HOST, learning::FAMILY),
    (learning::WWW_HOST, learning::FAMILY),
    (tiers::HOST, tiers::FAMILY),
];

/// The parts of a request a Crawl family reads: an
/// [`exchange::Request`](crate::exchange::Request)'s, with its host name.
pub(crate) struct Request<'a> {
    pub(crate) method: &'a Method,
    /// The host name, without its port, in lowercase.
    pub(crate) host: &'a str,
    /// The `Host` header, or HTTP/2's `:authority`, port included.
    pub(crate) authority: &'a str,
    pub(crate) path: &'a str,
    /// The query without its `?`; empty when the target has none.
    pub(crate) query: &'a str,
    /// The address the request came from.
    pub(crate) peer: IpAddr,
}

impl Request<'_> {
    /// The request's target, its query included.
    fn target(&self) -> String {
        if self.query.is_empty() {
            self.path.to_owned()
        } else {
            format!("{}?{}", self.path, self.query)
        }
    }

    /// The value of the query parameter `name`, as sent, if any.
    fn parameter(&self, name: &str) -> Option<&str> {
        self.query
            .split('&')
            .filter_map(|pair| pair.split_once('='))
            .find_map(|(key, value)| (key == name).then_some(value))
    }
}

/// What a Crawl family answers to one request.
pub(crate) struct Page {
    pub(crate) status: StatusCode,
    pub(crate) content_type: &'static str,
    /// Headers beyond `Content-Type`, such as `Location` and `Retry-After`.
    pub(crate) headers: Vec<(&'static str, String)>,
    pub(crate) body: Bytes,
    /// Set when the server holds the answer a while, its request in
    /// flight until it is sent.
    pub(crate) held: Option<Held>,
}

impl Page {
    /// The reply the server sends, and how long it holds it first.
    pub(crate) fn into_reply(self) -> (Reply, Option<Held>) {
        (
            Reply {
                status: self.status,
                content_type: self.content_type,
                headers: self.headers,
                body: self.body,
            },
            self.held,
        )
    }

    fn html(status: StatusCode, html: String) -> Self {
        Self {
            status,
            content_type: "text/html; charset=utf-8",
            headers: Vec::new(),
            body: html.into(),
            held: None,
        }
    }

    fn ok(html: String) -> Self {
        Self::html(StatusCode::OK, html)
    }

    fn text(content_type: &'static str, text: String) -> Self {
        Self {
            content_type,
            ..Self::ok(text)
        }
    }

    /// A redirect to `location`, a path on the same host.
    fn redirect(status: StatusCode, location: &str) -> Self {
        Self::html(status, document("Moved", "<p>This page has moved.</p>\n"))
            .header("location", location.to_owned())
    }

    fn not_found() -> Self {
        Self::html(
            StatusCode::NOT_FOUND,
            document("Not found", "<h1>Not found</h1>\n"),
        )
    }

    /// The page a flagged address gets.
    fn blocked() -> Self {
        Self::html(
            StatusCode::FORBIDDEN,
            document(
                "Access denied",
                "<h1>Access denied</h1>\n<p>Requests from your address have been flagged as automated and are refused.</p>\n",
            ),
        )
    }

    #[must_use]
    fn header(mut self, name: &'static str, value: String) -> Self {
        self.headers.push((name, value));
        self
    }
}

/// One request as a family serves it: the request, the seed, the time since
/// `serve` started, and where its findings go.
struct Visit<'a> {
    seed: u64,
    request: &'a Request<'a>,
    /// Microseconds since `serve` started.
    now: u64,
    observations: &'a Observations,
}

impl Visit<'_> {
    /// Records a finding on the request's task.
    fn find(&self, finding: Finding) {
        self.observations.finding(self.request.authority, finding);
    }
}

/// What the Crawl families keep for one `serve`: what they saw crawls do.
pub(crate) struct Families {
    stopwatch: Stopwatch,
    statuses: statuses::State,
    traps: traps::State,
    limits: limits::State,
    facets: facets::State,
    tiers: tiers::State,
}

impl Families {
    pub(crate) fn new() -> Self {
        Self {
            stopwatch: Stopwatch::start(),
            statuses: statuses::State::default(),
            traps: traps::State::default(),
            limits: limits::State::default(),
            facets: facets::State::default(),
            tiers: tiers::State::default(),
        }
    }

    /// Answers a request to a Crawl family's host name; `None` when the host
    /// is another family's.
    pub(crate) fn respond(
        &self,
        seed: u64,
        request: &Request<'_>,
        observations: &Observations,
    ) -> Option<Page> {
        let family = family_of(request.host)?;
        if !matches!(*request.method, Method::GET | Method::HEAD) {
            return Some(Page::not_found());
        }
        let visit = Visit {
            seed,
            request,
            now: self.stopwatch.microseconds(),
            observations,
        };
        Some(match family {
            statuses::FAMILY => statuses::respond(&visit, &self.statuses),
            traps::FAMILY => traps::respond(&visit, &self.traps),
            limits::FAMILY => limits::respond(&visit, &self.limits),
            parameters::FAMILY => parameters::respond(&visit),
            links::FAMILY => links::respond(&visit),
            pacing::FAMILY => pacing::respond(&visit),
            facets::FAMILY => facets::respond(&visit, &self.facets),
            fields::FAMILY => fields::respond(&visit),
            sitemaps::FAMILY => sitemaps::respond(&visit),
            scope::FAMILY => scope::respond(&visit),
            sitemap_seed::FAMILY => sitemap_seed::respond(&visit),
            learning::FAMILY => learning::respond(&visit),
            tiers::FAMILY => tiers::respond(&visit, &self.tiers),
            comparison::FAMILY => comparison::respond(&visit),
            _ => scale::respond(&visit),
        })
    }
}

/// The family whose host name, or one of whose task host names, `host` is.
fn family_of(host: &str) -> Option<&'static str> {
    if comparison::serves(host) {
        return Some(comparison::FAMILY);
    }
    if scope::serves(host) {
        return Some(scope::FAMILY);
    }
    if sitemap_seed::serves(host) {
        return Some(sitemap_seed::FAMILY);
    }
    FAMILIES.iter().find_map(|(name, family)| {
        let task_host = host
            .strip_suffix(name)
            .and_then(|rest| rest.strip_suffix('.'))
            .is_some_and(|ordinal| {
                !ordinal.is_empty() && ordinal.bytes().all(|b| b.is_ascii_digit())
            });
        (host == *name || task_host).then_some(*family)
    })
}

/// Every Crawl family's host names, each with the task its requests belong
/// to, if any.
pub(crate) fn hosts(seed: u64) -> Vec<(String, Option<String>)> {
    FAMILIES
        .iter()
        .map(|(host, _)| ((*host).to_owned(), None))
        .chain(scope::HOSTS.iter().map(|host| ((*host).to_owned(), None)))
        .chain(
            sitemap_seed::HOSTS
                .iter()
                .map(|host| ((*host).to_owned(), None)),
        )
        .chain(comparison::hosts().map(|host| (host, None)))
        .chain(
            tasks(seed, None)
                .into_iter()
                .map(|task| (task.host, Some(task.entry.id))),
        )
        .collect()
}

/// Every Crawl family's tasks, in the task list's order.
pub(crate) fn tasks(seed: u64, port: Option<u16>) -> Vec<Task> {
    let mut tasks = statuses::tasks(seed, port);
    tasks.extend(traps::tasks(seed, port));
    tasks.extend(parameters::tasks(seed, port));
    tasks.extend(limits::tasks(seed, port));
    tasks.extend(scale::tasks(seed, port));
    tasks.extend(links::tasks(seed, port));
    tasks.extend(pacing::tasks(seed, port));
    tasks.extend(facets::tasks(seed, port));
    tasks.extend(fields::tasks(seed, port));
    tasks.extend(sitemaps::tasks(seed, port));
    tasks
}

/// A crawl task of `family`, served on `<ordinal>.<host>`, starting at
/// `start` (a path) and expecting `records`.
fn crawl_task(
    (host, family): (&str, &'static str),
    ordinal: u32,
    port: Option<u16>,
    start: &str,
    description: &'static str,
    records: Vec<Record>,
) -> Task {
    let task_host = format!("{ordinal}.{host}");
    Task {
        entry: Entry {
            id: format!("{family}/{ordinal}"),
            family,
            kind: Kind::Crawl,
            start_url: format!("{}{start}", task::origin(&task_host, port)),
            description,
            account: None,
            time_steps: None,
        },
        host: task_host,
        expected: Expected::records(records),
    }
}

/// An HTML document with `title` and `body`.
fn document(title: &str, body: &str) -> String {
    format!(
        "<!doctype html>\n<html lang=\"en\">\n<head><meta charset=\"utf-8\"><title>{title}</title></head>\n<body>\n{body}</body>\n</html>\n"
    )
}

const ADJECTIVES: [&str; 16] = [
    "Amber", "Brisk", "Cedar", "Dusky", "Ember", "Frosted", "Gilded", "Hazel", "Ivory", "Jade",
    "Keen", "Lunar", "Mellow", "Nimble", "Olive", "Plush",
];
const NOUNS: [&str; 16] = [
    "Anchor", "Basket", "Candle", "Desk", "Easel", "Flask", "Globe", "Hammock", "Inkwell", "Jug",
    "Kettle", "Lamp", "Mirror", "Notebook", "Ottoman", "Pitcher",
];

/// A name drawn from the seed at `place`: an adjective and a noun.
fn name(seed: u64, place: &[u64]) -> String {
    let word = |part, words: &[&'static str; 16]| {
        let mut at = place.to_vec();
        at.push(part);
        words[usize::try_from(generator::below(seed, &at, 16))
            .expect("invariant: a word index is below 16")]
    };
    format!("{} {}", word(1, &ADJECTIVES), word(2, &NOUNS))
}

/// A number written without a sign or leading zeros, so each page has one
/// URL.
fn canonical_number(text: &str) -> Option<u64> {
    let number = text.parse::<u64>().ok()?;
    (number.to_string() == text).then_some(number)
}

/// The number of an ID `<prefix><digits>`, `digits` of them, within
/// `1..=last`.
fn numbered(id: &str, prefix: &str, digits: usize, last: u64) -> Option<u64> {
    let text = id.strip_prefix(prefix)?;
    let number = text.parse::<u64>().ok()?;
    (text.len() == digits
        && text.bytes().all(|b| b.is_ascii_digit())
        && (1..=last).contains(&number))
    .then_some(number)
}

/// Counts per task host, such as the requests to a trap.
#[derive(Default)]
struct Counts(Mutex<BTreeMap<String, u64>>);

impl Counts {
    /// Adds one to `host`'s count and returns the new count.
    fn add(&self, host: &str) -> u64 {
        let mut counts = self.0.lock();
        let count = counts.entry(host.to_owned()).or_default();
        *count += 1;
        *count
    }
}
