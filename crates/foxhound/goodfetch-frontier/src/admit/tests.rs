#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use goodfetch_engine::egress::Policy;
use goodfetch_engine::runs::CompletenessReason;
use goodfetch_protocol::foxhound::frontier::{Enqueued, HttpMethod, NotAddedReason, PathBudget};
use goodfetch_protocol::run::{AxisReason, Completeness};
use rusqlite::Connection;
use url::Url;

use super::{Candidate, Entering, Keyed, Link, admit, admit_keyed, key_link};
use crate::bounds::{self, Bounds, completeness_reasons};
use crate::identity::{Declared, TABLES};
use crate::invariants::Invariants;
use crate::scope::{self, Narrowing, Patterns, Scope, Strategy};
use crate::staged::Admitting;

/// A run file holding the Frontier's visited set, in memory, whose crawl
/// requests the test numbers from 1.
struct Frontier {
    connection: Connection,
    scope: Scope,
    policy: Policy,
    added: u32,
}

/// The domain policy that allows every host: the settings' defaults.
fn every_host() -> Policy {
    Policy::new(&["*"], &[] as &[&str]).unwrap()
}

impl Frontier {
    fn new() -> Self {
        Self::of(Strategy::SameHostname)
    }

    /// A Frontier of `strategy` under the domain policy allowing `allowed`
    /// and blocking `blocked`.
    fn policed(strategy: Strategy, allowed: &[&str], blocked: &[&str]) -> Self {
        Self {
            policy: Policy::new(allowed, blocked).unwrap(),
            ..Self::of(strategy)
        }
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
            policy: every_host(),
            added: 0,
        }
    }

    fn admit(&mut self, candidate: Candidate<'_>) -> Enqueued {
        let added = &mut self.added;
        // As the Run's start or resume builds them, after it records the
        // Bounds.
        let invariants =
            Invariants::new(self.scope.clone(), Bounds::read(&self.connection).unwrap()).unwrap();
        admit(
            &self.connection,
            &invariants,
            &self.policy,
            candidate,
            |_| {
                *added += 1;
                Ok(added.to_string())
            },
        )
        .unwrap()
    }

    fn seed(&mut self, url: &str) -> Enqueued {
        self.admit(Candidate::Seed {
            url,
            declared: get(),
        })
    }

    /// The Run's invariants, as its start builds them.
    fn invariants(&self) -> Invariants {
        Invariants::new(self.scope.clone(), Bounds::read(&self.connection).unwrap()).unwrap()
    }

    /// Admits the link `href`, keyed as `keyed`, of the page at
    /// `http://shop.test/`, at depth 1: the part of its admission that
    /// reads the run file.
    fn admit_keyed(&mut self, href: &str, keyed: &Keyed) -> Enqueued {
        let invariants = self.invariants();
        let added = &mut self.added;
        let from = Url::parse("http://shop.test/").unwrap();
        admit_keyed(
            &self.connection,
            &invariants,
            (href, keyed),
            Entering {
                from: Some(&from),
                depth: 1,
            },
            &mut Admitting::RunFile,
            |_| {
                *added += 1;
                Ok(added.to_string())
            },
        )
        .unwrap()
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
        keeps_fragment: false,
    }
}

/// Keys the link `href` of a UTF-8 page at `http://shop.test/` under
/// `invariants` and the domain policy `policy`, with nothing else: the
/// part of its admission its page's task runs, which is given no run file.
fn keyed(invariants: &Invariants, policy: &Policy, href: &str) -> Keyed {
    key_link(
        invariants,
        policy,
        Link {
            href,
            base: &Url::parse("http://shop.test/").unwrap(),
            encoding: encoding_rs::UTF_8,
            keeps_fragment: false,
            narrowing: Narrowing::none(),
        },
    )
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
/// A link's `href` resolves against its document's base URL with its
/// query in the document's encoding, the URL it is fetched at and keyed
/// by: a `Shift_JIS` page's link is a request apart from a handler's enqueue
/// of the same text, which is UTF-8, and the call's `include` narrows the
/// scope (ADR 0120).
///
/// Covers: story:6-232, story:6-233
#[test]
fn a_links_href_is_admitted_at_its_url_in_its_documents_encoding() {
    let mut frontier = Frontier::new();
    frontier.seed("http://shop.test/");
    let base = Url::parse("http://shop.test/books/").unwrap();
    let only_search = Patterns::globs(&["**/search?*".to_owned()]).unwrap();
    let none = Patterns::default();
    let mut link = |href: &str| {
        frontier.admit(Candidate::Href {
            href,
            base: &base,
            encoding: encoding_rs::SHIFT_JIS,
            keeps_fragment: false,
            from: &base,
            narrowing: Narrowing {
                include: &only_search,
                exclude: &none,
            },
            depth: 1,
        })
    };

    let answers = [
        link("../search?q=日本"),
        link("/search?q=日本"),
        link("../about"),
    ];
    let enqueued = frontier.enqueue("/search?q=日本", 1);

    assert_eq!(
        (answers, enqueued),
        (
            [
                added("2", "http://shop.test/search?q=%93%FA%96{"),
                not_added(NotAddedReason::Duplicate, "/search?q=日本"),
                not_added(NotAddedReason::OutOfScope, "../about"),
            ],
            added("3", "http://shop.test/search?q=%E6%97%A5%E6%9C%AC"),
        )
    );
}

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
                keeps_fragment: false,
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
            count: Some(count),
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

fn budget(pattern: &str, max_requests: u64) -> PathBudget {
    PathBudget {
        pattern: pattern.to_owned(),
        max_requests,
    }
}

/// `crawl.path_budgets` matches each entry's glob against a URL's path and
/// query: the first entry a URL matches counts it, seeds included, and one
/// past its count is not added with `path_budget`; a URL the first entry
/// counted is never counted by a later one it also matches, and a
/// duplicate is no refusal. Each entry that refused a URL makes the Run
/// `incomplete`, its reason naming the pattern and the counts (CO-08).
///
/// Covers: story:6-229, setting:crawl.path_budgets
#[test]
fn a_path_budget_refuses_past_its_count_and_only_the_first_matching_entry_counts() {
    let mut frontier = Frontier::new();
    Bounds {
        path_budgets: vec![budget("/shop?*", 2), budget("/shop**", 1)],
        ..Bounds::default()
    }
    .record(&frontier.connection)
    .unwrap();

    let answers = [
        frontier.seed("http://shop.test/shop?colour=red"),
        frontier.enqueue("/shop?size=m", 1),
        frontier.enqueue("/shop?colour=blue", 1),
        frontier.enqueue("/shop?size=m", 1),
        frontier.enqueue("/shop", 1),
        frontier.enqueue("/shop/1", 1),
        frontier.enqueue("/shop/2", 1),
        frontier.enqueue("/about?shop=1", 1),
    ];

    let refused = |message: &str, count: u64| CompletenessReason {
        completeness: Completeness::Incomplete,
        reason: AxisReason {
            code: "crawl.path_budgets".to_owned(),
            message: message.to_owned(),
            count: Some(count),
            call_id: None,
        },
    };
    assert_eq!(
        (
            answers.to_vec(),
            completeness_reasons(&frontier.connection).unwrap()
        ),
        (
            vec![
                added("1", "http://shop.test/shop?colour=red"),
                added("2", "http://shop.test/shop?size=m"),
                not_added(NotAddedReason::PathBudget, "/shop?colour=blue"),
                not_added(NotAddedReason::Duplicate, "/shop?size=m"),
                added("3", "http://shop.test/shop"),
                not_added(NotAddedReason::PathBudget, "/shop/1"),
                not_added(NotAddedReason::PathBudget, "/shop/2"),
                added("4", "http://shop.test/about?shop=1"),
            ],
            vec![
                refused(
                    "`crawl.path_budgets`'s entry `/shop?*` refused 1 URL past its 2 requests the crawl would otherwise have admitted.",
                    1,
                ),
                refused(
                    "`crawl.path_budgets`'s entry `/shop**` refused 2 URLs past its 1 request the crawl would otherwise have admitted.",
                    2,
                ),
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

/// A seed and an enqueue whose host the domain policy refuses, or a
/// subdomain of it, are not added with `domain_policy`, under a scope that
/// reaches every host; another host is added (ADRs 0093 and 0102).
///
/// Covers: story:4-96, setting:egress.blocked_domains
#[test]
fn a_seed_and_an_enqueue_the_domain_policy_refuses_are_not_added_with_domain_policy() {
    let mut frontier = Frontier::policed(Strategy::All, &["*"], &["elsewhere.test"]);

    let answers = [
        frontier.seed("http://shop.test/"),
        frontier.seed("http://elsewhere.test/a"),
        frontier.seed("http://news.elsewhere.test/"),
        frontier.enqueue("http://elsewhere.test/b", 1),
        frontier.enqueue("http://outlet.test/", 1),
    ];

    assert_eq!(
        answers.to_vec(),
        vec![
            added("1", "http://shop.test/"),
            not_added(NotAddedReason::DomainPolicy, "http://elsewhere.test/a"),
            not_added(NotAddedReason::DomainPolicy, "http://news.elsewhere.test/"),
            not_added(NotAddedReason::DomainPolicy, "http://elsewhere.test/b"),
            added("2", "http://outlet.test/"),
        ]
    );
}

/// A URL is judged by the scope, then the domain policy, then the caps on
/// its shape (ADR 0120): a link off the site that the policy refuses too
/// is `out_of_scope`, a refused seed widening nothing; an over-long link in
/// scope that the policy refuses is `domain_policy`, and an allowed one
/// `max_url_length`. The scope ignores one leading `www.` and the policy
/// does not: under a policy allowing `www.shop.test` alone, its bare host
/// is in scope and refused.
///
/// Covers: story:4-95, story:4-96, setting:egress.allowed_domains
#[test]
fn the_scope_judges_first_then_the_domain_policy_then_the_caps() {
    let mut frontier = Frontier::policed(Strategy::SameHostname, &["www.shop.test"], &[]);
    let long_bare = format!("http://shop.test/{}", "a".repeat(3_000));
    let long_www = format!("http://www.shop.test/{}", "a".repeat(3_000));

    let answers = [
        frontier.seed("http://www.shop.test/"),
        frontier.seed("http://elsewhere.test/"),
        frontier.enqueue("http://elsewhere.test/c", 1),
        frontier.enqueue(&long_bare, 1),
        frontier.enqueue(&long_www, 1),
        frontier.enqueue("http://shop.test/a", 1),
        frontier.enqueue("http://www.shop.test/a", 1),
    ];

    assert_eq!(
        answers.to_vec(),
        vec![
            added("1", "http://www.shop.test/"),
            not_added(NotAddedReason::DomainPolicy, "http://elsewhere.test/"),
            not_added(NotAddedReason::OutOfScope, "http://elsewhere.test/c"),
            not_added(NotAddedReason::DomainPolicy, &long_bare),
            not_added(NotAddedReason::MaxUrlLength, &long_www),
            not_added(NotAddedReason::DomainPolicy, "http://shop.test/a"),
            added("2", "http://www.shop.test/a"),
        ]
    );
}

/// The domain policy reads the host as the WHATWG parser leaves it (ADR
/// 0120): a refused name in Unicode, in capitals or in punycode, and a
/// refused address in hex, decimal, octal or shortened form, are each
/// refused; a neighbouring name and address are added.
///
/// Covers: story:4-95, setting:egress.blocked_domains
#[test]
fn every_spelling_of_a_refused_host_is_refused() {
    let mut frontier = Frontier::policed(
        Strategy::All,
        &["*"],
        &["bücher.test", "127.0.0.1", "192.0.2.1"],
    );

    let answers: Vec<Enqueued> = [
        "http://xn--bcher-kva.test/",
        "http://BÜCHER.test/",
        "http://0x7f000001/",
        "http://2130706433/",
        "http://127.1/",
        "http://0300.0.2.1/",
        "http://127.0.0.2/",
        "http://buecher.test/",
    ]
    .into_iter()
    .map(|url| frontier.seed(url))
    .collect();

    assert_eq!(
        answers,
        vec![
            not_added(NotAddedReason::DomainPolicy, "http://xn--bcher-kva.test/"),
            not_added(NotAddedReason::DomainPolicy, "http://BÜCHER.test/"),
            not_added(NotAddedReason::DomainPolicy, "http://0x7f000001/"),
            not_added(NotAddedReason::DomainPolicy, "http://2130706433/"),
            not_added(NotAddedReason::DomainPolicy, "http://127.1/"),
            not_added(NotAddedReason::DomainPolicy, "http://0300.0.2.1/"),
            added("1", "http://127.0.0.2/"),
            added("2", "http://buecher.test/"),
        ]
    );
}

mod model;

/// A page's link is keyed on its page's task, from the link and the Run's
/// invariants alone, and admitted where the Run's state is held (issue
/// 2244). The Scope's seeds are that state: a link to a host no seed
/// reaches is `out_of_scope`, and once a seed on that host is admitted
/// while the crawl runs, a link keyed after it is added. A link keyed
/// before the seed and admitted after it is added too, since its key holds
/// what it asks the seeds, and never an answer older than its admission.
///
/// Covers: story:6-211
#[test]
fn a_seed_admitted_while_the_crawl_runs_widens_the_scope_for_the_links_keyed_after_it() {
    let mut frontier = Frontier::new();
    frontier.seed("http://shop.test/");
    let (invariants, policy) = (frontier.invariants(), every_host());
    let early = keyed(&invariants, &policy, "http://other.test/early");
    let before = keyed(&invariants, &policy, "http://other.test/a");
    let before = frontier.admit_keyed("http://other.test/a", &before);

    frontier.seed("http://other.test/");
    let after = keyed(&invariants, &policy, "http://other.test/a");
    let after = frontier.admit_keyed("http://other.test/a", &after);
    let early = frontier.admit_keyed("http://other.test/early", &early);

    assert_eq!(
        (before, after, early),
        (
            not_added(NotAddedReason::OutOfScope, "http://other.test/a"),
            added("3", "http://other.test/a"),
            added("4", "http://other.test/early"),
        )
    );
}

/// The part of a link's admission that its page's task runs tells what it
/// judged in the place the whole admission tells it (issue 2244): a link
/// the domain policy refuses stays `out_of_scope` while no seed reaches
/// it, and a cap on its URL's shape comes after the Run's count of
/// requests, though the policy and the cap are judged first, on the task.
#[test]
fn a_keyed_links_verdicts_keep_the_order_of_the_whole_admission() {
    let mut frontier = Frontier::policed(Strategy::SameHostname, &["*"], &["blocked.test"]);
    frontier.seed("http://shop.test/");
    Bounds {
        max_requests: Some(2),
        max_url_length: Some(30),
        ..Bounds::default()
    }
    .record(&frontier.connection)
    .unwrap();
    let invariants = frontier.invariants();
    let policy = Policy::new(&["*"], &["blocked.test"]).unwrap();
    let long = "http://shop.test/a-path-past-the-cap";
    let link = |frontier: &mut Frontier, href: &str| {
        frontier.admit_keyed(href, &keyed(&invariants, &policy, href))
    };

    let answers = [
        link(&mut frontier, "http://blocked.test/"),
        link(&mut frontier, long),
        link(&mut frontier, "http://shop.test/a"),
        link(&mut frontier, long),
        link(&mut frontier, "mailto:shop@shop.test"),
    ];

    assert_eq!(
        answers,
        [
            not_added(NotAddedReason::OutOfScope, "http://blocked.test/"),
            not_added(NotAddedReason::MaxUrlLength, long),
            added("2", "http://shop.test/a"),
            not_added(NotAddedReason::MaxRequests, long),
            not_added(NotAddedReason::UnsupportedScheme, "mailto:shop@shop.test"),
        ]
    );
}
