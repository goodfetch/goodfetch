//! The formats request and the schema of its result (ADR 0103).

use rstest::rstest;
use serde_json::{Value, json};

use super::{FormatsRequest, MarkdownContent, page_formats_schema};

fn valid(schema: &Value, instance: &Value) -> bool {
    jsonschema::validator_for(schema)
        .expect("the result's schema is a JSON Schema")
        .is_valid(instance)
}

fn request(value: Value) -> FormatsRequest {
    serde_json::from_value(value).unwrap()
}

/// Covers: story:6-66
#[rstest]
#[case::markdown_given(json!({ "url": "https://a.test/", "base_url": "https://a.test/", "main_content": "found", "markdown": "# A" }), true)]
#[case::markdown_empty_with_no_main_content(json!({ "url": "https://a.test/", "base_url": "https://a.test/", "main_content": "none", "markdown": "" }), true)]
#[case::markdown_missing(json!({ "url": "https://a.test/", "base_url": "https://a.test/", "main_content": "found" }), false)]
#[case::base_url_missing(json!({ "url": "https://a.test/", "main_content": "found", "markdown": "# A" }), false)]
#[case::main_content_unknown(json!({ "url": "https://a.test/", "base_url": "https://a.test/", "main_content": "partial", "markdown": "# A" }), false)]
#[case::a_format_not_asked(json!({ "url": "https://a.test/", "base_url": "https://a.test/", "main_content": "found", "markdown": "# A", "html": "<h1>A</h1>" }), false)]
fn the_schema_of_a_markdown_requests_result_holds_markdown_and_the_envelope(
    #[case] result: Value,
    #[case] holds: bool,
) {
    let schema = page_formats_schema(&request(json!({ "markdown": true })));

    assert_eq!(valid(&schema, &result), holds);
}

/// Covers: story:6-66, story:6-8
#[rstest]
#[case::the_envelope_alone(json!({ "url": "https://a.test/", "base_url": "https://a.test/", "main_content": "none" }), true)]
#[case::markdown_not_asked(json!({ "url": "https://a.test/", "base_url": "https://a.test/", "main_content": "found", "markdown": "# A" }), false)]
fn the_schema_of_an_empty_requests_result_has_no_format(
    #[case] result: Value,
    #[case] holds: bool,
) {
    let schema = page_formats_schema(&request(json!({})));

    assert_eq!(valid(&schema, &result), holds);
}

#[rstest]
#[case::true_(json!({ "markdown": true }))]
#[case::options(json!({ "markdown": { "content": "main" } }))]
#[case::options_left_out(json!({ "markdown": {} }))]
fn markdown_is_asked_for_as_true_or_its_options_and_holds_the_main_content(#[case] value: Value) {
    let request = request(value);

    assert_eq!(
        request.markdown.map(super::MarkdownFormat::content),
        Some(MarkdownContent::Main)
    );
}

#[rstest]
#[case::false_(json!({ "markdown": false }))]
#[case::an_unknown_content(json!({ "markdown": { "content": "everything" } }))]
#[case::an_unknown_format(json!({ "screenshot": true }))]
fn a_request_naming_no_format_the_protocol_has_is_refused(#[case] value: Value) {
    let parsed = serde_json::from_value::<FormatsRequest>(value);

    assert!(parsed.is_err(), "{parsed:?}");
}
