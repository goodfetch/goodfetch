//! The request a page's one read takes under an Extractor Plugin: the
//! cleaned page in place of what the Plugin answers.

use goodfetch_protocol::page::FormatsRequest;
use rstest::rstest;
use serde_json::{Value, json};

use super::cleaned_request;

/// `asked` read as a `page.formats` request.
fn request(asked: Value) -> FormatsRequest {
    serde_json::from_value(asked).unwrap()
}

/// The read keeps every format the Plugin does not answer, the whole
/// body's Markdown among them, and reads the cleaned page for it.
///
/// Covers: setting:content.extractor
#[rstest]
#[case::main_markdown_and_text(
    json!({ "markdown": true, "text": true, "links": true }),
    json!({ "html": { "kind": "cleaned" }, "links": true })
)]
#[case::fit(
    json!({ "markdown": { "content": "fit" }, "metadata": true }),
    json!({ "html": { "kind": "cleaned" }, "metadata": true })
)]
#[case::full_markdown(
    json!({ "markdown": { "content": "full" }, "text": true }),
    json!({ "markdown": { "content": "full" }, "html": { "kind": "cleaned" } })
)]
#[case::raw_html(
    json!({ "html": { "kind": "raw" }, "text": true }),
    json!({ "html": { "kind": "cleaned" } })
)]
fn the_read_takes_the_cleaned_page_for_the_plugin(#[case] asked: Value, #[case] read: Value) {
    assert_eq!(
        cleaned_request(request(asked), "kettle").unwrap(),
        request(read)
    );
}

/// A Plugin answers no main HTML.
///
/// Covers: setting:content.extractor
#[rstest]
fn the_main_html_is_refused(#[values(json!(true), json!({ "kind": "main" }))] html: Value) {
    let refused =
        cleaned_request(request(json!({ "html": html, "text": true })), "kettle").unwrap_err();

    assert_eq!(
        (refused.code(), refused.message().contains("`kettle`")),
        ("invalid_params", true)
    );
}
