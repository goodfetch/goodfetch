#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use encoding_rs::{Encoding, SHIFT_JIS, UTF_8, WINDOWS_1252};
use rstest::rstest;

use super::{Matched, SelectorInvalid, decode, matched};

/// 日本 in Shift_JIS.
const NIHON: &[u8] = &[0x93, 0xFA, 0x96, 0x7B];

/// A document is read in its BOM's encoding, else its `Content-Type`
/// charset's, in any case and quoted, else as UTF-8; an unknown label is
/// none.
///
/// Covers: story:6-233
#[rstest]
#[case::a_charset(NIHON, Some("text/html; charset=Shift_JIS"), "日本", SHIFT_JIS)]
#[case::a_quoted_label_in_another_case(
    NIHON,
    Some("text/html;Charset=\"shift_jis\""),
    "日本",
    SHIFT_JIS
)]
#[case::a_label_alias(&[0xE9], Some("text/html; charset=latin1"), "é", WINDOWS_1252)]
#[case::a_bom_before_the_charset(
    &[0xEF, 0xBB, 0xBF, 0xE6, 0x97, 0xA5],
    Some("text/html; charset=Shift_JIS"),
    "日",
    UTF_8
)]
#[case::no_charset("日".as_bytes(), Some("text/html"), "日", UTF_8)]
#[case::an_unknown_label("日".as_bytes(), Some("text/html; charset=klingon"), "日", UTF_8)]
#[case::no_content_type(&[0xFF, b'a'], None, "\u{FFFD}a", UTF_8)]
fn a_document_is_decoded_in_its_bom_its_charset_or_utf_8(
    #[case] body: &[u8],
    #[case] content_type: Option<&str>,
    #[case] text: &str,
    #[case] encoding: &'static Encoding,
) {
    assert_eq!(decode(body, content_type), (text.to_owned(), encoding));
}

fn element(href: Option<&str>, text: &str, attributes: &[(&str, &str)]) -> Matched {
    Matched {
        href: href.map(str::to_owned),
        text: text.to_owned(),
        attributes: attributes
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect(),
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
