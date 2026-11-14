//! The host cache (spec #2164, issue 2186): pages on a host read before
//! build no rule list, and whatever the cache holds, drops or evicts, each
//! page loses what the reference takes from it with a rule list of its own.

use goodfetch_counted_work::{CountedWork, Row};
use proptest::prelude::*;
use rstest::rstest;

use super::super::HostCache;
use super::{
    Cleaned, EMBEDDED, Remove, clean, every_list, generated_page, lists, page, reference, rule,
    selector,
};
use crate::generic::GENERIC_PROBE;
use crate::list::List;
use crate::matching::Choices;
use crate::rules::{
    ADS_GENERIC_HIDE, ANNOYANCES_GENERIC_HIDE, FilterLists, TRACKERS_GENERIC_HIDE,
    UNBREAK_GENERIC_HIDE,
};

/// A page the embedded lists clean: of a generic rule a class keys and one
/// no class or ID keys, both EasyList's, and of Fanboy Annoyance's.
const ADS: &str = r#"<!DOCTYPE html><html><head><title>A page</title></head><body>
<div class="ad-slot">one</div><div data-ad-cls="x">two</div><div class="social-share">three</div><p>kept</p>
</body></html>"#;

/// Each page of `pages`, at its URL, as `remove` cleans it with `lists`
/// and `choices` through `hosts`, on `work`.
fn clean_all(
    remove: Remove,
    pages: &[(&str, &str)],
    lists: &FilterLists,
    choices: &Choices,
    hosts: &HostCache,
    work: &CountedWork,
) -> Vec<Cleaned> {
    pages
        .iter()
        .map(|(url, html)| {
            let rules = super::super::Rules {
                lists,
                version: "a test's",
                choices,
                hosts,
            };
            clean(remove, html, url, rules, work)
        })
        .collect()
}

/// Each page of `pages` as the reference cleans it, with a rule list and a
/// cache of its own.
fn by_reference(pages: &[(&str, &str)], lists: &FilterLists, choices: &Choices) -> Vec<Cleaned> {
    pages
        .iter()
        .flat_map(|page| {
            clean_all(
                reference::remove,
                &[*page],
                lists,
                choices,
                &HostCache::new(0),
                &CountedWork::default(),
            )
        })
        .collect()
}

/// Two hosts' pages, in turn.
const IN_TURN: [(&str, &str); 5] = [
    ("https://news.example/a", ADS),
    ("https://shop.example/a", ADS),
    ("https://news.example/b", ADS),
    ("https://shop.example/b", ADS),
    ("https://news.example/c", ADS),
];

/// A page on a host the cache holds builds no rule list; at a bound of 0
/// each page builds its own; and with the hosts in turn at a bound of one,
/// where each page may evict the other host, no removal changes.
#[rstest]
#[case::no_cache(0, Some(5))]
#[case::a_bound_of_one_and_hosts_in_turn(1, None)]
#[case::the_default_bound(1_024, Some(2))]
fn the_host_cache_changes_no_removal(#[case] bound: u64, #[case] builds: Option<u64>) {
    let hosts = HostCache::new(bound);
    let work = CountedWork::default();

    let cleaned = clean_all(
        super::super::remove,
        &IN_TURN,
        &EMBEDDED,
        &every_list(),
        &hosts,
        &work,
    );

    assert!(
        cleaned.iter().all(|page| page.kept == "A page kept"),
        "a page kept an ad"
    );
    assert_eq!(cleaned, by_reference(&IN_TURN, &EMBEDDED, &every_list()));
    assert!(hosts.entries() <= bound, "{} hosts held", hosts.entries());
    if let Some(builds) = builds {
        assert_eq!(work.read(Row::PageRuleListBuilds), builds);
    }
}

/// The Resource governor's `critical` level empties the cache: it holds no
/// host and no byte, and the next page on a host it held builds its rule
/// list again.
#[test]
fn clearing_the_cache_drops_every_host() {
    let hosts = HostCache::new(1_024);
    let work = CountedWork::default();
    let remove = |pages: &[(&str, &str)]| {
        clean_all(
            super::super::remove,
            pages,
            &EMBEDDED,
            &every_list(),
            &hosts,
            &work,
        )
    };
    remove(&IN_TURN[..2]);
    let held = (hosts.entries(), hosts.bytes() > 0);

    hosts.clear();
    let cleared = (hosts.entries(), hosts.bytes());
    let again = remove(&IN_TURN[2..3]);

    assert_eq!(
        (held, cleared, work.read(Row::PageRuleListBuilds)),
        ((2, true), (0, 0), 3)
    );
    assert_eq!(
        again,
        by_reference(&IN_TURN[2..3], &EMBEDDED, &every_list())
    );
}

/// Two Collie Sessions that turn on different lists, on one host, get a
/// rule list each, and each page loses what its own lists take.
#[test]
fn two_sets_of_lists_on_one_host_get_their_own_rule_lists() {
    let hosts = HostCache::new(1_024);
    let work = CountedWork::default();
    let ads_alone = Choices {
        ads: true,
        ..Choices::default()
    };
    let pages = [("https://news.example/a", ADS)];

    let choices = [ads_alone, every_list()];
    let cleaned = choices.each_ref().map(|choices| {
        clean_all(
            super::super::remove,
            &pages,
            &EMBEDDED,
            choices,
            &hosts,
            &work,
        )
    });

    assert_eq!(
        cleaned,
        choices
            .each_ref()
            .map(|choices| by_reference(&pages, &EMBEDDED, choices))
    );
    assert_ne!(cleaned[0], cleaned[1]);
    assert_eq!(
        (work.read(Row::PageRuleListBuilds), hosts.entries()),
        (2, 2)
    );
}

/// The `$generichide` and `$ghide` exceptions the build keeps of each
/// embedded list, as `adblock` parses them: EasyList's hostless host
/// prefixes, path-scoped hosts and `domain=` exceptions among them, and
/// none of EasyPrivacy's.
#[test]
fn the_build_keeps_each_lists_generic_hide_exceptions() {
    let counts = [
        ADS_GENERIC_HIDE,
        TRACKERS_GENERIC_HIDE,
        ANNOYANCES_GENERIC_HIDE,
        UNBREAK_GENERIC_HIDE,
    ]
    .map(|lines| lines.lines().count());

    assert_eq!(counts, [167, 0, 11, 82]);
    assert!(ADS_GENERIC_HIDE.contains("@@://127.0.0.1$generichide"));
    assert!(ADS_GENERIC_HIDE.contains("@@||www.google.*/search?$generichide"));
}

/// Whether a list's generic rules are off for a URL, as the small engine of
/// its `$generichide` exceptions answers, is what its whole engine answers:
/// on hostless host prefixes, path-scoped hosts and `domain=` exceptions.
#[rstest]
#[case::a_search_page("https://www.google.fr/search?q=news", true)]
#[case::the_same_hosts_maps("https://www.google.fr/maps", false)]
#[case::the_loopback_address("http://127.0.0.1:8080/", true)]
#[case::a_private_address("http://192.168.1.1/admin", true)]
#[case::localhost("http://localhost/", true)]
#[case::localhost_on_a_port("http://localhost:3000/page", true)]
#[case::a_domain_exceptions_host("https://screenrant.com/article", true)]
#[case::another_search_engine("https://duckduckgo.com/?q=news", true)]
#[case::a_host_no_exception_names("https://news.example/article", false)]
#[case::no_url("not a url", false)]
fn the_generic_hide_bit_is_the_whole_engines(#[case] url: &str, #[case] ads: bool) {
    let bits = [List::Ads, List::Trackers, List::Annoyances].map(|list| {
        (
            EMBEDDED.generic_hide(list, url),
            EMBEDDED
                .engine(list)
                .url_cosmetic_resources(url)
                .generichide,
        )
    });

    assert_eq!(bits.map(|(small, _)| small), bits.map(|(_, whole)| whole));
    assert_eq!(bits[0].0, ads);
}

/// A `$generichide` exception a regular expression scopes turns the
/// generic rules off on the pages it matches alone, on one host: each page
/// gets its own bit, and loses what the reference takes.
#[test]
fn a_regular_expression_exception_is_read_per_url() {
    let lists = lists(
        &[
            "##.ad-slot",
            "##div[data-ad]",
            r"@@/^https:\/\/news\.example\/print\//$generichide",
        ],
        &[],
    );
    let html = page(r#"<div class="ad-slot">one</div><div data-ad>two</div><p>kept</p>"#);
    let pages = [
        ("https://news.example/print/1", html.as_str()),
        ("https://news.example/story", html.as_str()),
        ("https://news.example/print/2", html.as_str()),
    ];
    let hosts = HostCache::new(1_024);
    let work = CountedWork::default();

    let cleaned = clean_all(
        super::super::remove,
        &pages,
        &lists,
        &every_list(),
        &hosts,
        &work,
    );

    assert_eq!(
        cleaned
            .iter()
            .map(|page| page.kept.as_str())
            .collect::<Vec<_>>(),
        ["A page one two kept", "A page kept", "A page one two kept"]
    );
    assert_eq!(cleaned, by_reference(&pages, &lists, &every_list()));
    assert_eq!(
        (work.read(Row::PageRuleListBuilds), hosts.entries()),
        (2, 2)
    );
}

/// The host the generic set is read for has no rule of its own in any
/// embedded list: no exception, no procedural rule, and the hiding
/// selectors of another reserved host.
#[test]
fn the_generic_probe_names_no_host_of_the_lists() {
    for list in [List::Ads, List::Trackers, List::Annoyances] {
        let engine = EMBEDDED.engine(list);
        let probe = engine.url_cosmetic_resources(GENERIC_PROBE);
        let other = engine.url_cosmetic_resources("https://another.example.invalid/");

        assert!(probe.exceptions.is_empty(), "{list:?}");
        assert!(probe.procedural_actions.is_empty(), "{list:?}");
        assert!(!probe.generichide, "{list:?}");
        assert_eq!(probe.hide_selectors, other.hide_selectors, "{list:?}");
    }
}

/// A host exception, or a `$generichide` exception of a host, a path, a
/// regular expression or a `domain=` list.
fn exception() -> impl Strategy<Value = String> {
    prop_oneof![
        selector().prop_map(|selector| format!("news.example#@#{selector}")),
        selector().prop_map(|selector| format!("sub.news.example#@#{selector}")),
        Just("@@||news.example^$generichide".to_owned()),
        Just("@@||news.example/print/$generichide".to_owned()),
        Just(r"@@/\/print\//$ghide".to_owned()),
        Just("@@$ghide,domain=sub.news.example".to_owned()),
    ]
}

/// The URLs the generated pages are read at: hosts the rules name, one
/// they do not, paths an exception scopes, and no URL.
const URLS: [&str; 6] = [
    "https://news.example/article",
    "https://news.example/print/1",
    "https://sub.news.example/x",
    "https://other.example/print/2",
    "https://other.example/",
    "not a url",
];

proptest! {
    #![proptest_config(goodfetch_test_support::proptest_config())]

    /// On generated pages, rules and exceptions, read at several URLs
    /// through one host cache, each page loses what the reference takes.
    #[test]
    fn the_host_cache_removes_what_the_reference_removes_from_generated_pages(
        page in generated_page(),
        ads in prop::collection::vec(prop_oneof![3 => rule(), 1 => exception()], 0..8),
        trackers in prop::collection::vec(prop_oneof![3 => rule(), 1 => exception()], 0..4),
        urls in prop::collection::vec(prop::sample::select(&URLS[..]), 1..6),
        bound in prop::sample::select(&[0_u64, 1, 1_024][..]),
    ) {
        let ads: Vec<&str> = ads.iter().map(String::as_str).collect();
        let trackers: Vec<&str> = trackers.iter().map(String::as_str).collect();
        let lists = lists(&ads, &trackers);
        let pages: Vec<(&str, &str)> = urls.iter().map(|url| (*url, page.as_str())).collect();
        let hosts = HostCache::new(bound);

        let cleaned = clean_all(
            super::super::remove,
            &pages,
            &lists,
            &every_list(),
            &hosts,
            &CountedWork::default(),
        );

        prop_assert_eq!(cleaned, by_reference(&pages, &lists, &every_list()));
    }
}
