//! Extraction-side removal on the embedded Filter lists (ADR 0047): the
//! elements the cosmetic rules of the lists that are on match go from a
//! document before any extraction, inside its open and closed shadow roots
//! too, and the caller learns what went: each element's rule, its text and
//! the links inside it, which area 6's trap-link judgement leaves out.

use std::sync::LazyLock;

use goodfetch_clean_up_rules::matching::{Choices, List};
use goodfetch_clean_up_rules::removal::{self, Share};
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
    let removal = removal::remove(PAGE, "https://news.example/article", &LISTS, &defaults());

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
                element.list,
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

/// Covers: story:5-98
#[test]
fn a_list_turned_off_removes_nothing() {
    let choices = Choices {
        ads: false,
        ..defaults()
    };
    let removal = removal::remove(PAGE, "https://news.example/article", &LISTS, &choices);

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
