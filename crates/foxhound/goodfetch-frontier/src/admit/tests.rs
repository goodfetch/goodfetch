use goodfetch_protocol::foxhound::frontier::{Enqueued, HttpMethod, NotAddedReason};
use rusqlite::Connection;
use url::Url;

use super::{Candidate, admit};
use crate::identity::{Declared, TABLES};

/// A run file holding the Frontier's visited set, in memory, whose crawl
/// requests the test numbers from 1.
struct Frontier {
    connection: Connection,
    added: u32,
}

impl Frontier {
    fn new() -> Self {
        let connection = Connection::open_in_memory().unwrap();
        for statement in TABLES.statements {
            connection.execute(statement, []).unwrap();
        }
        Self {
            connection,
            added: 0,
        }
    }

    fn admit(&mut self, candidate: Candidate<'_>) -> Enqueued {
        let added = &mut self.added;
        admit(&self.connection, candidate, |_| {
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
                declared: get(),
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

mod model;
