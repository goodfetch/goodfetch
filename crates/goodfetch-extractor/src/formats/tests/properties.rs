//! Properties of `formats` over generated documents.

use goodfetch_protocol::page::MainContent;
use proptest::prelude::*;
use serde_json::json;

use super::read;

/// A paragraph long enough to be an article's text.
const FILLER: &str = "<p>This paragraph is long enough to be read as the article's text by any main-content extractor, and it is followed by the generated words below.</p>";

/// Words of letters, marks and digits of any script, one space apart: text
/// Markdown writes as it stands, since none of it is Markdown's syntax.
fn words() -> impl Strategy<Value = String> {
    proptest::collection::vec("[\\p{L}\\p{M}\\p{N}]{1,12}", 1..12).prop_map(|words| words.join(" "))
}

/// Markup of a few elements around generated text, well formed or not.
fn markup() -> impl Strategy<Value = String> {
    let piece = prop_oneof![
        words(),
        Just("<p>".to_owned()),
        Just("</p>".to_owned()),
        Just("<a href=\"/x\">".to_owned()),
        Just("</a>".to_owned()),
        Just("<pre>".to_owned()),
        Just("<table><tr><th>".to_owned()),
        Just("<td>".to_owned()),
        Just("<h2>".to_owned()),
        Just("<nav>".to_owned()),
        Just(FILLER.to_owned()),
    ];
    proptest::collection::vec(piece, 0..24).prop_map(|pieces| pieces.concat())
}

proptest! {
    #![proptest_config(goodfetch_test_support::proptest_config())]

    /// A request's result holds the envelope and the formats it names,
    /// and no other key, whatever the document.
    /// Covers: story:6-8
    #[test]
    fn a_result_holds_the_formats_asked_and_no_other(html in markup(), markdown in any::<bool>()) {
        let request = if markdown { json!({ "markdown": true }) } else { json!({}) };

        let read = serde_json::to_value(read(&html, request)).unwrap();

        let keys: Vec<&str> = read.as_object().unwrap().keys().map(String::as_str).collect();
        let expected = if markdown {
            vec!["base_url", "main_content", "markdown", "url"]
        } else {
            vec!["base_url", "url"]
        };
        prop_assert_eq!(keys, expected);
    }

    /// A document with no main content answers empty Markdown.
    /// Covers: story:6-12
    #[test]
    fn no_main_content_is_empty_markdown(html in markup()) {
        let read = read(&html, json!({ "markdown": true }));

        if read.main_content == Some(MainContent::None) {
            prop_assert_eq!(read.markdown, Some(String::new()));
        }
    }

    /// An article's words come out as the document holds them, in any
    /// script and normalization form (ADR 0073).
    /// Covers: story:6-1
    #[test]
    fn an_articles_words_are_in_its_markdown_as_written(text in words()) {
        let html = format!("<!doctype html><html><head><title>Words</title></head><body><article>{FILLER}<p>{text}</p>{FILLER}</article></body></html>");

        let read = read(&html, json!({ "markdown": true }));

        let markdown = read.markdown.unwrap_or_default();
        prop_assert!(markdown.contains(&text), "{:?} not in {:?}", text, markdown);
    }
}
