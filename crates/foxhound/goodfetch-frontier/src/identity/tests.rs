#![expect(
    clippy::disallowed_methods,
    reason = "a test's own statements, on no Shared path (ADR 0140)"
)]
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use goodfetch_engine::egress::Policy;
use goodfetch_protocol::foxhound::frontier::{Enqueued, HttpMethod, NotAddedReason};
use proptest::prelude::*;
use rstest::rstest;
use rusqlite::Connection;
use url::Url;

use goodfetch_protocol::foxhound::outcomes::SkipReason;

use super::{Declared, Hop, TABLES, redirect_hop, redirect_reached, rewrite, url_key};
use crate::admit::{Candidate, admit};
use crate::bounds::{self, Bounds};
use crate::invariants::Invariants;
use crate::scope::{self, Patterns, Scope, Strategy as ScopeStrategy};

/// The domain policy that allows every host: the settings' defaults.
fn every_host() -> Policy {
    Policy::new(&["*"], &[] as &[&str]).unwrap()
}

/// The invariants of `scope` and the built-in bounds, as the Run's start
/// builds them.
fn within(scope: Scope) -> Invariants {
    Invariants::new(scope, Bounds::default()).unwrap()
}

/// A run file holding the visited set and the scope, in memory.
fn run_file() -> Connection {
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
    connection
}

fn key(text: &str) -> String {
    url_key(&Url::parse(text).unwrap())
}

/// Two URLs the key's fixed rules make one request (ADR 0120).
///
/// Covers: story:6-213
#[rstest]
#[case::an_encoded_tilde_lowercase_and_the_tilde("http://a.test/%7e", "http://a.test/~")]
#[case::an_encoded_tilde_uppercase_and_the_tilde("http://a.test/%7E", "http://a.test/~")]
#[case::lowercase_and_uppercase_hexadecimal_digits("http://a.test/%c3%a9", "http://a.test/%C3%A9")]
#[case::encoded_unreserved_characters_in_the_query(
    "http://a.test/?q=%41%2d%5f",
    "http://a.test/?q=A-_"
)]
#[case::the_fragment("http://a.test/page#a", "http://a.test/page#b")]
#[case::a_fragment_and_none("http://a.test/page#a", "http://a.test/page")]
#[case::utm_parameters(
    "http://a.test/p?utm_source=news&id=1&utm_medium=mail&utm_campaign=x",
    "http://a.test/p?id=1"
)]
#[case::gclid_and_fbclid("http://a.test/p?gclid=1&id=2&fbclid=3", "http://a.test/p?id=2")]
#[case::parameter_order("http://a.test/p?b=2&a=1&c=3", "http://a.test/p?a=1&b=2&c=3")]
#[case::empty_pairs("http://a.test/p?&a=1&&b=2&", "http://a.test/p?a=1&b=2")]
#[case::an_empty_query("http://a.test/p?", "http://a.test/p")]
#[case::a_query_of_tracking_parameters_only("http://a.test/p?utm_source=x", "http://a.test/p")]
#[case::a_jsessionid_path_parameter(
    "http://a.test/cart;jsessionid=0123ABC?id=1",
    "http://a.test/cart?id=1"
)]
#[case::a_jsessionid_in_any_case("http://a.test/cart;JSESSIONID=0123ABC", "http://a.test/cart")]
#[case::a_jsessionid_among_other_path_parameters(
    "http://a.test/cart;v=2;jsessionid=0123ABC;w=3",
    "http://a.test/cart;v=2;w=3"
)]
fn the_key_makes_one_request_of(#[case] one: &str, #[case] other: &str) {
    assert_eq!(key(one), key(other));
}

/// Two URLs the key keeps apart, since RFC 3986 or HTTP says they may
/// differ, or since only evidence merges them (ADR 0120).
///
/// Covers: story:6-213
#[rstest]
#[case::an_encoded_slash_and_a_slash("http://a.test/a%2Fb", "http://a.test/a/b")]
#[case::a_plus_and_an_encoded_space("http://a.test/?q=a+b", "http://a.test/?q=a%20b")]
#[case::paths_in_another_case("http://a.test/APPLE", "http://a.test/apple")]
#[case::values_of_one_name_in_another_order("http://a.test/?a=1&a=2", "http://a.test/?a=2&a=1")]
#[case::another_path_parameter("http://a.test/cart;v=2", "http://a.test/cart")]
#[case::a_parameter_named_jsessionid_in_the_query(
    "http://a.test/cart?jsessionid=1",
    "http://a.test/cart"
)]
#[case::a_session_parameter_merged_only_on_evidence(
    "http://a.test/story?sid=1",
    "http://a.test/story?sid=2"
)]
#[case::a_trailing_slash("http://a.test/a", "http://a.test/a/")]
#[case::the_www_label("http://a.test/", "http://www.a.test/")]
#[case::another_scheme("http://a.test/", "https://a.test/")]
#[case::an_index_file("http://a.test/a/index.html", "http://a.test/a/")]
#[case::a_stray_percent_sign_before_an_encoded_digit("http://a.test/?%3%30", "http://a.test/?%30")]
#[case::a_stray_percent_sign_and_an_encoded_percent_sign(
    "http://a.test/?%3%30",
    "http://a.test/?%2530"
)]
fn the_key_keeps_apart(#[case] one: &str, #[case] other: &str) {
    assert_ne!(key(one), key(other));
}

/// The key a URL gets, written out.
///
/// Covers: story:6-213
#[rstest]
#[case::every_rule_at_once(
    "http://a.test/%7e/a%2fb;JSessionID=1;v=2?utm_term=x&b=%41&&a=2&a=1#top",
    "http://a.test/~/a%2Fb;v=2?a=2&a=1&b=A"
)]
#[case::a_url_with_nothing_to_change("https://a.test:8443/a/b?c=d", "https://a.test:8443/a/b?c=d")]
#[case::a_stray_percent_sign_and_a_digit_before_an_encoded_digit(
    "http://a.test/?%3%30",
    "http://a.test/?%3%30"
)]
#[case::a_stray_percent_sign_before_an_encoded_digit("http://a.test/%%30", "http://a.test/%%30")]
#[case::a_stray_percent_sign_before_an_encoded_digit_and_a_digit(
    "http://a.test/%%30A",
    "http://a.test/%%30A"
)]
#[case::a_stray_percent_sign_before_two_encoded_digits(
    "http://a.test/%%30%31",
    "http://a.test/%%301"
)]
#[case::an_encoded_digit_after_a_whole_encoding("http://a.test/%2%35%36", "http://a.test/%2%356")]
#[case::a_jsessionid_on_a_dot_segment(
    "http://a.test/a/.;jsessionid=1/b",
    "http://a.test/a/.;jsessionid=1/b"
)]
#[case::a_jsessionid_on_a_double_dot_segment(
    "http://a.test/a/%2E%2e;JSESSIONID=1/b",
    "http://a.test/a/..;JSESSIONID=1/b"
)]
#[case::a_jsessionid_beside_another_parameter_on_a_dot_segment(
    "http://a.test/a/.;jsessionid=1;v=2/b",
    "http://a.test/a/.;v=2/b"
)]
fn the_key_of(#[case] url: &str, #[case] expected: &str) {
    assert_eq!(key(url), expected);
}

/// `href`s as pages write them: relative and absolute, with the
/// characters the key's rules read.
fn href_like() -> impl Strategy<Value = String> {
    "(https?://[a-z.]{1,8}|//[a-z]{1,4}|\\.{0,2}/)?([a-zA-Z0-9/;=~.%\\-_+é ]|%[0-9a-fA-F]{2}|;jsessionid=){0,16}(\\?([a-zA-Z0-9_=&%;+.~é ]|%[0-9a-fA-F]{2}){0,16})?(#[a-z]{0,4})?"
}

proptest! {
    /// A key is a URL whose key is itself, whatever the `href`.
    ///
    /// Covers: story:6-213
    #[test]
    fn the_key_of_a_key_is_itself(href in prop_oneof![any::<String>(), href_like()]) {
        let base = Url::parse("http://a.test/shelf/page?b=2&a=1").unwrap();
        if let Ok(url) = base.join(&href) {
            let key = url_key(&url);
            prop_assert_eq!(url_key(&Url::parse(&key).unwrap()), key);
        }
    }
}

/// A redirect reaching a key admitted for another request leads to a
/// duplicate, one back to the request's own key or to a key no request
/// holds is followed, and a redirect's final URL joins the visited set, so
/// a later enqueue of it, or another redirect to it, is a duplicate.
///
/// Covers: story:6-220
#[test]
fn a_redirect_to_a_key_admitted_for_another_request_is_a_duplicate() {
    let connection = run_file();
    let get = HttpMethod::get();
    let mut added = 0;
    let invariants = within(Scope::default());
    let mut seed = |url: &str| {
        admit(
            &connection,
            &invariants,
            &every_host(),
            Candidate::Seed {
                url,
                declared: Declared {
                    method: &get,
                    body: None,
                    key: None,
                    keeps_fragment: false,
                },
            },
            |_| {
                added += 1;
                Ok(added.to_string())
            },
        )
        .unwrap()
    };
    seed("http://shop.test/a");
    seed("http://shop.test/b");
    let hop = |crawl_request: &str, url: &str| {
        redirect_hop(
            &connection,
            &Scope::read(&connection).unwrap(),
            &every_host(),
            crawl_request,
            &Url::parse(url).unwrap(),
            &get,
            None,
        )
        .unwrap()
    };

    let before = [
        hop("1", "http://shop.test/b?utm_source=mail"),
        hop("1", "http://shop.test/a?utm_source=mail#top"),
        hop("1", "http://shop.test/c"),
    ];
    redirect_reached(
        &connection,
        "1",
        &Url::parse("http://shop.test/c").unwrap(),
        &get,
        None,
    )
    .unwrap();
    let after = (
        hop("2", "http://shop.test/c#top"),
        hop("1", "http://shop.test/c"),
        seed("http://shop.test/c?gclid=1"),
    );

    assert_eq!(
        (before, after),
        (
            [Hop::Duplicate, Hop::Follow, Hop::Follow],
            (
                Hop::Duplicate,
                Hop::Follow,
                Enqueued::NotAdded {
                    reason: NotAddedReason::Duplicate,
                    url: "http://shop.test/c?gclid=1".to_owned()
                }
            )
        )
    );
}

/// Two redirect chains that reach one key at once, each hop checked
/// before either chain's document arrived, both pass their hop's check;
/// the chain whose document arrives first records the key, and the other
/// is a duplicate at its chain's end, the key left the first's (ADR 0120's
/// #832 note, closed at the chain's end).
///
/// Covers: story:6-220
#[test]
fn of_two_chains_reaching_one_key_at_once_the_second_to_arrive_is_a_duplicate() {
    let connection = run_file();
    let get = HttpMethod::get();
    let slug = Url::parse("http://shop.test/product/1-slug").unwrap();
    // Requests A and B, which both redirect to the slug no request holds.
    for (id, url) in [
        ("A", "http://shop.test/p?id=1"),
        ("B", "http://shop.test/product/1"),
    ] {
        admit(
            &connection,
            &within(Scope::default()),
            &every_host(),
            Candidate::Seed {
                url,
                declared: Declared {
                    method: &get,
                    body: None,
                    key: None,
                    keeps_fragment: false,
                },
            },
            |_| Ok(id.to_owned()),
        )
        .unwrap();
    }
    let hop = |crawl_request: &str| {
        redirect_hop(
            &connection,
            &Scope::read(&connection).unwrap(),
            &every_host(),
            crawl_request,
            &slug,
            &get,
            None,
        )
        .unwrap()
    };
    let reached = |crawl_request: &str| {
        redirect_reached(&connection, crawl_request, &slug, &get, None).unwrap()
    };

    let checked = (hop("A"), hop("B"));
    let arrived = (reached("A"), reached("B"), reached("A"));
    let visitor: String = connection
        .query_row(
            "SELECT crawl_request FROM visited WHERE identity LIKE '%1-slug'",
            [],
            |row| row.get(0),
        )
        .unwrap();

    assert_eq!(
        (checked, arrived, visitor),
        (
            (Hop::Follow, Hop::Follow),
            (Hop::Follow, Hop::Duplicate, Hop::Follow),
            "A".to_owned()
        )
    );
}

/// A redirect hop leaving the crawl's scope ends the request `skipped`
/// with `out_of_scope` before any other check, a hop to a key admitted for
/// another request included, while one inside it is followed (ADR 0093;
/// pitfall U15).
///
/// Covers: story:6-224
#[test]
fn a_redirect_hop_leaving_the_scope_is_out_of_scope() {
    let connection = run_file();
    let get = HttpMethod::get();
    let scope = Scope::new(
        ScopeStrategy::SameHostname,
        Patterns::default(),
        Patterns::default(),
    );
    let mut added = 0;
    for url in ["http://shop.test/a", "http://outlet.test/sale"] {
        let candidate = Candidate::Seed {
            url,
            declared: Declared {
                method: &get,
                body: None,
                key: None,
                keeps_fragment: false,
            },
        };
        admit(
            &connection,
            &within(scope.clone()),
            &every_host(),
            candidate,
            |_| {
                added += 1;
                Ok(added.to_string())
            },
        )
        .unwrap();
    }
    let hop = |url: &str| {
        redirect_hop(
            &connection,
            &scope,
            &every_host(),
            "1",
            &Url::parse(url).unwrap(),
            &get,
            None,
        )
        .unwrap()
    };

    let hops = [
        hop("https://shop.test/b"),
        hop("http://www.shop.test/c"),
        hop("http://blog.shop.test/a"),
        hop("http://elsewhere.test/"),
        hop("http://outlet.test/sale"),
    ];

    assert_eq!(
        hops,
        [
            Hop::Follow,
            Hop::Follow,
            Hop::OutOfScope,
            Hop::OutOfScope,
            Hop::Duplicate
        ]
    );
}

/// The seeds `urls` admitted under `scope`, every host allowed, in the run
/// file `connection`.
fn seeded(connection: &Connection, scope: &Scope, urls: &[&str]) {
    let get = HttpMethod::get();
    for (at, url) in urls.iter().enumerate() {
        let candidate = Candidate::Seed {
            url,
            declared: Declared {
                method: &get,
                body: None,
                key: None,
                keeps_fragment: false,
            },
        };
        admit(
            connection,
            &within(scope.clone()),
            &every_host(),
            candidate,
            |_| Ok((at + 1).to_string()),
        )
        .unwrap();
    }
}

/// A redirect hop is judged by the scope, then by the domain policy, then
/// by the visited set (ADRs 0093 and 0120): a hop to a host the policy
/// refuses, in any spelling the WHATWG parser reads as that host, is
/// `domain_policy`, a key admitted for another request on it included; an
/// off-site hop the policy refuses too stays `out_of_scope`; and a hop to
/// another spelling of a seed's host is in scope and followed.
///
/// Covers: story:4-95, story:4-97, setting:egress.blocked_domains
#[test]
fn a_redirect_hop_the_domain_policy_refuses_is_domain_policy_after_the_scope() {
    let connection = run_file();
    let get = HttpMethod::get();
    let scope = Scope::new(
        ScopeStrategy::SameHostname,
        Patterns::default(),
        Patterns::default(),
    );
    seeded(
        &connection,
        &scope,
        &[
            "http://bücher.test/",
            "http://127.0.0.1/",
            "http://127.0.0.2/",
            "http://bücher.test/taken",
        ],
    );
    let policy = Policy::new(&["*"], &["bücher.test", "127.0.0.1", "elsewhere.test"]).unwrap();
    let hop = |url: &str| {
        redirect_hop(
            &connection,
            &scope,
            &policy,
            "1",
            &Url::parse(url).unwrap(),
            &get,
            None,
        )
        .unwrap()
    };

    let hops = [
        hop("http://xn--bcher-kva.test/a"),
        hop("http://BÜCHER.test/b"),
        hop("http://0x7f000001/c"),
        hop("http://2130706433/d"),
        hop("http://bücher.test/taken"),
        hop("http://elsewhere.test/"),
        hop("http://0x7f000002/e"),
        hop("http://127.2/f"),
    ];

    assert_eq!(
        hops,
        [
            Hop::DomainPolicy,
            Hop::DomainPolicy,
            Hop::DomainPolicy,
            Hop::DomainPolicy,
            Hop::DomainPolicy,
            Hop::OutOfScope,
            Hop::Follow,
            Hop::Follow,
        ]
    );
}

/// A Hook's rewrite is judged as an enqueue is (ADRs 0093 and 0104): one
/// leaving the scope ends its request `skipped` with `out_of_scope`, one in
/// scope to a host the domain policy refuses, in any spelling, with
/// `domain_policy`, an off-site host the policy refuses too staying
/// `out_of_scope`; one in scope to an allowed host is followed.
///
/// Covers: story:4-97, setting:egress.blocked_domains
#[test]
fn a_hook_rewrite_is_judged_by_the_scope_then_the_domain_policy() {
    let connection = run_file();
    let scope = Scope::new(
        ScopeStrategy::SameDomain,
        Patterns::default(),
        Patterns::default(),
    );
    seeded(&connection, &scope, &["http://shop.test/"]);
    let policy = Policy::new(&["*"], &["admin.shop.test", "elsewhere.test"]).unwrap();
    let judged =
        |url: &str| rewrite(&connection, &scope, &policy, &Url::parse(url).unwrap()).unwrap();

    let rewrites = [
        judged("http://shop.test/b"),
        judged("http://cdn.shop.test/a"),
        judged("http://ADMIN.shop.test/"),
        judged("http://elsewhere.test/"),
        judged("http://outlet.test/"),
    ];

    assert_eq!(
        rewrites,
        [
            None,
            None,
            Some(SkipReason::DomainPolicy),
            Some(SkipReason::OutOfScope),
            Some(SkipReason::OutOfScope),
        ]
    );
}

mod url_key_target {
    goodfetch_test_support::fuzz_replay!(crate::identity::fuzz_url_key, "fuzz/url_key");
}

/// What each seed of `urls` is answered, every host allowed, its key
/// keeping the fragment when `keeps_fragment` is on: `true` when added.
fn added_keeping(urls: &[&str], keeps_fragment: bool) -> Vec<bool> {
    let connection = run_file();
    let get = HttpMethod::get();
    let mut added = 0;
    urls.iter()
        .map(|url| {
            let candidate = Candidate::Seed {
                url,
                declared: Declared {
                    method: &get,
                    body: None,
                    key: None,
                    keeps_fragment,
                },
            };
            let answer = admit(
                &connection,
                &within(Scope::default()),
                &every_host(),
                candidate,
                |_| {
                    added += 1;
                    Ok(added.to_string())
                },
            )
            .unwrap();
            matches!(answer, Enqueued::Added { .. })
        })
        .collect()
}

/// A request whose Label keeps fragments (`crawl.keep_url_fragment`, ADR
/// 0120; pitfall U05) keeps its URL's fragment in its key, so a hash-routed
/// app's `#/a` and `#/b` are two requests and `#/a` twice is one, while
/// the other rules still apply: a tracking parameter still splits nothing.
/// With the setting off, the fragment is dropped and they are one.
///
/// Covers: story:6-221
#[test]
fn a_request_keeping_its_fragment_is_one_request_per_fragment() {
    let routes = [
        "http://app.test/#/a",
        "http://app.test/#/b",
        "http://app.test/?utm_source=x#/a",
        "http://app.test/",
    ];

    assert_eq!(
        (added_keeping(&routes, true), added_keeping(&routes, false)),
        (
            vec![true, true, false, true],
            vec![true, false, false, false]
        )
    );
}
