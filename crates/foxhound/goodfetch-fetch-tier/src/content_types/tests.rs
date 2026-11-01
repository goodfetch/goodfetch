use rstest::rstest;
use serde_json::{Value, json};

use crate::content_types::{Accepts, ContentTypes, is_document, typed};

/// `crawl.content_types` takes `type/subtype`, `type/*` and `*/*`, in any
/// case, and judges a response's essence by them; the built-in takes HTML
/// and XHTML alone.
///
/// Covers: story:6-264
#[rstest]
#[case::the_built_in_takes_html(None, "text/html", true)]
#[case::the_built_in_takes_xhtml(None, "application/xhtml+xml", true)]
#[case::the_built_in_refuses_a_pdf(None, "application/pdf", false)]
#[case::the_built_in_refuses_xml(None, "text/xml", false)]
#[case::an_exact_type(Some(json!(["application/pdf"])), "application/pdf", true)]
#[case::an_exact_type_refuses_another(Some(json!(["application/pdf"])), "text/html", false)]
#[case::a_type_in_another_case(Some(json!(["Application/PDF"])), "application/pdf", true)]
#[case::a_wildcard_subtype(Some(json!(["image/*"])), "image/png", true)]
#[case::a_wildcard_subtype_refuses_another_type(Some(json!(["image/*"])), "text/plain", false)]
#[case::every_type(Some(json!(["*/*"])), "application/zip", true)]
#[case::an_empty_list_takes_none(Some(json!([])), "text/html", false)]
fn a_list_of_media_ranges_judges_a_responses_type(
    #[case] set: Option<Value>,
    #[case] essence: &str,
    #[case] accepted: bool,
) {
    let types = set.map_or_else(ContentTypes::default, |set| ContentTypes::of(&set).unwrap());

    assert_eq!(types.accepts(essence), accepted);
}

/// A value that is no list of media ranges is refused, saying why.
///
/// Covers: story:6-264
#[rstest]
#[case::no_list(json!("text/html"))]
#[case::no_text(json!([1]))]
#[case::no_slash(json!(["html"]))]
#[case::an_empty_subtype(json!(["text/"]))]
#[case::a_wildcard_type_alone(json!(["*/html"]))]
#[case::a_parameter(json!(["text/html; charset=utf-8"]))]
fn a_value_that_is_no_list_of_media_ranges_is_refused(#[case] set: Value) {
    assert!(ContentTypes::of(&set).is_err(), "{set}");
}

/// A response's type is its `Content-Type` essence; with none, the type
/// Chromium's sniffer gives its body; with neither a type nor a body, an
/// empty HTML document, which a listed 301 is.
#[rstest]
#[case::declared(Some("Application/PDF; x=1"), b"<html>".as_slice(), "application/pdf")]
#[case::sniffed(None, b"%PDF-1.7".as_slice(), "application/pdf")]
#[case::sniffed_html(None, b"<!doctype html>".as_slice(), "text/html")]
#[case::neither_type_nor_body(None, b"".as_slice(), "text/html")]
#[case::any_type_is_sniffed(Some("*/*"), b"%PDF-1.7".as_slice(), "application/pdf")]
#[case::unknown_unknown_is_sniffed(Some("unknown/unknown"), b"%PDF-1.7".as_slice(), "application/pdf")]
#[case::application_unknown_is_sniffed(Some("Application/Unknown; x=1"), b"%PDF-1.7".as_slice(), "application/pdf")]
#[case::an_empty_type_is_sniffed(Some(" "), b"<!doctype html>".as_slice(), "text/html")]
#[case::no_slash_is_sniffed(Some("html"), b"<!doctype html>".as_slice(), "text/html")]
fn a_responses_type_is_its_header_else_its_sniffed_body(
    #[case] content_type: Option<&str>,
    #[case] body: &[u8],
    #[case] essence: &str,
) {
    assert_eq!(typed(content_type, body, "/").0, essence);
}

/// Only HTML and XHTML are documents.
#[rstest]
#[case("text/html", true)]
#[case("application/xhtml+xml", true)]
#[case("application/pdf", false)]
#[case("application/json", false)]
#[case("text/xml", false)]
fn only_html_and_xhtml_are_documents(#[case] essence: &str, #[case] document: bool) {
    assert_eq!(is_document(essence), document);
}

/// A response is judged by its type only when it would reach the handler:
/// a 2xx, or a status `crawl.handle_statuses` lists.
#[rstest]
#[case::a_2xx(200, true)]
#[case::a_listed_redirect(301, true)]
#[case::an_unlisted_redirect(302, false)]
#[case::an_unlisted_404(404, false)]
fn a_response_is_judged_only_when_it_would_reach_the_handler(
    #[case] status: u16,
    #[case] judged: bool,
) {
    let accepts = Accepts {
        types: ContentTypes::default(),
        handled: [301].into_iter().collect(),
    };

    assert_eq!(accepts.refused(status, "application/pdf").is_some(), judged);
    assert_eq!(accepts.refused(status, "text/html"), None);
}
