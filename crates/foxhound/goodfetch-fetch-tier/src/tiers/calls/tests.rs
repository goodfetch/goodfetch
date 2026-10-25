//! Covers: story:5-115

use super::matches;

/// A locator's CSS selector counts the elements of a static document's
/// own markup; the markup cannot tell for a selector the matcher does not
/// parse, nor for a document with a declarative shadow root, whose
/// browser matches it alone gives, so such a wait escalates.
#[rstest::rstest]
#[case::its_matches(
    "<ul><li class=\"product\">a</li><li class=\"product\">b</li></ul>",
    "li.product, li.item",
    Some(2)
)]
#[case::none_yet(
    "<ul class=\"items\"></ul><script>fill()</script>",
    "li.product, li.item",
    Some(0)
)]
#[case::a_selector_it_cannot_parse("<p>a</p>", "p[", None)]
#[case::a_declarative_shadow_root(
    "<div><template shadowrootmode=\"open\"><li class=\"item\">a</li></template></div>",
    "li.item",
    None
)]
fn a_static_documents_matches_are_its_markups_or_unknown(
    #[case] html: &str,
    #[case] selector: &str,
    #[case] expected: Option<u64>,
) {
    assert_eq!(matches(html, selector), expected);
}
