#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use goodfetch_engine::runs::CompletenessReason;
use goodfetch_protocol::foxhound::frontier::{Enqueued, HttpMethod, NotAddedReason};
use goodfetch_protocol::run::{AxisReason, Completeness};
use rusqlite::Connection;
use url::Url;

use super::{Candidate, admit};
use crate::bounds::{self, Bounds, completeness_reasons};
use crate::identity::{Declared, TABLES};
use crate::scope::{self, Narrowing, Patterns, Scope, Strategy};

/// A run file holding the Frontier's visited set, in memory, whose crawl
/// requests the test numbers from 1.
struct Frontier {
    connection: Connection,
    scope: Scope,
    added: u32,
}

impl Frontier {
    fn new() -> Self {
        Self::of(Strategy::SameHostname)
    }

    fn of(strategy: Strategy) -> Self {
        let connection = Connection::open_in_memory().unwrap();
        for statement in TABLES
            .statements
            .iter()
            .chain(scope::TABLES.statements)
            .chain(bounds::TABLES.statements)
            .chain(crate::rules::TABLES.statements)
        {
            connection.execute(statement, []).unwrap();
        }
        Self {
            connection,
            scope: Scope::new(strategy, Patterns::default(), Patterns::default()),
            added: 0,
        }
    }

    fn admit(&mut self, candidate: Candidate<'_>) -> Enqueued {
        let added = &mut self.added;
        admit(&self.connection, &self.scope, candidate, |_| {
            *added += 1;
            Ok(added.to_string())
        })
        .unwrap()
    }

    fn seed(&mut self, url: &str) -> Enqueued {
        self.admit(Candidate::Seed {
            url,
            declared: get(),
        })
    }

    /// Enqueues `url` at `depth` from a document at `http://shop.test/`.
    fn enqueue(&mut self, url: &str, depth: u32) -> Enqueued {
        let base = Url::parse("http://shop.test/").unwrap();
        self.admit(Candidate::Enqueue {
            url,
            base: &base,
            from: &base,
            declared: get(),
            narrowing: Narrowing::none(),
            depth,
        })
    }
}

/// A `GET` with no body and no declared key.
fn get() -> Declared<'static> {
    static GET: std::sync::LazyLock<HttpMethod> = std::sync::LazyLock::new(HttpMethod::get);
    Declared {
        method: &GET,
        body: None,
        key: None,
    }
}

fn added(id: &str, url: &str) -> Enqueued {
    Enqueued::Added {
        id: id.to_owned(),
        url: url.to_owned(),
    }
}

fn not_added(reason: NotAddedReason, url: &str) -> Enqueued {
    Enqueued::NotAdded {
        reason,
        url: url.to_owned(),
    }
}

/// Covers: story:6-201, story:6-202, story:6-211
#[test]
fn each_seed_is_answered_added_with_its_id_or_not_added_with_its_reason() {
    let mut frontier = Frontier::new();
    let answers: Vec<Enqueued> = [
        "http://bücher.example/a",
        "http://xn--bcher-kva.example/a",
        "/a",
        "mailto:books@example.com",
        "http://0xc0.0.2.1/",
        "HTTP://192.0.2.1:80/",
        "http://",
    ]
    .into_iter()
    .map(|url| frontier.seed(url))
    .collect();

    assert_eq!(
        answers,
        [
            added("1", "http://xn--bcher-kva.example/a"),
            not_added(NotAddedReason::Duplicate, "http://xn--bcher-kva.example/a"),
            not_added(NotAddedReason::InvalidUrl, "/a"),
            not_added(
                NotAddedReason::UnsupportedScheme,
                "mailto:books@example.com"
            ),
            added("2", "http://192.0.2.1/"),
            not_added(NotAddedReason::Duplicate, "HTTP://192.0.2.1:80/"),
            not_added(NotAddedReason::InvalidUrl, "http://"),
        ]
    );
}

/// Covers: story:6-231
#[test]
fn an_enqueue_resolves_against_its_documents_base_and_is_answered_as_a_seed_is() {
    let base = Url::parse("http://links.test/library/shelves/").unwrap();
    let mut frontier = Frontier::new();
    frontier.seed("http://links.test/library/shelves/book/1");

    let answers: Vec<Enqueued> = ["book/1", "book/2", "../book/2", "javascript:void(0)"]
        .into_iter()
        .map(|url| {
            frontier.admit(Candidate::Enqueue {
                url,
                base: &base,
                from: &base,
                declared: get(),
                narrowing: Narrowing::none(),
                depth: 1,
            })
        })
        .collect();

    assert_eq!(
        answers,
        [
            not_added(NotAddedReason::Duplicate, "book/1"),
            added("2", "http://links.test/library/shelves/book/2"),
            added("3", "http://links.test/library/book/2"),
            not_added(NotAddedReason::UnsupportedScheme, "javascript:void(0)"),
        ]
    );
}

/// A URL whose key another admitted URL has is one request, while the
/// request added is fetched at its own URL: its parameters in their order,
/// its tracking parameters kept.
///
/// Covers: story:6-213
#[test]
fn a_url_of_an_admitted_key_is_a_duplicate_and_the_request_keeps_its_own_url() {
    let mut frontier = Frontier::new();

    let answers: Vec<Enqueued> = [
        "http://shop.test/p?utm_source=news&b=2&a=1#reviews",
        "http://shop.test/p?a=1&b=2",
        "http://shop.test/p?a=1&gclid=x&&b=2&fbclid=y",
        "http://shop.test/p;JSESSIONID=0A1B?b=2&a=1",
        "http://shop.test/p?a=2&b=1",
    ]
    .into_iter()
    .map(|url| frontier.seed(url))
    .collect();

    assert_eq!(
        answers,
        [
            added("1", "http://shop.test/p?utm_source=news&b=2&a=1#reviews"),
            not_added(NotAddedReason::Duplicate, "http://shop.test/p?a=1&b=2"),
            not_added(
                NotAddedReason::Duplicate,
                "http://shop.test/p?a=1&gclid=x&&b=2&fbclid=y"
            ),
            not_added(
                NotAddedReason::Duplicate,
                "http://shop.test/p;JSESSIONID=0A1B?b=2&a=1"
            ),
            added("2", "http://shop.test/p?a=2&b=1"),
        ]
    );
}

/// A request's identity is its method, its URL's key and its body's hash:
/// a POST to one URL with two bodies is two requests, the same body twice
/// is one, and a GET of that URL is another.
///
/// Covers: story:6-212
#[test]
fn a_post_with_two_bodies_is_two_requests_and_the_same_body_twice_is_one() {
    let post = HttpMethod::parse("post").unwrap();
    let mut frontier = Frontier::new();
    let mut seed = |method: &HttpMethod, body: Option<&[u8]>| {
        frontier.admit(Candidate::Seed {
            url: "http://api.test/search",
            declared: Declared {
                method,
                body,
                key: None,
            },
        })
    };

    let answers = [
        seed(&post, Some(b"{\"page\":1}")),
        seed(&post, Some(b"{\"page\":2}")),
        seed(&post, Some(b"{\"page\":1}")),
        seed(&post, None),
        seed(&HttpMethod::get(), None),
        seed(&post, Some(b"")),
    ];

    assert_eq!(
        answers,
        [
            added("1", "http://api.test/search"),
            added("2", "http://api.test/search"),
            not_added(NotAddedReason::Duplicate, "http://api.test/search"),
            added("3", "http://api.test/search"),
            added("4", "http://api.test/search"),
            not_added(NotAddedReason::Duplicate, "http://api.test/search"),
        ]
    );
}

/// A declared `key` replaces the identity: two URLs of one key are one
/// request, one URL under two keys two, and a key never meets an identity
/// the Frontier made, even one it spells.
///
/// Covers: story:6-214
#[test]
fn a_declared_key_replaces_the_identity() {
    let mut frontier = Frontier::new();
    let mut seed = |url: &str, key: Option<&str>| {
        frontier.admit(Candidate::Seed {
            url,
            declared: Declared { key, ..get() },
        })
    };

    let answers = [
        seed("http://shop.test/item/1?ref=home", Some("item-1")),
        seed("http://shop.test/item/1?ref=search", Some("item-1")),
        seed("http://shop.test/item/2", Some("item-2")),
        seed("http://shop.test/item/2", Some("item-2b")),
        seed("http://shop.test/item/3", None),
        seed(
            "http://shop.test/item/4",
            Some("GET http://shop.test/item/3"),
        ),
    ];

    assert_eq!(
        answers,
        [
            added("1", "http://shop.test/item/1?ref=home"),
            not_added(
                NotAddedReason::Duplicate,
                "http://shop.test/item/1?ref=search"
            ),
            added("2", "http://shop.test/item/2"),
            added("3", "http://shop.test/item/2"),
            added("4", "http://shop.test/item/3"),
            added("5", "http://shop.test/item/4"),
        ]
    );
}

/// Seeds define the scope and an enqueue is measured against every seed:
/// a link off every seed's host is not added with `out_of_scope`, before
/// any other check, and a seed is never out of scope (ADR 0093).
///
/// Covers: story:6-224
#[test]
fn an_enqueue_outside_every_seeds_scope_is_out_of_scope() {
    let base = Url::parse("http://shop.test/").unwrap();
    let mut frontier = Frontier::of(Strategy::SameHostname);
    let seeds = [
        frontier.seed("http://shop.test/"),
        frontier.seed("http://www.outlet.test/"),
    ];

    let answers: Vec<Enqueued> = [
        "/cart",
        "http://outlet.test/sale",
        "http://blog.shop.test/",
        "http://elsewhere.test/",
        "http://elsewhere.test/",
        "ftp://elsewhere.test/",
    ]
    .into_iter()
    .map(|url| {
        frontier.admit(Candidate::Enqueue {
            url,
            base: &base,
            from: &base,
            declared: get(),
            narrowing: Narrowing::none(),
            depth: 1,
        })
    })
    .collect();

    assert_eq!(
        (seeds.to_vec(), answers),
        (
            vec![
                added("1", "http://shop.test/"),
                added("2", "http://www.outlet.test/"),
            ],
            vec![
                added("3", "http://shop.test/cart"),
                added("4", "http://outlet.test/sale"),
                not_added(NotAddedReason::OutOfScope, "http://blog.shop.test/"),
                not_added(NotAddedReason::OutOfScope, "http://elsewhere.test/"),
                not_added(NotAddedReason::OutOfScope, "http://elsewhere.test/"),
                not_added(NotAddedReason::UnsupportedScheme, "ftp://elsewhere.test/"),
            ]
        )
    );
}

/// The reason `name` gives, having refused `count` URLs.
fn reason(name: &str, completeness: Completeness, count: u64) -> CompletenessReason {
    let urls = if count == 1 { "URL" } else { "URLs" };
    CompletenessReason {
        completeness,
        reason: AxisReason {
            code: name.to_owned(),
            message: format!(
                "`{name}` refused {count} {urls} the crawl would otherwise have admitted."
            ),
            call_id: None,
        },
    }
}

/// `crawl.max_depth` refuses an enqueue deeper than it, seeds at depth 0,
/// and `crawl.max_requests` one past the requests admitted, seeds
/// included, each with its reason; a duplicate stays a `duplicate` and an
/// enqueue outside the scope `out_of_scope`, so only what the crawl would
/// otherwise have fetched counts as cut, and the run file reports each
/// bound's refusals as making the Run `incomplete`.
///
/// Covers: story:6-227, setting:crawl.max_depth, setting:crawl.max_requests
#[test]
fn a_bound_refuses_past_it_with_its_reason_and_counts_what_it_cut() {
    let mut frontier = Frontier::new();
    Bounds {
        max_depth: Some(1),
        max_requests: Some(3),
        ..Bounds::default()
    }
    .record(&frontier.connection)
    .unwrap();

    let answers = [
        frontier.seed("http://shop.test/"),
        frontier.enqueue("/shelf", 1),
        frontier.enqueue("/shelf/book", 2),
        frontier.enqueue("/shelf", 2),
        frontier.enqueue("http://elsewhere.test/", 2),
        frontier.enqueue("/cart", 1),
        frontier.enqueue("/account", 1),
        frontier.enqueue("/basket", 0),
        frontier.enqueue("/cart", 1),
    ];

    assert_eq!(
        (
            answers.to_vec(),
            completeness_reasons(&frontier.connection).unwrap()
        ),
        (
            vec![
                added("1", "http://shop.test/"),
                added("2", "http://shop.test/shelf"),
                not_added(NotAddedReason::MaxDepth, "/shelf/book"),
                not_added(NotAddedReason::Duplicate, "/shelf"),
                not_added(NotAddedReason::OutOfScope, "http://elsewhere.test/"),
                added("3", "http://shop.test/cart"),
                not_added(NotAddedReason::MaxRequests, "/account"),
                not_added(NotAddedReason::MaxRequests, "/basket"),
                not_added(NotAddedReason::Duplicate, "/cart"),
            ],
            vec![
                reason("crawl.max_depth", Completeness::Incomplete, 1),
                reason("crawl.max_requests", Completeness::Incomplete, 2),
            ]
        )
    );
}

/// The built-in caps refuse a seed and an enqueue past them, after the
/// bounds, and their refusals make the Run `unverified`; an over-long link
/// to another host stays `out_of_scope` and is no cap's (ADR 0120).
///
/// Covers: story:6-228, setting:crawl.max_url_length, setting:crawl.max_path_segments, setting:crawl.max_segment_repeats
#[test]
fn a_cap_refuses_what_would_otherwise_be_admitted_and_leaves_the_run_unverified() {
    let mut frontier = Frontier::new();
    let long = format!("http://shop.test/{}", "a".repeat(3_000));
    let long_elsewhere = format!("http://elsewhere.test/{}", "a".repeat(3_000));
    let deep = (1..=25)
        .map(|n| format!("/{n}"))
        .collect::<Vec<_>>()
        .concat();

    let answers = [
        frontier.seed(&long),
        frontier.seed("http://shop.test/"),
        frontier.enqueue(&long_elsewhere, 1),
        frontier.enqueue(&deep, 1),
        frontier.enqueue("/archive/more/more/more/", 1),
        frontier.enqueue("/archive/more/more/", 1),
    ];

    assert_eq!(
        (
            answers.to_vec(),
            completeness_reasons(&frontier.connection).unwrap()
        ),
        (
            vec![
                not_added(NotAddedReason::MaxUrlLength, &long),
                added("1", "http://shop.test/"),
                not_added(NotAddedReason::OutOfScope, &long_elsewhere),
                not_added(NotAddedReason::MaxPathSegments, &deep),
                not_added(
                    NotAddedReason::MaxSegmentRepeats,
                    "/archive/more/more/more/"
                ),
                added("2", "http://shop.test/archive/more/more/"),
            ],
            vec![
                reason("crawl.max_path_segments", Completeness::Unverified, 1),
                reason("crawl.max_segment_repeats", Completeness::Unverified, 1),
                reason("crawl.max_url_length", Completeness::Unverified, 1),
            ]
        )
    );
}

/// Bounds recorded again, as a resume records its new settings, take
/// effect from then on, while the requests already admitted still count.
///
/// Covers: story:6-227, setting:crawl.max_requests
#[test]
fn bounds_recorded_again_apply_to_the_requests_already_admitted() {
    let mut frontier = Frontier::new();
    let bounded = |max_requests| Bounds {
        max_requests: Some(max_requests),
        ..Bounds::default()
    };
    bounded(1).record(&frontier.connection).unwrap();
    let first = [
        frontier.seed("http://shop.test/"),
        frontier.enqueue("/a", 1),
    ];
    bounded(2).record(&frontier.connection).unwrap();
    let resumed = [frontier.enqueue("/a", 1), frontier.enqueue("/b", 1)];

    assert_eq!(
        (
            first.to_vec(),
            resumed.to_vec(),
            Bounds::read(&frontier.connection).unwrap()
        ),
        (
            vec![
                added("1", "http://shop.test/"),
                not_added(NotAddedReason::MaxRequests, "/a"),
            ],
            vec![
                added("2", "http://shop.test/a"),
                not_added(NotAddedReason::MaxRequests, "/b"),
            ],
            bounded(2)
        )
    );
}

mod model;
