//! A code-free crawl's fields as pure functions of the element each
//! locator found (ADR 0123): the value each Value transform gives, what
//! the Record's Provenance names of it, the reason a field is null, and the
//! type each field takes in the Dataset's schema.

use goodfetch_fetch_tier::FieldElement;
use goodfetch_protocol::foxhound::crawl::ExtractionField;
use goodfetch_protocol::foxhound::provenance::{TransformProvenance, UnreadField};
use rstest::rstest;
use serde_json::{Value, json};

use super::{check, read, schema};

const BASE: &str = "http://shop.test/kites/delta";

fn field(value: Value) -> ExtractionField {
    serde_json::from_value(value).unwrap()
}

fn element(tag: &str, text: &str, attributes: &[(&str, &str)]) -> FieldElement {
    FieldElement {
        tag: tag.to_owned(),
        text: text.to_owned(),
        attributes: attributes
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect(),
    }
}

fn transforms(value: Value) -> Vec<TransformProvenance> {
    serde_json::from_value(value).unwrap()
}

fn unread(value: Value) -> Option<UnreadField> {
    serde_json::from_value(value).unwrap()
}

/// A field reads the first element's text with its runs of white space
/// read as one space, or an attribute, an `href` resolved against the
/// base URL; a `match` picks a group of that text; `number`, `date` and
/// `date_time` read a machine-readable value in its invariant form,
/// whatever the locale, and a `date_time` stating no offset takes its
/// `time_zone`'s; each gives its value, and Provenance names each
/// transform with the text it read, its locale and its zone.
#[rstest]
#[case::text_with_its_white_space_collapsed(
    json!({ "locator": { "kind": "css", "selector": "h1" } }),
    element("h1", "\n  Delta \t kite\n", &[]),
    json!("Delta kite"),
    json!([{ "field": "/name", "text": "\n  Delta \t kite\n", "transform": "text" }]),
)]
#[case::an_href_resolved_against_the_base_url(
    json!({ "locator": { "kind": "css", "selector": "a.manual" }, "attribute": "href" }),
    element("a", "Manual", &[("class", "manual"), ("href", "../manuals/delta.pdf")]),
    json!("http://shop.test/manuals/delta.pdf"),
    json!([{ "field": "/name", "text": "../manuals/delta.pdf", "transform": "text" }]),
)]
#[case::a_match_then_text(
    json!({ "locator": { "kind": "css", "selector": ".sku" }, "match": { "pattern": "SKU: (\\S+)" } }),
    element("p", "SKU: DK-0042 (new)", &[]),
    json!("DK-0042"),
    json!([
        { "field": "/name", "text": "SKU: DK-0042 (new)", "transform": "match" },
        { "field": "/name", "text": "DK-0042", "transform": "text" },
    ]),
)]
#[case::a_number_from_a_data_value_under_de_de(
    json!({ "locator": { "kind": "css", "selector": ".price" }, "transform": { "kind": "number", "locale": "de-DE" } }),
    element("data", "1.099,98 €", &[("class", "price"), ("value", "1099.980")]),
    json!(1099.98),
    json!([{ "field": "/name", "text": "1099.980", "transform": "number", "locale": "de-DE" }]),
)]
#[case::a_number_from_a_content_attribute(
    json!({ "locator": { "kind": "css", "selector": "[itemprop=price]" }, "transform": { "kind": "number", "locale": "en-GB" } }),
    element("span", "£49.90", &[("itemprop", "price"), ("content", "49.90")]),
    json!(49.9),
    json!([{ "field": "/name", "text": "49.90", "transform": "number", "locale": "en-GB" }]),
)]
#[case::a_date_from_a_time_datetime(
    json!({ "locator": { "kind": "css", "selector": "time" }, "transform": { "kind": "date", "locale": "de-DE" } }),
    element("time", "1. Februar 2026", &[("datetime", "2026-02-01")]),
    json!("2026-02-01"),
    json!([{ "field": "/name", "text": "2026-02-01", "transform": "date", "locale": "de-DE" }]),
)]
#[case::a_date_time_with_no_offset_takes_its_time_zones(
    json!({ "locator": { "kind": "css", "selector": "time" }, "transform": { "kind": "date_time", "locale": "de-DE", "time_zone": "Europe/Berlin" } }),
    element("time", "1. Februar 2026, 10:00", &[("datetime", "2026-02-01T10:00")]),
    json!("2026-02-01T10:00:00+01:00"),
    json!([{ "field": "/name", "text": "2026-02-01T10:00", "transform": "date_time", "locale": "de-DE", "time_zone": "Europe/Berlin" }]),
)]
#[case::a_date_time_keeps_the_offset_its_text_states(
    json!({ "locator": { "kind": "css", "selector": "time" }, "transform": { "kind": "date_time", "locale": "en-IN", "time_zone": "Europe/Berlin" } }),
    element("time", "1 Feb 2026, 10:00 IST", &[("datetime", "2026-02-01T10:00:00+05:30")]),
    json!("2026-02-01T10:00:00+05:30"),
    json!([{ "field": "/name", "text": "2026-02-01T10:00:00+05:30", "transform": "date_time", "locale": "en-IN" }]),
)]
fn a_field_gives_its_transforms_value_and_provenance_names_each_transform(
    #[case] declared: Value,
    #[case] found: FieldElement,
    #[case] value: Value,
    #[case] named: Value,
) {
    let read = read("name", &field(declared), Some(&found), BASE);

    assert_eq!(
        (read.value, read.transforms, read.unread),
        (value, transforms(named), None)
    );
}

/// A field its page does not give is null, never an error, and
/// Provenance names why, with the text where one was read: no element, no
/// such attribute, a `match` that did not match, a text printed for people
/// (the locale's parsers are #844's and #848's), a decimal a JSON number
/// cannot hold exactly, an impossible date, and a `date_time` stating no
/// offset with no `time_zone`.
#[rstest]
#[case::no_element(
    json!({ "locator": { "kind": "css", "selector": ".discount" } }),
    None,
    json!({ "field": "/name", "reason": "no_element" }),
)]
#[case::no_such_attribute(
    json!({ "locator": { "kind": "css", "selector": "a" }, "attribute": "href" }),
    Some(element("a", "Manual", &[])),
    json!({ "field": "/name", "reason": "no_attribute" }),
)]
#[case::a_match_that_does_not_match(
    json!({ "locator": { "kind": "css", "selector": ".sku" }, "match": { "pattern": "SKU: (\\S+)" } }),
    Some(element("p", "Ref. DK-0042", &[])),
    json!({ "field": "/name", "reason": "no_match", "text": "Ref. DK-0042" }),
)]
#[case::a_printed_number(
    json!({ "locator": { "kind": "css", "selector": ".price" }, "transform": { "kind": "number", "locale": "de-DE" } }),
    Some(element("span", "1.099,98\u{a0}€", &[])),
    json!({ "field": "/name", "reason": "not_parsed", "text": "1.099,98 €" }),
)]
#[case::a_decimal_no_json_number_holds_exactly(
    json!({ "locator": { "kind": "css", "selector": "data" }, "transform": { "kind": "number", "locale": "en-US" } }),
    Some(element("data", "", &[("value", "12345678901234567890.12")])),
    json!({ "field": "/name", "reason": "not_parsed", "text": "12345678901234567890.12" }),
)]
#[case::an_impossible_date(
    json!({ "locator": { "kind": "css", "selector": "time" }, "transform": { "kind": "date", "locale": "en-GB" } }),
    Some(element("time", "", &[("datetime", "2026-02-30")])),
    json!({ "field": "/name", "reason": "not_parsed", "text": "2026-02-30" }),
)]
#[case::a_date_time_with_no_zone_anywhere(
    json!({ "locator": { "kind": "css", "selector": "time" }, "transform": { "kind": "date_time", "locale": "de-DE" } }),
    Some(element("time", "", &[("datetime", "2026-02-01T10:00")])),
    json!({ "field": "/name", "reason": "no_time_zone", "text": "2026-02-01T10:00" }),
)]
fn a_field_its_page_does_not_give_is_null_with_its_reason(
    #[case] declared: Value,
    #[case] found: Option<FieldElement>,
    #[case] reason: Value,
) {
    let read = read("name", &field(declared), found.as_ref(), BASE);

    assert_eq!((read.value, read.unread), (Value::Null, unread(reason)));
}

/// A field name is written into its JSON Pointer escaped, `~` and `/` as
/// RFC 6901 writes them.
#[test]
fn a_fields_pointer_escapes_its_name() {
    let read = read(
        "a/b~c",
        &field(json!({ "locator": { "kind": "css", "selector": "p" } })),
        None,
        BASE,
    );

    assert_eq!(read.unread.unwrap().field, "/a~1b~0c");
}

/// Each field takes the type its transform gives, or null.
#[rstest]
#[case::text(json!({}), json!({ "type": ["string", "null"] }))]
#[case::number(json!({ "kind": "number", "locale": "de-DE" }), json!({ "type": ["number", "null"] }))]
#[case::date(json!({ "kind": "date", "locale": "de-DE" }), json!({ "type": ["string", "null"], "format": "date" }))]
#[case::date_time(json!({ "kind": "date_time", "locale": "de-DE" }), json!({ "type": ["string", "null"], "format": "date-time" }))]
fn a_fields_schema_is_its_transforms_type_or_null(#[case] transform: Value, #[case] typed: Value) {
    let mut declared = json!({ "locator": { "kind": "css", "selector": "p" } });
    if transform != json!({}) {
        declared["transform"] = transform;
    }

    assert_eq!(schema(&field(declared)), typed);
}

/// A field the engine could never read is refused at the start: a CSS
/// selector it does not read, a regular expression that does not compile
/// or lacks the group named, a locale that is no BCP 47 tag, and a time
/// zone it has no rules for.
#[rstest]
#[case::css(json!({ "locator": { "kind": "css", "selector": "p[" } }), true)]
#[case::regex(json!({ "locator": { "kind": "css", "selector": "p" }, "match": { "pattern": "(" } }), true)]
#[case::group(json!({ "locator": { "kind": "css", "selector": "p" }, "match": { "pattern": "(a)", "group": 2 } }), true)]
#[case::locale(json!({ "locator": { "kind": "css", "selector": "p" }, "transform": { "kind": "number", "locale": "not a tag" } }), true)]
#[case::time_zone(json!({ "locator": { "kind": "css", "selector": "p" }, "transform": { "kind": "date_time", "locale": "de-DE", "time_zone": "Europe/Nowhere" } }), true)]
#[case::xpath_left_to_the_browser(json!({ "locator": { "kind": "xpath", "selector": "//h1" } }), false)]
#[case::whole_match(json!({ "locator": { "kind": "css", "selector": "p" }, "match": { "pattern": "a+", "group": 0 } }), false)]
fn a_field_the_engine_could_never_read_is_refused(#[case] declared: Value, #[case] refused: bool) {
    assert_eq!(check("name", &field(declared)).is_err(), refused);
}
