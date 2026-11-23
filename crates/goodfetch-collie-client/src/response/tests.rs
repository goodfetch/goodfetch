//! A response's body as text and as JSON (ADR 0119): `text()` decodes with
//! the response's own encoding, a navigation's document with the one its
//! document was decoded with, any other with its BOM, its `Content-Type`
//! charset, Chrome's `<meta>` prescan for HTML or the XML declaration for
//! an XML type, then UTF-8; `json()` decodes UTF-8 and ignores a BOM.
//! Covers: story:11-85
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use goodfetch_protocol::error_code::ErrorCode;
use rstest::rstest;
use serde_json::{Value, json};

use super::decoding::{json, text};

/// The URL of the response a JSON test reads.
const URL: &str = "https://shop.test/data.json";

/// 日本語 in `Shift_JIS`.
const SHIFT_JIS: [u8; 6] = [0x93, 0xFA, 0x96, 0x7B, 0x8C, 0xEA];

/// `é` in windows-1252.
const E_ACUTE: u8 = 0xE9;

/// `head`, then the `Shift_JIS` bytes of 日本語, then `tail`.
fn with_shift_jis(head: &str, tail: &str) -> Vec<u8> {
    [head.as_bytes(), &SHIFT_JIS, tail.as_bytes()].concat()
}

/// `head`, then the windows-1252 byte of `é`, then `tail`.
fn with_e_acute(head: &str, tail: &str) -> Vec<u8> {
    [head.as_bytes(), &[E_ACUTE], tail.as_bytes()].concat()
}

#[test]
fn a_navigations_document_decodes_with_the_encoding_its_document_was_decoded_with() {
    let body = with_shift_jis("<p>", "</p>");

    let decoded = text(&body, Some("Shift_JIS"), Some("text/html; charset=utf-8"));

    assert_eq!(decoded, "<p>日本語</p>");
}

#[test]
fn a_bom_wins_over_the_content_type_charset_and_is_left_out() {
    let body = [b"\xEF\xBB\xBF".as_slice(), "café".as_bytes()].concat();

    let decoded = text(&body, None, Some("text/plain; charset=windows-1252"));

    assert_eq!(decoded, "café");
}

#[rstest]
#[case::plain("text/plain; charset=Shift_JIS")]
#[case::quoted("text/plain; charset=\"shift_jis\"")]
#[case::among_parameters("text/plain; format=flowed; Charset=sjis")]
fn a_legacy_content_type_charset_decodes_the_body(#[case] content_type: &str) {
    let decoded = text(&SHIFT_JIS, None, Some(content_type));

    assert_eq!(decoded, "日本語");
}

/// The `<meta>` reads as Chrome's prescan reads it, as the fetch tiers
/// read it: one among head tags past the first 1,024 bytes is still read,
/// where the HTML standard's byte-level prescan stops at 1,024.
#[rstest]
#[case::charset_attribute(
    "<!doctype html><html><head><meta charset=\"shift_jis\"><title>",
    "</title>"
)]
#[case::pragma(
    "<html><head><meta http-equiv=\"Content-Type\" content=\"text/html; charset=Shift_JIS\"><title>",
    "</title>"
)]
#[case::late_among_head_tags(
    &format!("<html><head>{}<meta charset=sjis><title>", "<meta name=k content=v>".repeat(50)),
    "</title>"
)]
fn an_html_body_declaring_its_encoding_in_a_meta_decodes_with_it(
    #[case] head: &str,
    #[case] tail: &str,
) {
    let body = with_shift_jis(head, tail);

    let decoded = text(&body, None, Some("text/html"));

    assert_eq!(decoded, format!("{head}日本語{tail}"));
}

#[test]
fn a_content_type_charset_no_encoding_has_gives_way_to_the_meta() {
    let head = "<meta charset=\"windows-1252\"><p>";
    let body = with_e_acute(head, "</p>");

    let decoded = text(&body, None, Some("text/html; charset=no-such-encoding"));

    assert_eq!(decoded, format!("{head}é</p>"));
}

#[rstest]
#[case::xml("application/xml")]
#[case::text_xml("text/xml")]
#[case::suffixed("application/atom+xml")]
#[case::html_without_a_meta("text/html")]
fn an_xml_declaration_at_the_start_decodes_the_body(#[case] content_type: &str) {
    let head = "<?xml version=\"1.0\" encoding=\"windows-1252\"?><a>";
    let body = with_e_acute(head, "</a>");

    let decoded = text(&body, None, Some(content_type));

    assert_eq!(decoded, format!("{head}é</a>"));
}

#[rstest]
#[case::another_type(Some("text/plain"))]
#[case::no_type(None)]
fn a_body_of_another_type_reads_no_declaration_and_decodes_as_utf_8(
    #[case] content_type: Option<&str>,
) {
    let head = "<?xml version=\"1.0\" encoding=\"windows-1252\"?><a>";
    let body = with_e_acute(head, "</a>");

    let decoded = text(&body, None, content_type);

    assert_eq!(decoded, format!("{head}\u{FFFD}</a>"));
}

#[test]
fn json_ignores_a_leading_bom_and_decodes_utf_8_whatever_the_charset() {
    let body = [b"\xEF\xBB\xBF".as_slice(), "{\"name\":\"café\"}".as_bytes()].concat();

    let parsed: Value = json(&body, URL.to_owned()).unwrap();

    assert_eq!(parsed, json!({ "name": "café" }));
}

/// Covers: error:response_not_json
#[test]
fn json_of_a_body_that_is_no_json_fails_with_response_not_json_naming_its_url() {
    let refused = json::<Value>(b"<html></html>", URL.to_owned()).unwrap_err();

    assert_eq!(
        (refused.code(), refused.url()),
        (ErrorCode::RESPONSE_NOT_JSON, Some(URL))
    );
}
