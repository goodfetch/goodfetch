use goodfetch_frontier::trap_links::{Seen, Sight};
use rstest::rstest;

use super::{Matched, SelectorInvalid, check_selector, first_elements, matched};

fn element(href: Option<&str>, text: &str, attributes: &[(&str, &str)]) -> Matched {
    Matched {
        href: href.map(str::to_owned),
        text: text.to_owned(),
        attributes: attributes
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect(),
        sight: Sight::Markup(Seen::default()),
    }
}

/// A locator matches its elements in document order, each with its `href`
/// as written, entities decoded, its text and its attributes; one with no
/// `href` is matched too.
///
/// Covers: story:6-232
#[test]
fn a_locator_matches_each_element_with_its_href_text_and_attributes() {
    let html = r#"<!doctype html><html><head><base href="/shelf/"></head><body>
<nav><a href="/home">Home</a></nav>
<ul class="books">
<li><a class="book" href="book/1?a=1&amp;b=2" data-id="B1">First <em>book</em></a></li>
<li><a class="book" name="anchor">No link</a></li>
<li><a class="book" href=" mailto:shop@links.test ">Mail</a></li>
</ul></body></html>"#;

    assert_eq!(
        matched(html, "ul.books a.book"),
        Ok(vec![
            element(
                Some("book/1?a=1&b=2"),
                "First book",
                &[
                    ("class", "book"),
                    ("href", "book/1?a=1&b=2"),
                    ("data-id", "B1")
                ],
            ),
            element(None, "No link", &[("class", "book"), ("name", "anchor")]),
            element(
                Some(" mailto:shop@links.test "),
                "Mail",
                &[("class", "book"), ("href", " mailto:shop@links.test ")],
            ),
        ])
    );
}

#[test]
fn a_selector_that_is_no_css_is_refused() {
    assert_eq!(
        matched("<a href=x>x</a>", "a >"),
        Err(SelectorInvalid("a >".to_owned()))
    );
}

/// A selector holding one of `dom_query`'s own extensions, which a browser
/// refuses, is refused on both tiers alike, so a handler's call answers
/// `invalid_params` wherever its page was served; the same text inside a
/// string or an escaped name is no extension.
///
/// Covers: story:6-232
#[rstest]
#[case::contains(r#"a:contains("Next")"#, false)]
#[case::has_text(r#"a:has-text("Next")"#, false)]
#[case::only_text("p:only-text", false)]
#[case::upper_case(r#"a:CONTAINS("Next")"#, false)]
#[case::inside_not(r#"li:not(:contains("x")) a"#, false)]
#[case::inside_has(r#"li:has(a:has-text("x"))"#, false)]
#[case::escaped_in_its_name(r#"a:\68 as-text("x")"#, false)]
#[case::in_an_attribute_value(r#"a[title=":contains(x)"]"#, true)]
#[case::an_escaped_colon_in_a_class(r".a\:contains", true)]
#[case::a_standard_pseudo_class("a:not(.ad):first-child", true)]
fn a_selector_a_browser_refuses_is_refused_on_both_tiers(
    #[case] selector: &str,
    #[case] accepted: bool,
) {
    let refused = Err(SelectorInvalid(selector.to_owned()));

    assert_eq!(check_selector(selector).is_ok(), accepted, "{selector}");
    assert_eq!(
        matched("<p><a href=x>x</a></p>", selector).is_ok(),
        accepted,
        "{selector}"
    );
    if !accepted {
        assert_eq!(check_selector(selector), refused);
        assert_eq!(
            first_elements("<p>x</p>", &[selector.to_owned()]),
            refused.map(|()| Vec::new())
        );
    }
}

/// A link a served document's markup hides inside an element a button
/// names in `aria-controls`, a disclosure, is one a person reveals; one
/// hidden in an element nothing controls is not.
///
/// Covers: story:8-228
#[test]
fn a_link_hidden_in_what_a_button_controls_reads_revealable() {
    let html = r#"<button aria-expanded="false" aria-controls="other more">More</button>
<div id="more" hidden><a href="/shown">Shown</a></div>
<div id="less" hidden><a href="/kept">Kept</a></div>"#;

    let revealable: Vec<bool> = matched(html, "a")
        .unwrap()
        .into_iter()
        .map(|element| match element.sight {
            Sight::Markup(seen) => {
                assert!(seen.no_box);
                seen.revealable
            }
            Sight::Layout(_) => unreachable!("a served document has no layout"),
        })
        .collect();

    assert_eq!(revealable, [true, false]);
}
