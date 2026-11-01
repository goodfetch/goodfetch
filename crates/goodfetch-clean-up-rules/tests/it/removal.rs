//! Extraction-side removal on the embedded Filter lists (ADR 0047): the
//! elements the cosmetic rules of the lists that are on match go from a
//! document before any extraction, inside its open and closed shadow roots
//! too, and the caller learns what went: each element's rule, its text and
//! the links inside it, which area 6's trap-link judgement leaves out.

use std::sync::LazyLock;

use goodfetch_clean_up_rules::matching::{Choices, List};
use goodfetch_clean_up_rules::removal::{self, Selectors, Share};
use goodfetch_clean_up_rules::rules::FilterLists;

static LISTS: LazyLock<FilterLists> =
    LazyLock::new(|| FilterLists::embedded().expect("the embedded Filter lists load"));

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
    let removal = removal::remove(
        PAGE,
        "https://news.example/article",
        (&LISTS, &defaults()),
        &Selectors::default(),
    );

    let kept = |text: &str| removal.html.contains(text);
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
        "{}",
        removal.html
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

/// The pages that share their selectors, as a Run's do, parse each of a
/// rule set's selectors once: a page whose rules the earlier pages held
/// parses none, and one that holds a class a generic rule keys parses that
/// rule's selector alone (#1765). Each removes what a page with its own
/// selectors does.
/// Covers: story:5-98
#[test]
fn pages_that_share_their_selectors_parse_each_selector_of_their_rules_once() {
    let shared = Selectors::default();
    let remove = |html, path| {
        let url = format!("https://news.example/{path}");
        let removal = removal::remove(html, &url, (&LISTS, &defaults()), &shared);
        let alone = removal::remove(html, &url, (&LISTS, &defaults()), &Selectors::default());
        assert_eq!(
            (&removal.html, &removal.removed),
            (&alone.html, &alone.removed)
        );
        (shared.parsed(), removal.removed.len())
    };

    let (first, _) = remove(PLAIN, "first");
    let [second, with_ads, again] = [
        remove(PLAIN, "second"),
        remove(PAGE, "third"),
        remove(PAGE, "fourth"),
    ];

    assert!(first > 0, "the first page parsed no selector");
    // `.ad-slot`, EasyList's generic rule the class `ad-slot` keys, is the
    // one selector the third page's rules hold that the first's did not.
    assert_eq!(
        [second, with_ads, again],
        [(first, 0), (first + 1, 3), (first + 1, 3)]
    );
}

/// Covers: story:5-98
#[test]
fn a_list_turned_off_removes_nothing() {
    let choices = Choices {
        ads: false,
        ..defaults()
    };
    let removal = removal::remove(
        PAGE,
        "https://news.example/article",
        (&LISTS, &choices),
        &Selectors::default(),
    );

    assert_eq!(
        (removal.removed.len(), removal.html.contains("A light ad")),
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
    let removal = removal::remove(
        NOTICED,
        "https://news.example/article",
        (&LISTS, &defaults()),
        &Selectors::default(),
    );

    let removal = removal::remove_selected(
        removal,
        &["#cookie-notice".to_owned(), ".banner".to_owned()],
    );

    assert_eq!(
        (
            [
                "The article's own text.",
                "This website uses cookies.",
                "Cookies in a root."
            ]
            .map(|text| removal.html.contains(text)),
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
