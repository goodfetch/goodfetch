//! Extraction-side removal on the embedded Filter lists (ADR 0047): the
//! elements the cosmetic rules of the lists that are on match go from a
//! document before any extraction, inside its open and closed shadow roots
//! too, and the caller learns what went: each element's rule, its text and
//! the links inside it, which area 6's trap-link judgement leaves out.
#![expect(
    clippy::disallowed_methods,
    reason = "ADR 0138: a test's own parse, on no page's path"
)]

use std::sync::LazyLock;

use goodfetch_clean_up_rules::list::List;
use goodfetch_clean_up_rules::matching::Choices;
use goodfetch_clean_up_rules::removal::{self, HostCache, Removal, Rules, Share};
use goodfetch_clean_up_rules::rules::{FilterLists, ListTexts};
use goodfetch_counted_work::{CountedWork, Row};

static LISTS: LazyLock<FilterLists> =
    LazyLock::new(|| FilterLists::embedded().expect("the embedded Filter lists load"));

/// The digest the tests' rules are of.
const VERSION: &str = "embedded";

/// `html`, the document of the page at `url`, with what the rules of the
/// lists `choices` turn on removed in place on its one tree, through the
/// engine's cache on `work`: what was removed, and the tree serialized.
fn removed(html: &str, url: &str, choices: &Choices, work: &CountedWork) -> (Removal, String) {
    let document = work.parse_html(Row::PageParses, html);
    let removal = removal::remove(
        &document,
        url,
        Rules {
            lists: &LISTS,
            version: VERSION,
            choices,
            hosts: &HostCache::new(0),
        },
        work,
    );
    (removal, document.html().to_string())
}

fn defaults() -> Choices {
    Choices {
        ads: true,
        trackers: true,
        annoyances: true,
        ..Choices::default()
    }
}

/// An article with an ad (EasyList's `##.ad-slot`) in its light DOM, one in
/// an open shadow root and one in a closed one, as a page serialized with
/// its shadow roots reads.
const PAGE: &str = r#"<!DOCTYPE html>
<html><head><title>An article</title></head><body>
<main>
<h1>An article</h1>
<p>The article's own text.</p>
<div class="ad-slot">A light ad <a href="https://ads.example/light">Buy</a></div>
<div id="open-host"><template shadowrootmode="open"><p>Text in an open root.</p><div class="ad-slot">An open-root ad</div></template></div>
<div id="closed-host"><template shadowrootmode="closed"><p>Text in a closed root.</p><div class="ad-slot">A closed-root ad <a href="https://ads.example/closed">Buy</a></div></template></div>
</main>
</body></html>"#;

/// Covers: story:5-98
#[test]
fn removal_walks_open_and_closed_shadow_roots_and_exposes_what_it_matched() {
    let (removal, html) = removed(
        PAGE,
        "https://news.example/article",
        &defaults(),
        &CountedWork::default(),
    );

    let kept = |text: &str| html.contains(text);
    assert_eq!(
        [
            "The article's own text.",
            "Text in an open root.",
            "Text in a closed root.",
            "A light ad",
            "An open-root ad",
            "A closed-root ad",
        ]
        .map(kept),
        [true, true, true, false, false, false],
        "{html}"
    );
    let removed: Vec<(&str, List, bool, Vec<&str>)> = removal
        .removed
        .iter()
        .map(|element| {
            (
                element.rule.as_str(),
                element.list.expect("a Filter list's rule names its list"),
                element.in_shadow_root,
                element.links.iter().map(String::as_str).collect(),
            )
        })
        .collect();
    assert_eq!(
        removed,
        vec![
            (
                ".ad-slot",
                List::Ads,
                false,
                vec!["https://ads.example/light"]
            ),
            (".ad-slot", List::Ads, true, vec![]),
            (
                ".ad-slot",
                List::Ads,
                true,
                vec!["https://ads.example/closed"]
            ),
        ]
    );
    // "AlightadBuy", "Anopen-rootad" and "Aclosed-rootadBuy": the text
    // removed, white space aside.
    assert_eq!(removal.removed_text, 11 + 13 + 17);
}

/// An article with no element any rule keys by its class or ID.
const PLAIN: &str = "<!DOCTYPE html>
<html><head><title>An article</title></head><body>
<main><h1>An article</h1><p>The article's own text.</p></main>
</body></html>";

/// The pages that share the engine's cache of compilations, as its Runs'
/// do, compile each selector they test once: a page that tests no rule
/// beyond what the earlier pages tested compiles none, and one whose
/// elements a rule is filed under compiles that rule's selector the first
/// time it is tested (#1765, #1912, #2186). Each removes what a page with a
/// cache of its own does. Rules of another version compile their selectors
/// again.
/// Covers: story:5-98
#[test]
fn pages_that_share_the_engines_cache_compile_each_selector_of_their_rules_once() {
    let shared = CountedWork::default();
    let compiled = || shared.read(Row::PageSelectorCompilations);
    let remove = |html, path| {
        let url = format!("https://news.example/{path}");
        let (removal, cleaned) = removed(html, &url, &defaults(), &shared);
        let (alone, alone_cleaned) = removed(html, &url, &defaults(), &CountedWork::default());
        assert_eq!(
            (&cleaned, &removal.removed),
            (&alone_cleaned, &alone.removed)
        );
        (compiled(), removal.removed.len())
    };

    let (first, _) = remove(PLAIN, "first");
    let [second, (with_ads, removed), again] = [
        remove(PLAIN, "second"),
        remove(PAGE, "third"),
        remove(PAGE, "fourth"),
    ];
    let another = FilterLists::embedded().expect("the embedded Filter lists load");
    let document = shared.parse_html(Row::PageParses, PAGE);
    let _ = removal::remove(
        &document,
        "https://news.example/fifth",
        Rules {
            lists: &another,
            version: "another",
            choices: &defaults(),
            hosts: &HostCache::new(0),
        },
        &shared,
    );

    assert!(first > 0, "the first page compiled no selector");
    assert!(
        with_ads > first,
        "the page with ads compiled no selector of the rules it tested"
    );
    // The shadow roots' selector is the engine's own, of no version: the
    // page of the other version compiles every other the third page did.
    assert_eq!(
        ([second, (with_ads, removed), again], compiled()),
        (
            [(first, 0), (with_ads, 3), (with_ads, 3)],
            2 * with_ads - first
        )
    );
}

/// Removal counts its page's rule list as one build (`F1.rule_list_builds`)
/// and each rule it tests (`F1.rule_tests`): a plain rule against an element
/// whose ID, classes, attributes or tag it is filed under, up to the first
/// that matches, and a procedural rule once a root. A page with no list on
/// builds no rule list and tests no rule.
#[test]
fn removal_counts_its_pages_rule_list_and_each_rule_it_tests() {
    // In rule order, which is their selectors' text order.
    let lists = FilterLists::compile(&ListTexts {
        ads: "news.example###ad\n\
              news.example##.promo:empty\n\
              news.example##[data-ad]\n\
              news.example##aside\n\
              news.example##div:has-text(Sponsored)",
        trackers: "",
        annoyances: "",
        unbreak: "",
    });
    // `html`, `head`, `title`, `body` and `p` are filed under by no rule.
    // The first `div` is tested against `.promo:empty`, which it does not
    // match. The `aside` is tested against `.promo:empty`, then `[data-ad]`,
    // which matches it, and not against `aside`. The second `div` is tested
    // against `#ad`. The procedural rule runs once, on the document.
    let page = r#"<!DOCTYPE html>
<html><head><title>An article</title></head><body>
<p>The article's own text.</p>
<div class="promo">Sponsored</div>
<aside class="promo" data-ad="1">An ad</aside>
<div id="ad"></div>
</body></html>"#;
    let clean = |choices: &Choices, work: &CountedWork| {
        let document = work.parse_html(Row::PageParses, page);
        let removal = removal::remove(
            &document,
            "https://news.example/article",
            Rules {
                lists: &lists,
                version: VERSION,
                choices,
                hosts: &HostCache::new(0),
            },
            work,
        );
        let rules: Vec<String> = removal
            .removed
            .into_iter()
            .map(|element| element.rule)
            .collect();
        (
            rules,
            work.read(Row::PageRuleListBuilds),
            work.read(Row::PageRuleTests),
        )
    };
    let work = CountedWork::default();

    let (removed, builds, tests) = clean(&defaults(), &work);
    let (_, builds_of_two, tests_of_two) = clean(&defaults(), &work);
    let off = clean(&Choices::default(), &CountedWork::default());

    assert_eq!(
        (&removed[..2], removed.len(), builds, tests),
        (
            &["[data-ad]".to_owned(), "#ad".to_owned()][..],
            3,
            1,
            1 + 2 + 1 + 1
        )
    );
    assert_eq!((builds_of_two, tests_of_two), (2, 2 * tests));
    assert_eq!(off, (vec![], 0, 0));
}

/// Covers: story:5-98
#[test]
fn a_list_turned_off_removes_nothing() {
    let choices = Choices {
        ads: false,
        ..defaults()
    };
    let (removal, html) = removed(
        PAGE,
        "https://news.example/article",
        &choices,
        &CountedWork::default(),
    );

    assert_eq!(
        (removal.removed.len(), html.contains("A light ad")),
        (0, true)
    );
}

/// Covers: story:5-98
#[test]
fn a_removal_over_a_fifth_of_the_main_contents_text_is_flagged() {
    let shares = [(1000, 1000), (1000, 800), (1000, 799), (1000, 0), (0, 0)]
        .map(|(before, after)| Share::of(before, after).flagged());

    assert_eq!(shares, [false, false, true, true, false]);
}

/// A page with a cookie notice the Consent prompt runner hid, in its light
/// DOM and in a shadow root.
const NOTICED: &str = r#"<!DOCTYPE html>
<html><head><title>An article</title></head><body>
<main><h1>An article</h1><p>The article's own text.</p></main>
<div id="cookie-notice"><p>This website uses cookies.</p><button>Got it</button></div>
<div id="host"><template shadowrootmode="open"><div class="banner">Cookies in a root.</div></template></div>
</body></html>"#;

/// The prompts the Consent prompt runner hid go from extracted content as
/// the cosmetic rules' matches do, each recorded with its selector and no
/// Filter list (ADR 0048).
/// Covers: story:5-98
#[test]
fn the_prompts_the_runner_hid_go_from_extracted_content_with_no_list() {
    let work = CountedWork::default();
    let document = work.parse_html(Row::PageParses, NOTICED);
    let mut removal = removal::remove(
        &document,
        "https://news.example/article",
        Rules {
            lists: &LISTS,
            version: VERSION,
            choices: &defaults(),
            hosts: &HostCache::new(0),
        },
        &work,
    );

    removal::remove_selected(
        &document,
        &mut removal,
        &["#cookie-notice".to_owned(), ".banner".to_owned()],
        &work,
    );
    let html = document.html().to_string();

    assert_eq!(
        (
            [
                "The article's own text.",
                "This website uses cookies.",
                "Cookies in a root."
            ]
            .map(|text| html.contains(text)),
            removal
                .removed
                .iter()
                .map(|element| (element.rule.as_str(), element.list, element.in_shadow_root))
                .collect::<Vec<_>>(),
        ),
        (
            [true, false, false],
            vec![("#cookie-notice", None, false), (".banner", None, true)],
        )
    );
}
