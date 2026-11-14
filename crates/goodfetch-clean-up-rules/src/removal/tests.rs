//! The removal's index against the removal as it was before it (spec
//! #2164): on a table of pages, on a corpus and on generated pages and rule
//! sets, [`super::remove`] takes the elements [`reference::remove`] takes,
//! each credited to the same rule and list, in the same order, and leaves
//! the same tree.
#![expect(
    clippy::disallowed_methods,
    reason = "ADR 0138: a test's own parse, on no page's path"
)]

mod reference;

use std::path::PathBuf;
use std::sync::LazyLock;

use dom_query::NodeRef;
use goodfetch_counted_work::{CountedWork, ParsedHtml, Row};
use proptest::prelude::*;
use rstest::rstest;

use super::{Removal, Removed, Rules};
use crate::matching::{Choices, List};
use crate::rules::{FilterLists, ListTexts};

/// The page every test's rules are read for.
const URL: &str = "https://news.example/article";

static EMBEDDED: LazyLock<FilterLists> =
    LazyLock::new(|| FilterLists::embedded().expect("the embedded Filter lists load"));

/// The Filter lists `ads` compiles to as EasyList and `trackers` as
/// EasyPrivacy, each line one rule.
fn lists(ads: &[&str], trackers: &[&str]) -> FilterLists {
    FilterLists::compile(&ListTexts {
        ads: &ads.join("\n"),
        trackers: &trackers.join("\n"),
        annoyances: "",
        unbreak: "",
    })
}

fn every_list() -> Choices {
    Choices {
        ads: true,
        trackers: true,
        annoyances: true,
        ..Choices::default()
    }
}

/// What a removal made of a page.
#[derive(Debug, PartialEq, Eq)]
struct Cleaned {
    /// What it took, in its order.
    removed: Vec<Removed>,
    removed_text: usize,
    /// The tree it left, its shadow roots included.
    html: String,
    /// The words the document keeps outside its shadow roots.
    kept: String,
}

type Remove = fn(&ParsedHtml, &str, Rules<'_>, &CountedWork) -> Removal;

/// `html` cleaned by `remove` with the rules of `lists` that are on for
/// [`URL`], on a parse and a cache of its own, and the rule tests it
/// counted.
fn cleaned(remove: Remove, html: &str, lists: &FilterLists) -> (Cleaned, u64) {
    let work = CountedWork::default();
    let document = work.parse_html(Row::PageParses, html);
    let removal = remove(
        &document,
        URL,
        Rules {
            lists,
            version: "a test's",
            choices: &every_list(),
        },
        &work,
    );
    let kept = document
        .root()
        .descendants_it()
        .filter(NodeRef::is_text)
        .flat_map(|node| {
            node.text()
                .split_whitespace()
                .map(str::to_owned)
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>()
        .join(" ");
    (
        Cleaned {
            removed: removal.removed,
            removed_text: removal.removed_text,
            html: document.html().to_string(),
            kept,
        },
        work.read(Row::PageRuleTests),
    )
}

/// `html` as the index cleans it and as the reference does, each with the
/// rule tests it counted.
fn both(html: &str, lists: &FilterLists) -> [(Cleaned, u64); 2] {
    [
        cleaned(super::remove, html, lists),
        cleaned(reference::remove, html, lists),
    ]
}

/// A page of `body`, with a doctype.
fn page(body: &str) -> String {
    format!("<!DOCTYPE html><html><head><title>A page</title></head><body>{body}</body></html>")
}

/// Each case's page loses the elements named, each to the rule and list
/// named, in that order, and keeps the words named: by the index as by the
/// reference.
#[rstest]
// A procedural rule read on the untouched tree would take the card too: it
// reads the tree the plain rules left, where the card no longer says
// "Sponsored".
#[case::a_procedural_rule_reads_the_tree_the_plain_rules_left(
    &["news.example##.sponsor-label", "news.example##div.card:has-text(Sponsored)"],
    &[],
    page(r#"<div class="card"><span class="sponsor-label">Sponsored</span><p>story</p></div>"#),
    &[(".sponsor-label", List::Ads)],
    "A page story"
)]
// No doctype: the page is in quirks mode, where a browser matches classes
// without their case. The match is in no-quirks mode on every document.
#[case::a_quirks_mode_pages_class_keeps_its_case(
    &["news.example##.ad-slot"],
    &[],
    "<html><head><title>A page</title></head><body><div class=AD-SLOT>upper</div><div class=ad-slot>lower</div></body></html>".to_owned(),
    &[(".ad-slot", List::Ads)],
    "A page upper"
)]
#[case::a_rules_class_keeps_its_case(
    &["news.example##.AD-SLOT"],
    &[],
    page("<div class=AD-SLOT>upper</div><div class=ad-slot>lower</div>"),
    &[(".AD-SLOT", List::Ads)],
    "A page lower"
)]
#[case::a_generic_rule_the_pages_class_keys(
    &["##.ad-slot"],
    &[],
    page("<div class=AD-SLOT>upper</div><div class=ad-slot>lower</div>"),
    &[(".ad-slot", List::Ads)],
    "A page upper"
)]
#[case::a_selector_list_takes_what_each_member_matches(
    &["news.example##.promo, #sidebar-ad, aside[data-ad]"],
    &[],
    page(r#"<p>kept</p><div class="promo">one</div><div id="sidebar-ad">two</div><aside data-ad>three</aside><aside>too</aside>"#),
    &[
        (".promo, #sidebar-ad, aside[data-ad]", List::Ads),
        (".promo, #sidebar-ad, aside[data-ad]", List::Ads),
        (".promo, #sidebar-ad, aside[data-ad]", List::Ads),
    ],
    "A page kept too"
)]
#[case::attribute_only_rules(
    &["news.example##[data-ad-slot]", r#"news.example##[aria-label="Advertisement"]"#],
    &[],
    page(r#"<div aria-label="Advertisement">one</div><div data-ad-slot="1">two</div><div aria-label="Navigation">kept</div>"#),
    &[(r#"[aria-label="Advertisement"]"#, List::Ads), ("[data-ad-slot]", List::Ads)],
    "A page kept"
)]
#[case::an_attribute_rule_on_a_class_or_an_id(
    &["news.example##[class~=\"promo\"]", "news.example##[id^=\"ad-\"]"],
    &[],
    page(r#"<div class="wide promo">one</div><div id="ad-1">two</div><div id="bad-1" class="promoted">kept</div>"#),
    &[(r#"[class~="promo"]"#, List::Ads), (r#"[id^="ad-"]"#, List::Ads)],
    "A page kept"
)]
#[case::escaped_names(
    &[r"news.example##.ad\:slot", r"news.example###\31 st"],
    &[],
    page(r#"<div class="ad:slot">one</div><div id="1st">two</div><div class="ad" id="st">kept</div>"#),
    &[(r".ad\:slot", List::Ads), (r"#\31 st", List::Ads)],
    "A page kept"
)]
// `adblock` reads `*|a.sponsored` as `a.sponsored`, which names no
// namespace and so matches SVG's `a` as HTML's. `[*|href]` matches an
// attribute of any namespace, `xlink:href` among them, and `[href]` one of
// none.
#[case::namespaces(
    &[
        "news.example##*|a.sponsored",
        r#"news.example##[*|href="/any"]"#,
        r#"news.example##[href="/none"]"#,
    ],
    &[],
    page(concat!(
        r#"<svg><a class="sponsored">one</a><a xlink:href="/any">two</a><a xlink:href="/none">kept</a></svg>"#,
        r#"<a class="sponsored">three</a><a href="/any">four</a><a href="/none">five</a>"#,
    )),
    &[
        ("a.sponsored", List::Ads),
        (r#"[*|href="/any"]"#, List::Ads),
        ("a.sponsored", List::Ads),
        (r#"[*|href="/any"]"#, List::Ads),
        (r#"[href="/none"]"#, List::Ads),
    ],
    "A page kept"
)]
// HTML's tags match without their case and a foreign element's with it.
#[case::a_tags_case(
    &["news.example##ASIDE", "news.example##foreignobject"],
    &[],
    page("<aside>one</aside><svg><foreignObject>kept</foreignObject></svg>"),
    &[("ASIDE", List::Ads)],
    "A page kept"
)]
// Rule order is the selectors' text order: `#` before `.` before `[`.
#[case::the_earliest_rule_in_rule_order_wins(
    &[
        "news.example##div.banner",
        "news.example##[id]",
        "news.example##.banner",
        "news.example###ad",
    ],
    &[],
    page(r#"<div id="ad" class="banner">one</div><div class="banner">two</div><div id="other">three</div><div>kept</div>"#),
    &[("#ad", List::Ads), (".banner", List::Ads), ("[id]", List::Ads)],
    "A page kept"
)]
#[case::a_selector_two_lists_hold_is_the_first_lists(
    &["news.example##.promo"],
    &["news.example##.promo", "news.example##.beacon"],
    page(r#"<div class="beacon">one</div><div class="promo">two</div><p>kept</p>"#),
    &[(".beacon", List::Trackers), (".promo", List::Ads)],
    "A page kept"
)]
#[case::rules_no_key_files(
    &["news.example##:is(.promo, .advert)", "news.example##:not(html, head, title, body, p)"],
    &[],
    page(r#"<p class="promo">one</p><p>kept</p><p class="advert">two</p><div>three</div>"#),
    &[
        (":is(.promo, .advert)", List::Ads),
        (":is(.promo, .advert)", List::Ads),
        (":not(html, head, title, body, p)", List::Ads),
    ],
    "A page kept"
)]
// The matches of one root share their caches: an element's position among
// its siblings and what it holds are read once, on the untouched tree.
#[case::rules_that_read_an_elements_siblings_and_descendants(
    &["news.example##li:nth-child(2n+1)", "news.example##div:has(> img)"],
    &[],
    page(concat!(
        "<ul><li>one</li><li>kept</li><li>three</li></ul>",
        r#"<div><img alt="">four</div><div><p><img alt=""></p>too</div>"#,
    )),
    &[
        ("li:nth-child(2n+1)", List::Ads),
        ("li:nth-child(2n+1)", List::Ads),
        ("div:has(> img)", List::Ads),
    ],
    "A page kept too"
)]
#[case::an_element_inside_one_removed_is_not_listed(
    &["news.example##.promo"],
    &[],
    page(r#"<div class="promo">one <span class="promo">two</span></div><p>kept</p>"#),
    &[(".promo", List::Ads)],
    "A page kept"
)]
#[case::a_rule_whose_style_names_other_declarations_is_left_out(
    &["news.example##.price:style(color: red)"],
    &[],
    page(r#"<p class="price">kept</p>"#),
    &[],
    "A page kept"
)]
fn the_index_removes_what_the_reference_removes(
    #[case] ads: &[&str],
    #[case] trackers: &[&str],
    #[case] page: String,
    #[case] removed: &[(&str, List)],
    #[case] kept: &str,
) {
    let lists = lists(ads, trackers);

    let [(indexed, _), (reference, _)] = both(&page, &lists);

    let credited: Vec<(&str, List)> = indexed
        .removed
        .iter()
        .map(|element| {
            (
                element.rule.as_str(),
                element.list.expect("a Filter list's rule names its list"),
            )
        })
        .collect();
    assert_eq!(
        (credited.as_slice(), indexed.kept.as_str()),
        (removed, kept)
    );
    assert_eq!(indexed, reference);
}

/// Rules of every kind of key, for [`URL`], that match much of any page.
const BROAD: [&str; 16] = [
    "news.example##a[href^=\"http\"]",
    "news.example##li:nth-child(2n)",
    "news.example##p:nth-of-type(3)",
    "news.example##div:has(> img)",
    "news.example##h2 + p",
    "news.example##table tr:last-child",
    "news.example##img, figure, .ad-slot",
    "news.example##[id]:not(:has(*))",
    "news.example##span:first-child:last-child",
    "news.example##[class*=\"nav\"]",
    "news.example###content > :empty",
    "news.example##:is(aside, footer) a",
    "news.example##blockquote:has-text(the)",
    "news.example##code:upward(1)",
    "news.example##li:min-text-length(40)",
    "news.example##em:upward(p)",
];

/// The Filter lists the corpus is cleaned with.
#[derive(Clone, Copy, Debug)]
enum Corpus {
    /// The lists the crate embeds.
    Embedded,
    /// [`BROAD`] as EasyList.
    Broad,
}

static BROAD_LISTS: LazyLock<FilterLists> = LazyLock::new(|| lists(&BROAD, &[]));

impl Corpus {
    fn lists(self) -> &'static FilterLists {
        match self {
            Self::Embedded => &EMBEDDED,
            Self::Broad => &BROAD_LISTS,
        }
    }
}

/// The corpus: the removal tests' own pages and the Proving ground's
/// `clean-up` pages, whose markup the fixtures carry, the extractor's format
/// fixtures and the lab's HTML fixtures. On each page, with the embedded
/// lists and with rules of every kind of key, the index removes what the
/// reference removes, in no more rule tests.
#[rstest]
fn the_index_removes_what_the_reference_removes_from_the_corpus(
    #[files("src/removal/tests/fixtures/*.html")]
    #[files("../goodfetch-extractor/src/formats/fixtures/*.html")]
    #[files("../../rnd/lab/fixtures/markdown-structure/test/html/*.html")]
    page: PathBuf,
    #[values(Corpus::Embedded, Corpus::Broad)] corpus: Corpus,
) {
    let html = std::fs::read_to_string(&page)
        .unwrap_or_else(|error| panic!("{} cannot be read: {error}", page.display()));

    let [(indexed, with_index), (reference, without)] = both(&html, corpus.lists());

    assert_eq!(indexed, reference);
    assert!(
        with_index <= without,
        "{with_index} rule tests against {without}"
    );
}

/// The vocabulary the generated pages and rules share.
const TAGS: [&str; 7] = ["div", "span", "p", "a", "ul", "li", "DIV"];
const CLASSES: [&str; 5] = ["ad", "AD", "promo", "card", "sponsor-label"];
const IDS: [&str; 3] = ["top", "ad", "Ad"];
const ATTRIBUTES: [&str; 3] = ["data-ad", "href", "title"];
const VALUES: [&str; 2] = ["x", "Sponsored"];
const WORDS: [&str; 3] = ["Sponsored", "story", "a longer run of text"];

fn one_of(words: &'static [&'static str]) -> impl Strategy<Value = &'static str> {
    prop::sample::select(words)
}

/// A start tag's attributes: an ID, classes and other attributes.
fn attributes() -> impl Strategy<Value = String> {
    (
        prop::option::of(one_of(&IDS)),
        prop::collection::vec(one_of(&CLASSES), 0..3),
        prop::collection::vec((one_of(&ATTRIBUTES), one_of(&VALUES)), 0..3),
    )
        .prop_map(|(id, classes, others)| {
            let id = id.map(|id| format!(" id=\"{id}\""));
            let class = (!classes.is_empty()).then(|| format!(" class=\"{}\"", classes.join(" ")));
            let others = others
                .into_iter()
                .map(|(name, value)| format!(" {name}=\"{value}\""));
            id.into_iter().chain(class).chain(others).collect()
        })
}

/// An element: one that holds a word, or one that holds elements, in its
/// light DOM or in a shadow root.
fn element() -> impl Strategy<Value = String> {
    let leaf = (one_of(&TAGS), attributes(), one_of(&WORDS))
        .prop_map(|(tag, attributes, word)| format!("<{tag}{attributes}>{word}</{tag}>"));
    leaf.prop_recursive(4, 24, 4, |inner| {
        (
            one_of(&TAGS),
            attributes(),
            prop::collection::vec(inner, 0..4),
            prop::bool::weighted(0.2),
        )
            .prop_map(|(tag, attributes, children, in_shadow_root)| {
                let children = children.concat();
                if in_shadow_root {
                    format!(
                        "<{tag}{attributes}><template shadowrootmode=\"open\">{children}</template></{tag}>"
                    )
                } else {
                    format!("<{tag}{attributes}>{children}</{tag}>")
                }
            })
    })
}

/// A page, in no-quirks or in quirks mode.
fn generated_page() -> impl Strategy<Value = String> {
    (any::<bool>(), prop::collection::vec(element(), 1..5)).prop_map(|(doctype, elements)| {
        format!(
            "{}<html><head><title>A page</title></head><body>{}</body></html>",
            if doctype { "<!DOCTYPE html>" } else { "" },
            elements.concat()
        )
    })
}

/// A compound selector over the vocabulary.
fn compound() -> impl Strategy<Value = String> {
    (
        prop::option::of(one_of(&TAGS)),
        prop::option::of(one_of(&IDS)),
        prop::collection::vec(one_of(&CLASSES), 0..3),
        prop::option::of((one_of(&ATTRIBUTES), prop::option::of(one_of(&VALUES)))),
        prop::option::weighted(
            0.3,
            one_of(&[
                ":first-child",
                ":last-child",
                ":nth-child(2)",
                ":nth-of-type(odd)",
                ":empty",
                ":not(.ad)",
                ":has(.promo)",
                ":has(> span)",
            ]),
        ),
    )
        .prop_map(|(tag, id, classes, attribute, pseudo_class)| {
            let compound: String = tag
                .map(str::to_owned)
                .into_iter()
                .chain(id.map(|id| format!("#{id}")))
                .chain(classes.into_iter().map(|class| format!(".{class}")))
                .chain(attribute.map(|(name, value)| match value {
                    Some(value) => format!("[{name}=\"{value}\"]"),
                    None => format!("[{name}]"),
                }))
                .chain(pseudo_class.map(str::to_owned))
                .collect();
            if compound.is_empty() {
                "*".to_owned()
            } else {
                compound
            }
        })
}

/// A selector: compounds joined by combinators, alone or in a list.
fn selector() -> impl Strategy<Value = String> {
    let complex = (
        compound(),
        prop::collection::vec((one_of(&[" ", " > ", " + ", " ~ "]), compound()), 0..3),
    )
        .prop_map(|(first, rest)| {
            rest.into_iter()
                .fold(first, |selector, (combinator, compound)| {
                    selector + combinator + &compound
                })
        });
    prop::collection::vec(complex, 1..3).prop_map(|members| members.join(", "))
}

/// A cosmetic rule: generic or for [`URL`]'s host, plain or procedural.
fn rule() -> impl Strategy<Value = String> {
    (
        prop::bool::weighted(0.2),
        selector(),
        prop::option::weighted(
            0.2,
            one_of(&[
                ":has-text(Sponsored)",
                ":min-text-length(10)",
                ":upward(1)",
                ":upward(div)",
            ]),
        ),
    )
        .prop_map(|(generic, selector, procedural)| {
            format!(
                "{}##{selector}{}",
                if generic { "" } else { "news.example" },
                procedural.unwrap_or_default()
            )
        })
}

proptest! {
    #![proptest_config(goodfetch_test_support::proptest_config())]

    /// On a generated page and rule sets, over one vocabulary so that
    /// rules match, the index removes what the reference removes.
    #[test]
    fn the_index_removes_what_the_reference_removes_from_generated_pages(
        page in generated_page(),
        ads in prop::collection::vec(rule(), 0..8),
        trackers in prop::collection::vec(rule(), 0..4),
    ) {
        let ads: Vec<&str> = ads.iter().map(String::as_str).collect();
        let trackers: Vec<&str> = trackers.iter().map(String::as_str).collect();
        let lists = lists(&ads, &trackers);

        let [(indexed, with_index), (reference, without)] = both(&page, &lists);

        prop_assert_eq!(indexed, reference);
        prop_assert!(with_index <= without, "{} tests against {}", with_index, without);
    }
}
