//! The scope check as a pure function of a seed, a URL, the strategy and
//! the patterns (spec #581's Testing Decisions: pure rules in process).

#![expect(
    clippy::disallowed_methods,
    reason = "a test's own statements, on no Shared path (ADR 0140)"
)]

use proptest::prelude::*;
use proptest::strategy::Strategy as _;
use rstest::rstest;
use rusqlite::Connection;
use url::Url;

use super::{Narrowing, Patterns, Scope, Strategy, TABLES};

fn url(text: &str) -> Url {
    Url::parse(text).unwrap()
}

fn patterns(globs: &[&str]) -> Patterns {
    let globs: Vec<String> = globs.iter().map(|glob| (*glob).to_owned()).collect();
    Patterns::globs(&globs).unwrap()
}

/// An in-memory run file whose scope `seeds` widened.
fn seeded(seeds: &[&Url]) -> Connection {
    let connection = run_file();
    for seed in seeds {
        Scope::widen(&connection, seed).unwrap();
    }
    connection
}

fn run_file() -> Connection {
    let connection = Connection::open_in_memory().unwrap();
    for statement in TABLES.statements {
        connection.execute(statement, []).unwrap();
    }
    connection
}

/// Whether each strategy reaches a URL from a seed: host names compared as
/// the WHATWG parser leaves them, one leading `www.` ignored, and an `http`
/// seed's origin admitting its host's `https` origin (ADRs 0093, 0120).
/// The `www.` and `https` pairs, subdomains and `all` are the properties'
/// below; these cases hold the hosts their generators never draw.
///
/// Covers: story:6-224, story:6-226, setting:crawl.scope.strategy
#[rstest]
#[case::the_same_host_by_hostname(
    Strategy::SameHostname,
    "http://shop.test/",
    "http://shop.test/cart",
    true
)]
#[case::only_one_www_label_is_ignored(
    Strategy::SameHostname,
    "http://shop.test/",
    "http://www.www.shop.test/a",
    false
)]
#[case::www_inside_a_host_is_no_www_label(
    Strategy::SameHostname,
    "http://shop.test/",
    "http://wwwshop.test/a",
    false
)]
#[case::another_scheme_and_port_by_hostname(
    Strategy::SameHostname,
    "http://shop.test/",
    "https://shop.test:8443/a",
    true
)]
#[case::a_host_in_another_case_by_hostname(
    Strategy::SameHostname,
    "http://shop.test/",
    "http://SHOP.test/a",
    true
)]
#[case::a_sister_domain_by_hostname(
    Strategy::SameHostname,
    "http://shop.test/",
    "http://shop-outlet.test/a",
    false
)]
#[case::a_sibling_subdomain_by_domain(
    Strategy::SameDomain,
    "http://www.shop.test/",
    "http://cdn.shop.test/a",
    true
)]
#[case::a_sister_domain_by_domain(
    Strategy::SameDomain,
    "http://shop.test/",
    "http://shop-outlet.test/a",
    false
)]
#[case::two_sites_under_a_shared_suffix_by_domain(
    Strategy::SameDomain,
    "https://alice.github.io/",
    "https://bob.github.io/",
    false
)]
#[case::a_subdomain_under_a_shared_suffix_by_domain(
    Strategy::SameDomain,
    "https://alice.github.io/",
    "https://docs.alice.github.io/",
    true
)]
#[case::the_same_address_by_domain(
    Strategy::SameDomain,
    "http://127.0.0.1/",
    "http://127.0.0.1:8080/a",
    true
)]
#[case::another_address_by_domain(
    Strategy::SameDomain,
    "http://127.0.0.1/",
    "http://127.0.0.2/a",
    false
)]
#[case::the_same_origin(Strategy::SameOrigin, "http://shop.test/", "http://shop.test/a", true)]
#[case::another_port_by_origin(
    Strategy::SameOrigin,
    "http://shop.test/",
    "http://shop.test:8080/a",
    false
)]
fn a_strategy_reaches(
    #[case] strategy: Strategy,
    #[case] seed: &str,
    #[case] link: &str,
    #[case] reaches: bool,
) {
    assert_eq!(strategy.reaches(&url(seed), &url(link)), reaches);
}

/// `include` and `exclude` narrow the strategy and never replace it; an
/// `enqueueLinks` call's own patterns narrow it further (ADR 0093).
///
/// Covers: story:6-225, setting:crawl.scope.include, setting:crawl.scope.exclude
#[rstest]
#[case::no_pattern(&[], &[], &[], &[], "http://shop.test/cart", true)]
#[case::an_include_match(&["**/cart"], &[], &[], &[], "http://shop.test/cart", true)]
#[case::no_include_match(&["**/cart"], &[], &[], &[], "http://shop.test/about", false)]
#[case::an_exclude_match(&[], &["**/cart"], &[], &[], "http://shop.test/cart", false)]
#[case::an_exclude_beating_an_include(&["**/cart"], &["**/cart"], &[], &[], "http://shop.test/cart", false)]
#[case::a_calls_include_narrowing(&[], &[], &["**/about"], &[], "http://shop.test/cart", false)]
#[case::a_calls_include_never_widening(&[], &[], &["**/cart"], &[], "http://elsewhere.test/cart", false)]
#[case::a_calls_include_never_widening_the_crawls_include(&["**/about"], &[], &["**/cart"], &[], "http://shop.test/cart", false)]
#[case::a_calls_exclude(&[], &[], &[], &["**/cart"], "http://shop.test/cart", false)]
fn the_patterns_narrow_the_strategy(
    #[case] include: &[&str],
    #[case] exclude: &[&str],
    #[case] call_include: &[&str],
    #[case] call_exclude: &[&str],
    #[case] link: &str,
    #[case] admits: bool,
) {
    let connection = seeded(&[&url("http://shop.test/")]);
    let scope = Scope::new(Strategy::SameHostname, patterns(include), patterns(exclude));
    let (call_include, call_exclude) = (patterns(call_include), patterns(call_exclude));
    let narrowing = Narrowing {
        include: &call_include,
        exclude: &call_exclude,
    };

    assert_eq!(
        scope.admits(&connection, &url(link), narrowing).unwrap(),
        admits
    );
}

/// A crawl of several seeds is measured against every seed, and a scope of
/// none admits nothing (ADR 0093).
///
/// Covers: story:6-224
#[test]
fn the_scope_is_measured_against_every_seed() {
    let connection = run_file();
    let scope = Scope::new(
        Strategy::SameHostname,
        Patterns::default(),
        Patterns::default(),
    );
    let admits = |link: &str| {
        scope
            .admits(&connection, &url(link), Narrowing::none())
            .unwrap()
    };
    let before = admits("http://b.test/x");
    Scope::widen(&connection, &url("http://a.test/")).unwrap();
    Scope::widen(&connection, &url("http://b.test/")).unwrap();

    assert_eq!(
        (
            before,
            admits("http://a.test/x"),
            admits("http://b.test/x"),
            admits("http://c.test/x"),
        ),
        (false, true, true, false)
    );
}

/// The scope a Run recorded as it began is the one its run file gives
/// back, its strategy, patterns and seeds, so it stays fixed for the Run's
/// life (ADR 0102).
///
/// Covers: story:6-224, setting:crawl.scope.strategy
#[test]
fn the_scope_a_run_recorded_is_the_one_it_reads_back() {
    let connection = run_file();
    let recorded = Scope::new(
        Strategy::SameDomain,
        patterns(&["**/shop/**"]),
        patterns(&["**/shop/admin/**"]),
    );
    recorded.record(&connection).unwrap();
    Scope::widen(&connection, &url("http://www.shop.test/")).unwrap();

    let read = Scope::read(&connection).unwrap();

    let admits = |link: &str| {
        read.admits(&connection, &url(link), Narrowing::none())
            .unwrap()
    };
    assert_eq!(
        (
            read.strategy,
            admits("http://cdn.shop.test/shop/1"),
            admits("http://cdn.shop.test/about"),
            admits("http://cdn.shop.test/shop/admin/1"),
            admits("http://outlet.test/shop/1"),
        ),
        (Strategy::SameDomain, true, false, false, false)
    );
}

/// A host label that is never `www`.
fn label() -> impl proptest::strategy::Strategy<Value = String> {
    "[a-z][a-z0-9]{0,6}".prop_filter("not www", |label| label != "www")
}

fn scheme() -> impl proptest::strategy::Strategy<Value = &'static str> {
    prop_oneof![Just("http"), Just("https")]
}

fn port() -> impl proptest::strategy::Strategy<Value = Option<u16>> {
    prop_oneof![Just(None), Just(Some(8080)), Just(Some(8443))]
}

fn origin(scheme: &str, host: &str, port: Option<u16>) -> String {
    port.map_or_else(
        || format!("{scheme}://{host}"),
        |port| format!("{scheme}://{host}:{port}"),
    )
}

prop_compose! {
    /// A URL on a host of one to three labels under `test`, with an
    /// optional `www.` label, a scheme, a port and a path.
    fn site_url()(
        labels in proptest::collection::vec(label(), 1..=3),
        www in any::<bool>(),
        scheme in scheme(),
        port in port(),
        path in "(/[a-z]{0,4}){0,3}",
    ) -> Url {
        let host = format!("{}{}.test", if www { "www." } else { "" }, labels.join("."));
        url(&format!("{}{path}", origin(scheme, &host, port)))
    }
}

proptest! {
    /// The strategies nest: what `same_origin` reaches `same_hostname`
    /// does, what `same_hostname` reaches `same_domain` does, and `all`
    /// reaches every URL.
    ///
    /// Covers: story:6-224
    #[test]
    fn the_strategies_nest(seed in site_url(), link in site_url()) {
        let reaches = |strategy: Strategy| strategy.reaches(&seed, &link);
        prop_assert!(!reaches(Strategy::SameOrigin) || reaches(Strategy::SameHostname));
        prop_assert!(!reaches(Strategy::SameHostname) || reaches(Strategy::SameDomain));
        prop_assert!(reaches(Strategy::All));
    }

    /// A subdomain of the seed's host, or a sister domain beside it, never
    /// leaks into a `same_hostname` or `same_origin` crawl, while a
    /// subdomain stays in a `same_domain` one and a sister domain does not
    /// (pitfall U15).
    ///
    /// Covers: story:6-224
    #[test]
    fn neither_a_subdomain_nor_a_sister_domain_leaks(
        domain in label(),
        sub in label(),
        sister in label(),
        scheme in scheme(),
        port in port(),
    ) {
        prop_assume!(sister != domain);
        let seed = url(&format!("{}/", origin(scheme, &format!("{domain}.test"), port)));
        let subdomain = url(&format!("{}/a", origin(scheme, &format!("{sub}.{domain}.test"), port)));
        let sister = url(&format!("{}/a", origin(scheme, &format!("{sister}.test"), port)));
        for strategy in [Strategy::SameHostname, Strategy::SameOrigin] {
            prop_assert!(!strategy.reaches(&seed, &subdomain));
            prop_assert!(!strategy.reaches(&seed, &sister));
        }
        prop_assert!(Strategy::SameDomain.reaches(&seed, &subdomain));
        prop_assert!(!Strategy::SameDomain.reaches(&seed, &sister));
    }

    /// One leading `www.` label is ignored on either side, under every
    /// strategy, and an `http` seed's origin admits its host's `https`
    /// origin on its port, while an `https` seed's never admits `http`
    /// (ADR 0120).
    ///
    /// Covers: story:6-226
    #[test]
    fn one_www_label_and_the_https_upgrade_never_wait_for_evidence(
        labels in proptest::collection::vec(label(), 1..=3),
        port in port(),
        path in "(/[a-z]{0,4}){0,3}",
    ) {
        let host = format!("{}.test", labels.join("."));
        let bare = url(&format!("{}{path}", origin("http", &host, port)));
        let www = url(&format!("{}{path}", origin("http", &format!("www.{host}"), port)));
        let https = url(&format!("{}{path}", origin("https", &host, port)));
        for strategy in [Strategy::SameHostname, Strategy::SameDomain, Strategy::SameOrigin] {
            prop_assert!(strategy.reaches(&bare, &www));
            prop_assert!(strategy.reaches(&www, &bare));
        }
        prop_assert!(Strategy::SameOrigin.reaches(&bare, &https));
        prop_assert!(Strategy::SameOrigin.reaches(&www, &https));
        prop_assert!(!Strategy::SameOrigin.reaches(&https, &bare));
    }

    /// Patterns only narrow: whatever the crawl's and the call's patterns,
    /// a URL the scope admits is one its strategy reaches from a seed, and
    /// one an `exclude` pattern matches is never admitted.
    ///
    /// Covers: story:6-225
    #[test]
    fn patterns_never_widen_the_strategy(
        seed in site_url(),
        link in site_url(),
        include in proptest::collection::vec("\\*\\*/[a-z]{0,2}\\*\\*", 0..3),
        exclude in proptest::collection::vec("\\*\\*/[a-z]{0,2}\\*\\*", 0..3),
        strategy in prop_oneof![
            Just(Strategy::SameHostname),
            Just(Strategy::SameDomain),
            Just(Strategy::SameOrigin),
        ],
    ) {
        let connection = seeded(&[&seed]);
        let (include, exclude) = (Patterns::globs(&include).unwrap(), Patterns::globs(&exclude).unwrap());
        let scope = Scope::new(strategy, include.clone(), exclude.clone());
        let unnarrowed = Scope::new(strategy, Patterns::default(), Patterns::default());

        let admitted = scope.admits(&connection, &link, Narrowing::none()).unwrap();
        let narrowed = unnarrowed
            .admits(&connection, &link, Narrowing { include: &include, exclude: &exclude })
            .unwrap();

        prop_assert!(!admitted || strategy.reaches(&seed, &link));
        prop_assert!(!admitted || !exclude.any_matches(&link));
        prop_assert_eq!(admitted, narrowed);
    }

    /// The run file's lookup of a URL's measures among its seeds' answers
    /// what the strategy's own comparison with each seed does, under every
    /// strategy, so a scope never needs to read every seed.
    ///
    /// Covers: story:6-224
    #[test]
    fn the_seeds_lookup_answers_as_the_strategy_reaches(
        seeds in proptest::collection::vec(site_url(), 1..4),
        link in site_url(),
        strategy in prop_oneof![
            Just(Strategy::SameHostname),
            Just(Strategy::SameDomain),
            Just(Strategy::SameOrigin),
            Just(Strategy::All),
        ],
    ) {
        let connection = seeded(&seeds.iter().collect::<Vec<_>>());
        let scope = Scope::new(strategy, Patterns::default(), Patterns::default());

        let admitted = scope.admits(&connection, &link, Narrowing::none()).unwrap();

        prop_assert_eq!(admitted, seeds.iter().any(|seed| strategy.reaches(seed, &link)));
    }
}
