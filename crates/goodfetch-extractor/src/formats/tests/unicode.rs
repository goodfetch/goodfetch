//! Text as the document holds it (ADR 0073): the Unicode record's fields
//! come out of `formats` byte for byte, never repaired or normalized.

use goodfetch_test_support::unicode_record;
use rstest::rstest;
use serde_json::json;

use super::read;

/// A document holding each field of the Unicode record in a paragraph of
/// its own, between paragraphs long enough to be the page's main content.
/// White space outside `<pre>` is collapsed as HTML renders it, so the
/// field that holds a tab and a newline is in a `<pre>`.
fn unicode_document() -> String {
    let filler = "<p>This paragraph is long enough to be read as the article's text by any main-content extractor, and it holds no character the record does.</p>";
    let fields: String = unicode_record()
        .fields
        .iter()
        .map(|field| {
            if field.text.contains(['\t', '\n']) {
                format!("<pre>{}</pre>", field.text)
            } else {
                format!("<p>{}</p>", field.text)
            }
        })
        .collect();
    format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>Record</title></head><body><article>{filler}{fields}{filler}</article></body></html>"
    )
}

/// Covers: story:6-1
#[rstest]
#[case::bidi("bidi")]
#[case::nfc("nfc")]
#[case::nfd("nfd")]
#[case::zwj_emoji("zwj_emoji")]
#[case::astral("astral")]
#[case::zero_width_joiner("zero_width_joiner")]
#[case::bom("bom")]
#[case::tab_and_newline("tab_and_newline")]
fn each_field_of_the_unicode_record_is_in_the_markdown_byte_for_byte(#[case] name: &str) {
    let record = unicode_record();
    let field = record
        .fields
        .iter()
        .find(|field| field.name == name)
        .unwrap();

    let read = read(&unicode_document(), json!({ "markdown": true }));

    let markdown = read.markdown.unwrap();
    assert!(
        markdown.contains(&field.text),
        "{name}: {:?} not in {markdown:?}",
        field.text
    );
}
