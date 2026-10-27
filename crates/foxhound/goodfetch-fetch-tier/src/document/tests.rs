use goodfetch_frontier::trap_links::{Seen, Sight};

use super::{Matched, SelectorInvalid, matched};

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
