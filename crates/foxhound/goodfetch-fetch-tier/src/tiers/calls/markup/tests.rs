//! Covers: story:6-259
#![expect(
    clippy::disallowed_methods,
    reason = "ADR 0138: a test's own parse, on no page's path"
)]

use super::{Shown, shown};

/// The markup decides an element's visibility where it hides it itself,
/// and leaves to the browser what needs layout or a strict single match,
/// and an element it would call visible in a document a stylesheet may
/// hide it in.
#[rstest::rstest]
#[case::text("<h1>title</h1>", "h1", Some(Shown::Visible))]
#[case::absent("<h1>title</h1>", "h2", Some(Shown::Absent))]
#[case::hidden_attribute("<p hidden>a</p>", "p", Some(Shown::Hidden))]
#[case::under_a_hidden_ancestor("<div hidden><p>a</p></div>", "p", Some(Shown::Hidden))]
#[case::display_none(
    "<div style=\"color: red; DISPLAY : none !important\"><p>a</p></div>",
    "p",
    Some(Shown::Hidden)
)]
#[case::display_none_overridden(
    "<p style=\"display: none; display: block\">a</p>",
    "p",
    Some(Shown::Visible)
)]
#[case::visibility_hidden("<p style=\"visibility:hidden\">a</p>", "p", Some(Shown::Hidden))]
#[case::visibility_restored_inside(
    "<div style=\"visibility: hidden\"><p style=\"visibility: visible\">a</p></div>",
    "p",
    Some(Shown::Visible)
)]
#[case::a_templates_content_is_no_element(
    "<template><p>a</p></template>",
    "p",
    Some(Shown::Absent)
)]
#[case::in_the_head("<head><title>a</title></head><p>b</p>", "title", Some(Shown::Hidden))]
#[case::a_hidden_input("<input type=\"Hidden\" value=\"a\">", "input", Some(Shown::Hidden))]
#[case::an_image("<p><img src=\"a.png\"></p>", "p", Some(Shown::Visible))]
#[case::nothing_to_draw("<ul class=\"items\"></ul>", "ul", None)]
#[case::in_closed_details("<details><p>a</p></details>", "p", None)]
#[case::in_open_details("<details open><p>a</p></details>", "p", Some(Shown::Visible))]
#[case::two_matches("<p>a</p><p>b</p>", "p", None)]
#[case::a_selector_it_cannot_parse("<p>a</p>", "p[", None)]
#[case::a_declarative_shadow_root(
    "<div><template shadowrootmode=\"open\"><p>a</p></template></div>",
    "div",
    None
)]
#[case::styled_by_a_style_element(
    "<style>.modal { display: none }</style><p class=\"modal\">a</p>",
    "p",
    None
)]
#[case::styled_by_a_stylesheet(
    "<head><link rel=\"Preload StyleSheet\" href=\"a.css\"></head><p>a</p>",
    "p",
    None
)]
#[case::hidden_whatever_its_stylesheet(
    "<link rel=\"stylesheet\" href=\"a.css\"><p hidden>a</p>",
    "p",
    Some(Shown::Hidden)
)]
#[case::absent_whatever_its_stylesheet(
    "<style>p { color: red }</style><p>a</p>",
    "h2",
    Some(Shown::Absent)
)]
#[case::a_link_that_is_no_stylesheet(
    "<link rel=\"icon\" href=\"a.png\"><p>a</p>",
    "p",
    Some(Shown::Visible)
)]
fn an_elements_visibility_is_its_markups_or_unknown(
    #[case] html: &str,
    #[case] selector: &str,
    #[case] expected: Option<Shown>,
) {
    let work = goodfetch_counted_work::CountedWork::default();
    let tree = work.parse_html(goodfetch_counted_work::Row::PageParses, html);

    assert_eq!(shown((&tree, html), selector, &work), expected);
}
