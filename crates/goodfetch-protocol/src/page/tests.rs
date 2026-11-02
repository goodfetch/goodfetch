//! The formats request and the schema of its result (ADR 0103).

use rstest::rstest;
use serde_json::{Value, json};

use super::{
    FormatsRequest, HtmlFormat, HtmlKind, MarkdownContent, MarkdownFormat, page_formats_schema,
};

#[expect(
    clippy::disallowed_methods,
    reason = "a test's own validator, on no Shared path (ADR 0140)"
)]
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
#[case::main_content_missing(json!({ "url": "https://a.test/", "base_url": "https://a.test/", "markdown": "# A" }), false)]
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
#[case::the_envelope_alone(json!({ "url": "https://a.test/", "base_url": "https://a.test/" }), true)]
#[case::main_content_not_asked(json!({ "url": "https://a.test/", "base_url": "https://a.test/", "main_content": "none" }), false)]
#[case::markdown_not_asked(json!({ "url": "https://a.test/", "base_url": "https://a.test/", "markdown": "# A" }), false)]
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
fn markdown_is_asked_for_as_true_or_its_options_and_holds_the_main_content(#[case] value: Value) {
    let request = request(value);

    assert_eq!(
        request.markdown.map(MarkdownFormat::content),
        Some(MarkdownContent::Main)
    );
}

/// Covers: story:6-2
#[rstest]
#[case::main(json!({ "content": "main" }), MarkdownContent::Main)]
#[case::full(json!({ "content": "full" }), MarkdownContent::Full)]
#[case::fit(json!({ "content": "fit" }), MarkdownContent::Fit)]
fn markdown_holds_the_content_its_options_name(
    #[case] options: Value,
    #[case] content: MarkdownContent,
) {
    let request = request(json!({ "markdown": options }));

    assert_eq!(request.markdown.map(MarkdownFormat::content), Some(content));
}

/// Covers: story:6-3
#[rstest]
#[case::true_(json!(true), HtmlKind::Main)]
#[case::raw(json!({ "kind": "raw" }), HtmlKind::Raw)]
#[case::cleaned(json!({ "kind": "cleaned" }), HtmlKind::Cleaned)]
#[case::main(json!({ "kind": "main" }), HtmlKind::Main)]
fn html_holds_the_kind_its_options_name_and_the_main_content_by_default(
    #[case] options: Value,
    #[case] kind: HtmlKind,
) {
    let request = request(json!({ "html": options }));

    assert_eq!(request.html.map(HtmlFormat::kind), Some(kind));
}

/// Covers: story:6-3
#[test]
fn the_schema_of_an_html_and_text_requests_result_holds_both() {
    let schema = page_formats_schema(&request(json!({ "html": true, "text": true })));
    let envelope =
        json!({ "url": "https://a.test/", "base_url": "https://a.test/", "main_content": "found" });
    let mut both = envelope.clone();
    both["html"] = json!("<p>A</p>");
    both["text"] = json!("A");
    let mut html_only = envelope;
    html_only["html"] = json!("<p>A</p>");

    assert_eq!(
        (valid(&schema, &both), valid(&schema, &html_only)),
        (true, false)
    );
}

#[rstest]
#[case::false_(json!({ "markdown": false }))]
#[case::options_without_content(json!({ "markdown": {} }))]
#[case::an_unknown_content(json!({ "markdown": { "content": "everything" } }))]
#[case::an_unknown_format(json!({ "screenshot": true }))]
#[case::an_unknown_kind(json!({ "html": { "kind": "pretty" } }))]
#[case::an_unknown_option(json!({ "html": { "kind": "raw", "pretty": true } }))]
#[case::text_with_options(json!({ "text": { "content": "full" } }))]
fn a_request_naming_no_format_the_protocol_has_is_refused(#[case] value: Value) {
    let parsed = serde_json::from_value::<FormatsRequest>(value);

    assert!(parsed.is_err(), "{parsed:?}");
}

/// The result of a request for the page's references and tables, each
/// field given.
fn references_and_tables() -> Value {
    json!({
        "url": "https://a.test/",
        "base_url": "https://a.test/",
        "links": [
            { "url": "https://a.test/b", "href": "b", "text": "B", "rel": ["next"] },
            { "url": null, "href": "http://[::1", "text": "Broken", "rel": [] },
        ],
        "images": [{ "url": "https://a.test/c.png", "src": "c.png", "alt": null }],
        "media": [{ "url": "https://a.test/d.mp4", "src": "d.mp4", "kind": "video", "type": "video/mp4" }],
        "tables": [{ "caption": "E", "header_rows": [["", "F"]], "body_rows": [["G", "1"]] }],
    })
}

/// Covers: story:6-4, story:6-5
#[rstest]
#[case::every_field_given_a_url_that_does_not_parse_null(references_and_tables(), true)]
#[case::a_link_without_its_url(remove(references_and_tables(), "/links/0", "url"), false)]
#[case::an_image_without_its_alt(remove(references_and_tables(), "/images/0", "alt"), false)]
#[case::media_of_an_unknown_kind(set(references_and_tables(), "/media/0/kind", json!("image")), false)]
#[case::a_table_without_its_caption(remove(references_and_tables(), "/tables/0", "caption"), false)]
#[case::the_tables_missing(remove(references_and_tables(), "", "tables"), false)]
fn the_schema_of_a_references_and_tables_requests_result_holds_each(
    #[case] result: Value,
    #[case] holds: bool,
) {
    let schema = page_formats_schema(&request(
        json!({ "links": true, "images": true, "media": true, "tables": true }),
    ));

    assert_eq!(valid(&schema, &result), holds);
}

/// `value` without the field `field` of the object at `pointer`.
fn remove(mut value: Value, pointer: &str, field: &str) -> Value {
    value
        .pointer_mut(pointer)
        .and_then(Value::as_object_mut)
        .map(|object| object.remove(field));
    value
}

/// `value` with `replacement` at `pointer`.
fn set(mut value: Value, pointer: &str, replacement: Value) -> Value {
    if let Some(slot) = value.pointer_mut(pointer) {
        *slot = replacement;
    }
    value
}

/// Covers: story:6-4, story:6-5
#[rstest]
#[case::links(json!({ "links": { "rel": true } }))]
#[case::images_false(json!({ "images": false }))]
#[case::tables_with_options(json!({ "tables": { "headers": true } }))]
fn references_and_tables_are_asked_for_with_true_alone(#[case] value: Value) {
    let parsed = serde_json::from_value::<FormatsRequest>(value);

    assert!(parsed.is_err(), "{parsed:?}");
}

/// The result of a request for the page's metadata and contacts, each
/// field given.
fn metadata_and_contacts() -> Value {
    json!({
        "url": "https://a.test/",
        "base_url": "https://a.test/",
        "metadata": {
            "title": "A",
            "description": null,
            "canonical": "https://a.test/a",
            "lang": "fr-FR",
            "open_graph": [{ "property": "og:title", "content": "A" }],
            "twitter": [{ "property": "twitter:card", "content": "summary" }],
            "json_ld": [{ "@context": "https://schema.org", "@type": "Thing" }],
            "microdata": [{
                "type": ["https://schema.org/Product"],
                "id": null,
                "properties": { "name": ["A"], "offers": [{ "type": [], "id": null, "properties": { "price": ["1"] } }] },
            }],
            "meta": { "author": "B" },
        },
        "contacts": {
            "emails": [{ "value": "a@a.test", "text": "Write to us" }],
            "phones": [
                { "value": "01 23 45 67 89", "text": "01 23 45 67 89", "e164": "+33123456789" },
                { "value": "0123", "text": "Call", "e164": null },
            ],
            "social": [{ "value": "https://github.com/a", "text": "A on GitHub", "network": "github" }],
        },
    })
}

/// Covers: story:6-6, story:6-7
#[rstest]
#[case::every_field_given(metadata_and_contacts(), true)]
#[case::a_title_missing(remove(metadata_and_contacts(), "/metadata", "title"), false)]
#[case::json_ld_missing(remove(metadata_and_contacts(), "/metadata", "json_ld"), false)]
#[case::a_microdata_item_without_its_id(
    remove(metadata_and_contacts(), "/metadata/microdata/0", "id"),
    false
)]
#[case::a_microdata_value_neither_text_nor_item(set(metadata_and_contacts(), "/metadata/microdata/0/properties/name/0", json!(1)), false)]
#[case::a_phone_without_e164(remove(metadata_and_contacts(), "/contacts/phones/1", "e164"), false)]
#[case::a_profile_of_an_unknown_network(set(metadata_and_contacts(), "/contacts/social/0/network", json!("myspace")), false)]
#[case::the_contacts_missing(remove(metadata_and_contacts(), "", "contacts"), false)]
fn the_schema_of_a_metadata_and_contacts_requests_result_holds_each(
    #[case] result: Value,
    #[case] holds: bool,
) {
    let schema = page_formats_schema(&request(json!({ "metadata": true, "contacts": true })));

    assert_eq!(valid(&schema, &result), holds);
}

/// Covers: story:6-6, story:6-7
#[rstest]
#[case::metadata_with_options(json!({ "metadata": { "json_ld": true } }))]
#[case::contacts_false(json!({ "contacts": false }))]
fn metadata_and_contacts_are_asked_for_with_true_alone(#[case] value: Value) {
    let parsed = serde_json::from_value::<FormatsRequest>(value);

    assert!(parsed.is_err(), "{parsed:?}");
}
