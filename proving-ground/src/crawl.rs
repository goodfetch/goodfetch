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
//! - `scale`: millions of seeded pages, with sitemaps.
//!
//! Every page and expected result is a pure function of the seed and the
//! page's place; what a family keeps between requests is what it saw a
//! crawl do, which it records as findings in the observations, for the
//! oracle.

mod limits;
mod parameters;
mod scale;
mod statuses;
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
use crate::task::{Entry, Expected, Finding, Kind, Record, Task};

pub(crate) use limits::Held;

/// The family host names, each with its family's name.
const FAMILIES: [(&str, &str); 5] = [
    (statuses::HOST, statuses::FAMILY),
    (traps::HOST, traps::FAMILY),
    (limits::HOST, limits::FAMILY),
    (parameters::HOST, parameters::FAMILY),
    (scale::HOST, scale::FAMILY),
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
}

impl Families {
    pub(crate) fn new() -> Self {
        Self {
            stopwatch: Stopwatch::start(),
            statuses: statuses::State::default(),
            traps: traps::State::default(),
            limits: limits::State::default(),
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
            _ => scale::respond(&visit),
        })
    }
}

/// The family whose host name, or one of whose task host names, `host` is.
fn family_of(host: &str) -> Option<&'static str> {
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
        .chain(
            tasks(seed, 0)
                .into_iter()
                .map(|task| (task.host, Some(task.entry.id))),
        )
        .collect()
}

/// Every Crawl family's tasks, in the task list's order.
pub(crate) fn tasks(seed: u64, port: u16) -> Vec<Task> {
    let mut tasks = statuses::tasks(seed, port);
    tasks.extend(traps::tasks(seed, port));
    tasks.extend(parameters::tasks(seed, port));
    tasks.extend(limits::tasks(seed, port));
    tasks.extend(scale::tasks(seed, port));
    tasks
}

/// A crawl task of `family`, served on `<ordinal>.<host>`, starting at
/// `start` (a path) and expecting `records`.
fn crawl_task(
    (host, family): (&str, &'static str),
    ordinal: u32,
    port: u16,
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
            start_url: format!("http://{task_host}:{port}{start}"),
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
