//! A page's fields as pure functions of the elements each locator found
//! (ADRs 0123 and 0147): the value each Value transform gives, a list's
//! values and rows' objects, what Provenance names of each, the reason a
//! value is null, and the type each field takes in a Dataset's schema.

use std::collections::BTreeMap;

use goodfetch_counted_work::{CountedWork, Row};
use goodfetch_protocol::page::read::{PageField, TransformProvenance, UnreadField};
use rstest::rstest;
use serde_json::{Value, json};

use super::{FieldElement, Fields, Found, Values, check, schema};

const BASE: &str = "http://shop.test/kites/delta";

/// What the fields read of what their locators found, with the readers
/// they build.
fn read(fields: &[(&str, Value)], found: Vec<Found>) -> Values {
    let fields: BTreeMap<String, PageField> = fields
        .iter()
        .map(|(name, declared)| ((*name).to_owned(), field(declared.clone())))
        .collect();
    Fields::of(&fields, &CountedWork::default()).read(found, BASE)
}

/// What the one value field `name` reads of `element`: its value, its
/// transforms and why it is null.
fn read_one(
    name: &str,
    declared: Value,
    element: Option<FieldElement>,
) -> (Value, Vec<TransformProvenance>, Option<UnreadField>) {
    let mut read = read(&[(name, declared)], vec![Found::One(element)]);
    (
        read.values.remove(name).unwrap(),
        read.transforms,
        read.unread.pop(),
    )
}

fn field(value: Value) -> PageField {
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
/// whatever the locale, else the text as the field's locale prints it, and
/// a `date_time` stating no offset takes its `time_zone`'s; each gives its
/// value, and Provenance names each transform with the text it read, its
/// locale and its zone.
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
#[case::a_number_printed_under_de_de(
    json!({ "locator": { "kind": "css", "selector": ".price" }, "transform": { "kind": "number", "locale": "de-DE" } }),
    element("span", "1.099,98\u{a0}€", &[]),
    json!(1099.98),
    json!([{ "field": "/name", "text": "1.099,98 €", "transform": "number", "locale": "de-DE" }]),
)]
#[case::a_date_printed_under_de_de(
    json!({ "locator": { "kind": "css", "selector": ".published" }, "transform": { "kind": "date", "locale": "de-DE" } }),
    element("p", "1. Februar 2026", &[]),
    json!("2026-02-01"),
    json!([{ "field": "/name", "text": "1. Februar 2026", "transform": "date", "locale": "de-DE" }]),
)]
#[case::a_date_time_printed_under_en_us_takes_its_time_zones_offset(
    json!({ "locator": { "kind": "css", "selector": ".at" }, "transform": { "kind": "date_time", "locale": "en-US", "time_zone": "America/New_York" } }),
    element("p", "Feb 1, 2026, 3:05 PM", &[]),
    json!("2026-02-01T15:05:00-05:00"),
    json!([{ "field": "/name", "text": "Feb 1, 2026, 3:05 PM", "transform": "date_time", "locale": "en-US", "time_zone": "America/New_York" }]),
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
    assert_eq!(
        read_one("name", declared, Some(found)),
        (value, transforms(named), None)
    );
}

/// A field its page does not give is null, never an error, and
/// Provenance names why, with the text where one was read: no element, no
/// such attribute, a `match` that did not match, a number printed in
/// another locale's form than the field's, a decimal a JSON number
/// cannot hold exactly, an impossible date, and a `date_time` stating no
/// offset with no `time_zone`, printed or not.
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
#[case::a_number_printed_in_another_locales_form(
    json!({ "locator": { "kind": "css", "selector": ".price" }, "transform": { "kind": "number", "locale": "de-DE" } }),
    Some(element("span", "1,099.98\u{a0}€", &[])),
    json!({ "field": "/name", "reason": "not_parsed", "text": "1,099.98 €" }),
)]
#[case::a_printed_date_time_with_no_zone_anywhere(
    json!({ "locator": { "kind": "css", "selector": ".at" }, "transform": { "kind": "date_time", "locale": "en-US" } }),
    Some(element("p", "Feb 1, 2026, 3:05 PM", &[])),
    json!({ "field": "/name", "reason": "no_time_zone", "text": "Feb 1, 2026, 3:05 PM" }),
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
    let (value, _, unread_field) = read_one("name", declared, found);

    assert_eq!((value, unread_field), (Value::Null, unread(reason)));
}

/// A field name is written into its JSON Pointer escaped, `~` and `/` as
/// RFC 6901 writes them.
#[test]
fn a_fields_pointer_escapes_its_name() {
    let (_, _, unread_field) = read_one(
        "a/b~c",
        json!({ "locator": { "kind": "css", "selector": "p" } }),
        None,
    );

    assert_eq!(unread_field.unwrap().field, "/a~1b~0c");
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

/// A list takes an array of its value's type; rows, an array of objects
/// holding each of their fields, every one present.
#[test]
fn a_list_and_rows_take_arrays_of_their_values() {
    let list = json!({ "list": { "locator": { "kind": "css", "selector": "li" } } });
    let rows = json!({
        "rows": { "kind": "css", "selector": "li" },
        "fields": {
            "name": { "locator": { "kind": "css", "selector": "a" } },
            "price": { "locator": { "kind": "css", "selector": ".price" }, "transform": { "kind": "number", "locale": "en-US" } },
        },
    });

    assert_eq!(
        (schema(&field(list)), schema(&field(rows))),
        (
            json!({ "type": "array", "items": { "type": ["string", "null"] } }),
            json!({
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "name": { "type": ["string", "null"] },
                        "price": { "type": ["number", "null"] },
                    },
                    "required": ["name", "price"],
                    "additionalProperties": false,
                },
            }),
        )
    );
}

/// A list reads every element its locator found, in document order, each
/// value at its own index's pointer.
#[test]
fn a_list_reads_each_element() {
    let read = read(
        &[(
            "tags",
            json!({ "list": { "locator": { "kind": "css", "selector": "li" }, "match": { "pattern": "#(\\w+)" } } }),
        )],
        vec![Found::List(vec![
            element("li", "#kites", &[]),
            element("li", "sale", &[]),
            element("li", "#delta", &[]),
        ])],
    );

    assert_eq!(
        (read.values, read.unread),
        (
            json!({ "tags": ["kites", null, "delta"] })
                .as_object()
                .unwrap()
                .clone(),
            vec![
                unread(json!({ "field": "/tags/1", "reason": "no_match", "text": "sale" }))
                    .unwrap()
            ],
        )
    );
}

/// Rows give one object per row, each holding every one of their fields;
/// a row lacking a field keeps its other fields, the missing one null with
/// its reason at the row's pointer.
#[test]
fn a_row_lacking_a_field_keeps_its_other_fields() {
    let read = read(
        &[(
            "offers",
            json!({
                "rows": { "kind": "css", "selector": "li.product" },
                "fields": {
                    "name": { "locator": { "kind": "css", "selector": "a" } },
                    "price": { "locator": { "kind": "css", "selector": ".price" }, "transform": { "kind": "number", "locale": "en-US" } },
                },
            }),
        )],
        vec![Found::Rows(vec![
            vec![
                Some(element("a", "Delta", &[])),
                Some(element("span", "49.90", &[])),
            ],
            vec![Some(element("a", "Kestrel", &[])), None],
        ])],
    );

    assert_eq!(
        (read.values, read.unread),
        (
            json!({ "offers": [
                { "name": "Delta", "price": 49.9 },
                { "name": "Kestrel", "price": null },
            ] })
            .as_object()
            .unwrap()
            .clone(),
            vec![unread(json!({ "field": "/offers/1/price", "reason": "no_element" })).unwrap()],
        )
    );
}

/// A list or rows whose locators found nothing is an empty array.
#[rstest]
#[case::a_list(json!({ "list": { "locator": { "kind": "css", "selector": "li" } } }), Found::List(Vec::new()))]
#[case::rows(
    json!({ "rows": { "kind": "css", "selector": "li" }, "fields": { "name": { "locator": { "kind": "css", "selector": "a" } } } }),
    Found::Rows(Vec::new()),
)]
fn a_list_or_rows_finding_nothing_is_empty(#[case] declared: Value, #[case] found: Found) {
    let read = read(&[("items", declared)], vec![found]);

    assert_eq!(read.values["items"], json!([]));
}

/// A field the engine could never read is refused at the start, its
/// locators aside: a regular expression that does not compile or lacks the
/// group named, a locale that is no BCP 47 tag, and a time zone it has no
/// rules for; a row's field is named under its rows.
#[rstest]
#[case::regex(json!({ "locator": { "kind": "css", "selector": "p" }, "match": { "pattern": "(" } }), Some("name"))]
#[case::group(json!({ "locator": { "kind": "css", "selector": "p" }, "match": { "pattern": "(a)", "group": 2 } }), Some("name"))]
#[case::locale(json!({ "locator": { "kind": "css", "selector": "p" }, "transform": { "kind": "number", "locale": "not a tag" } }), Some("name"))]
#[case::time_zone(json!({ "locator": { "kind": "css", "selector": "p" }, "transform": { "kind": "date_time", "locale": "de-DE", "time_zone": "Europe/Nowhere" } }), Some("name"))]
#[case::whole_match(json!({ "locator": { "kind": "css", "selector": "p" }, "match": { "pattern": "a+", "group": 0 } }), None)]
#[case::a_lists_locale(json!({ "list": { "locator": { "kind": "css", "selector": "p" }, "transform": { "kind": "date", "locale": "not a tag" } } }), Some("name"))]
#[case::a_rows_field(
    json!({ "rows": { "kind": "css", "selector": "li" }, "fields": { "price": { "locator": { "kind": "css", "selector": "p" }, "match": { "pattern": "(" } } } }),
    Some("name/price"),
)]
fn a_field_the_engine_could_never_read_is_refused(
    #[case] declared: Value,
    #[case] named: Option<&str>,
) {
    let refused = check("name", &field(declared), &CountedWork::default()).err();

    assert_eq!(
        refused.map(|sentence| sentence
            .starts_with(&format!("The field `{}`'s", named.unwrap_or_default()))),
        named.map(|_| true)
    );
}

/// A locale's forms are learned once for the engine, whichever declaration
/// asks for them: a `page.read` on each page, or a field and a row's field
/// of the same locale, builds them once (`F1.locale_form_builds`); another
/// locale's are its own.
#[test]
fn a_locales_forms_are_built_once_for_the_engine() {
    let work = CountedWork::default();
    let declared = |transform: Value| {
        BTreeMap::from([
            (
                "price".to_owned(),
                field(
                    json!({ "locator": { "kind": "css", "selector": ".price" }, "transform": transform }),
                ),
            ),
            (
                "items".to_owned(),
                field(
                    json!({ "rows": { "kind": "css", "selector": "li" }, "fields": { "price": { "locator": { "kind": "css", "selector": "p" }, "transform": transform } } }),
                ),
            ),
        ])
    };
    let german = declared(json!({ "kind": "number", "locale": "de-DE" }));

    let _ = Fields::of(&german, &work);
    let _ = Fields::of(&german, &work);
    let once = work.read(Row::PageLocaleFormBuilds);
    let _ = Fields::of(
        &declared(json!({ "kind": "number", "locale": "fr-FR" })),
        &work,
    );

    assert_eq!((once, work.read(Row::PageLocaleFormBuilds)), (1, 2));
}
