//! The field declaration `page.read` and the code-free crawl take (ADR 0147).

use std::collections::BTreeMap;

use rstest::rstest;
use serde_json::{Value, json};

use super::{FieldFind, FieldLocator, FieldTransform, ListField, PageField, RowsField, ValueField};

/// The readers serde derives, which `decode`'s read as on every object the
/// schema allows. Each enum bears the name of the type it stands for, which
/// serde writes in its errors.
mod derived {
    use serde::Deserialize;

    use super::{ListField, RowsField, ValueField};

    #[derive(Deserialize)]
    #[serde(untagged)]
    pub(super) enum PageField {
        Value(ValueField),
        List(ListField),
        Rows(RowsField),
    }

    #[derive(Deserialize)]
    #[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
    pub(super) enum FieldLocator {
        Css { selector: String },
        Xpath { selector: String },
    }

    impl From<PageField> for super::PageField {
        fn from(field: PageField) -> Self {
            match field {
                PageField::Value(field) => Self::Value(field),
                PageField::List(list) => Self::List(list),
                PageField::Rows(rows) => Self::Rows(rows),
            }
        }
    }

    impl From<FieldLocator> for super::FieldLocator {
        fn from(locator: FieldLocator) -> Self {
            match locator {
                FieldLocator::Css { selector } => Self::Css { selector },
                FieldLocator::Xpath { selector } => Self::Xpath { selector },
            }
        }
    }
}

fn css(selector: &str) -> FieldLocator {
    FieldLocator::Css {
        selector: selector.to_owned(),
    }
}

fn text(selector: &str) -> ValueField {
    ValueField {
        locator: css(selector),
        attribute: None,
        fallbacks: Vec::new(),
        matched: None,
        transform: FieldTransform::Text,
    }
}

#[rstest]
#[case::a_value_field(
    json!({ "locator": { "kind": "css", "selector": "h1" } }),
    PageField::Value(text("h1")),
)]
#[case::a_value_field_with_a_ladder(
    json!({
        "locator": { "kind": "css", "selector": "h1" },
        "fallbacks": [{ "kind": "css", "selector": "h2" }, { "kind": "css", "selector": ".title" }],
    }),
    PageField::Value(ValueField { fallbacks: vec![css("h2"), css(".title")], ..text("h1") }),
)]
#[case::a_list(
    json!({ "list": { "locator": { "kind": "css", "selector": "li" } } }),
    PageField::List(ListField { list: text("li") }),
)]
#[case::rows(
    json!({
        "rows": { "kind": "css", "selector": "li.product" },
        "fields": { "name": { "locator": { "kind": "css", "selector": "a" } } },
    }),
    PageField::Rows(RowsField {
        rows: css("li.product"),
        fields: [("name".to_owned(), text("a"))].into(),
    }),
)]
fn a_field_is_a_value_field_a_list_or_rows(#[case] declared: Value, #[case] field: PageField) {
    assert_eq!(
        serde_json::from_value::<PageField>(declared).unwrap(),
        field
    );
}

#[rstest]
#[case::a_value_field_with_an_unknown_key(
    json!({ "locator": { "kind": "css", "selector": "h1" }, "selector": "h1" })
)]
#[case::a_list_with_a_locator_beside_it(json!({
    "list": { "locator": { "kind": "css", "selector": "li" } },
    "locator": { "kind": "css", "selector": "li" },
}))]
#[case::rows_without_fields(json!({ "rows": { "kind": "css", "selector": "li" } }))]
#[case::rows_holding_a_list(json!({
    "rows": { "kind": "css", "selector": "li" },
    "fields": { "names": { "list": { "locator": { "kind": "css", "selector": "a" } } } },
}))]
fn a_declaration_that_is_none_of_the_three_is_refused(#[case] declared: Value) {
    assert!(serde_json::from_value::<PageField>(declared).is_err());
}

/// What a read of a field gave: the field, or its error's words without
/// where in the text it is, which differs as the derived enum reads its
/// copy and `decode` the text (the engine's errors name no position).
fn outcome<T, E: std::fmt::Display>(read: Result<T, E>) -> Result<T, String> {
    read.map_err(|error| {
        let words = error.to_string();
        match words.find(" at line ") {
            Some(at) => words[..at].to_owned(),
            None => words,
        }
    })
}

/// A field is read from its first key with no copy of its object, and
/// gives what serde's untagged enum of the three gives, field or error, on
/// the same object, members in any order, read from text or from a value
/// (issue 3324); any other value is none of the three to both.
#[rstest]
#[case::a_value_field(r#"{"locator": {"kind": "css", "selector": "h1"}}"#)]
#[case::a_value_field_led_by_its_transform(
    r#"{"transform": {"kind": "number", "locale": "fr"}, "attribute": "content", "locator": {"kind": "xpath", "selector": "//h1"}}"#
)]
#[case::a_value_field_led_by_its_match(
    r#"{"match": {"pattern": "(\\d+)"}, "fallbacks": [{"kind": "css", "selector": "h2"}], "locator": {"kind": "css", "selector": "h1"}}"#
)]
#[case::a_value_field_led_by_its_fallbacks(
    r#"{"fallbacks": [], "locator": {"kind": "css", "selector": "h1"}, "attribute": null}"#
)]
#[case::a_value_field_whose_key_is_escaped(
    "{\"\x5cu006cocator\": {\"kind\": \"css\", \"selector\": \"h1\"}}"
)]
#[case::a_list(r#"{"list": {"locator": {"kind": "css", "selector": "li"}}}"#)]
#[case::rows_led_by_their_locator(
    r#"{"rows": {"kind": "css", "selector": "li"}, "fields": {"name": {"locator": {"kind": "css", "selector": "a"}}}}"#
)]
#[case::rows_led_by_their_fields(
    r#"{"fields": {"name": {"locator": {"kind": "css", "selector": "a"}}}, "rows": {"kind": "css", "selector": "li"}}"#
)]
#[case::an_empty_object("{}")]
#[case::an_unknown_key_first(r#"{"selector": "h1", "locator": {"kind": "css", "selector": "h1"}}"#)]
#[case::an_unknown_key_last(r#"{"locator": {"kind": "css", "selector": "h1"}, "selector": "h1"}"#)]
#[case::a_locator_twice(
    r#"{"locator": {"kind": "css", "selector": "h1"}, "locator": {"kind": "css", "selector": "h2"}}"#
)]
#[case::fallbacks_without_a_locator(r#"{"fallbacks": [{"kind": "css", "selector": "h2"}]}"#)]
#[case::a_locator_of_text(r#"{"locator": "h1"}"#)]
#[case::a_locator_of_an_unknown_kind(r#"{"locator": {"kind": "regex", "selector": "h1"}}"#)]
#[case::an_attribute_of_a_number(
    r#"{"locator": {"kind": "css", "selector": "a"}, "attribute": 1}"#
)]
#[case::fallbacks_of_an_object(
    r#"{"locator": {"kind": "css", "selector": "a"}, "fallbacks": {"kind": "css", "selector": "b"}}"#
)]
#[case::a_match_group_of_text(
    r#"{"locator": {"kind": "css", "selector": "a"}, "match": {"pattern": "a", "group": "1"}}"#
)]
#[case::a_transform_of_an_unknown_kind(
    r#"{"locator": {"kind": "css", "selector": "a"}, "transform": {"kind": "money"}}"#
)]
#[case::a_list_of_text(r#"{"list": "li"}"#)]
#[case::a_list_with_a_locator_beside_it(
    r#"{"list": {"locator": {"kind": "css", "selector": "li"}}, "locator": {"kind": "css", "selector": "li"}}"#
)]
#[case::a_locator_with_a_list_beside_it(
    r#"{"locator": {"kind": "css", "selector": "li"}, "list": {"locator": {"kind": "css", "selector": "li"}}}"#
)]
#[case::rows_of_a_number(r#"{"rows": 1, "fields": {}}"#)]
#[case::rows_without_fields(r#"{"rows": {"kind": "css", "selector": "li"}}"#)]
#[case::rows_holding_a_list(
    r#"{"rows": {"kind": "css", "selector": "li"}, "fields": {"names": {"list": {"locator": {"kind": "css", "selector": "a"}}}}}"#
)]
#[case::fields_of_an_array(r#"{"fields": [], "rows": {"kind": "css", "selector": "li"}}"#)]
#[case::null("null")]
#[case::a_boolean("true")]
#[case::a_whole_number("1")]
#[case::a_negative_number("-1")]
#[case::a_decimal("1.5")]
#[case::text(r#""h1""#)]
#[case::an_empty_array("[]")]
fn a_field_reads_as_the_untagged_enum_of_the_three(#[case] declared: &str) {
    let untagged =
        |read: Result<derived::PageField, serde_json::Error>| outcome(read.map(PageField::from));
    let value: Value = serde_json::from_str(declared).unwrap();

    assert_eq!(
        outcome(serde_json::from_str::<PageField>(declared)),
        untagged(serde_json::from_str::<derived::PageField>(declared)),
        "{declared}"
    );
    assert_eq!(
        outcome(serde_json::from_value::<PageField>(value.clone())),
        untagged(serde_json::from_value::<derived::PageField>(value)),
        "{declared}"
    );
}

/// A locator reads to what serde's internally tagged enum reads from the
/// same object, value or error, which the fields above read through; a
/// value that is no object is refused by both.
#[rstest]
#[case::css(r#"{"kind": "css", "selector": "h1"}"#)]
#[case::xpath(r#"{"kind": "xpath", "selector": "//h1"}"#)]
#[case::led_by_its_selector(r#"{"selector": "h1", "kind": "css"}"#)]
#[case::whose_kind_is_escaped("{\"\x5cu006bind\": \"css\", \"selector\": \"h1\"}")]
#[case::of_a_kind_by_index(r#"{"kind": 1, "selector": "//h1"}"#)]
#[case::of_a_kind_past_the_indexes(r#"{"kind": 2, "selector": "h1"}"#)]
#[case::of_an_unknown_kind(r#"{"kind": "regex", "selector": "h1"}"#)]
#[case::of_a_kind_of_null(r#"{"kind": null, "selector": "h1"}"#)]
#[case::of_a_kind_of_an_object(r#"{"kind": {}, "selector": "h1"}"#)]
#[case::without_a_kind(r#"{"selector": "h1"}"#)]
#[case::without_a_selector(r#"{"kind": "css"}"#)]
#[case::empty("{}")]
#[case::of_a_kind_twice(r#"{"kind": "css", "selector": "h1", "kind": "css"}"#)]
#[case::of_a_selector_twice(r#"{"kind": "css", "selector": "h1", "selector": "h2"}"#)]
#[case::of_a_selector_twice_the_second_a_number(
    r#"{"kind": "css", "selector": "h1", "selector": 1}"#
)]
#[case::of_an_unknown_key(r#"{"kind": "css", "selector": "h1", "name": "h1"}"#)]
#[case::of_an_unknown_key_without_a_kind(r#"{"name": "h1", "selector": "h1"}"#)]
#[case::of_an_unknown_key_then_a_kind_twice(
    r#"{"kind": "css", "name": "h1", "kind": "css", "selector": "h1"}"#
)]
#[case::of_an_unknown_key_then_an_unknown_kind(
    r#"{"name": "h1", "kind": "regex", "selector": "h1"}"#
)]
#[case::of_an_unknown_key_then_a_selector_of_a_number(
    r#"{"kind": "css", "name": "h1", "selector": 1}"#
)]
#[case::of_a_selector_of_a_number_then_an_unknown_key(
    r#"{"kind": "css", "selector": 1, "name": "h1"}"#
)]
#[case::of_a_selector_of_a_number_then_a_kind_twice(
    r#"{"kind": "css", "selector": 1, "kind": "xpath"}"#
)]
#[case::of_a_selector_of_a_number_without_a_kind(r#"{"selector": 1}"#)]
#[case::of_a_selector_of_a_negative_number(r#"{"kind": "css", "selector": -1}"#)]
#[case::of_a_selector_of_a_decimal(r#"{"kind": "css", "selector": 1.5}"#)]
#[case::of_a_selector_of_a_boolean(r#"{"kind": "css", "selector": false}"#)]
#[case::of_a_selector_of_null(r#"{"kind": "css", "selector": null}"#)]
#[case::of_a_selector_of_an_object(r#"{"kind": "css", "selector": {"kind": "css"}, "name": 1}"#)]
#[case::of_a_selector_of_an_array(r#"{"kind": "css", "selector": ["h1", {}], "name": 1}"#)]
#[case::text(r#""h1""#)]
#[case::a_number("1")]
#[case::null("null")]
fn a_locator_reads_as_the_internally_tagged_enum(#[case] declared: &str) {
    let tagged = |read: Result<derived::FieldLocator, serde_json::Error>| {
        outcome(read.map(FieldLocator::from))
    };
    let value: Value = serde_json::from_str(declared).unwrap();

    assert_eq!(
        outcome(serde_json::from_str::<FieldLocator>(declared)),
        tagged(serde_json::from_str::<derived::FieldLocator>(declared)),
        "{declared}"
    );
    assert_eq!(
        outcome(serde_json::from_value::<FieldLocator>(value.clone())),
        tagged(serde_json::from_value::<derived::FieldLocator>(value)),
        "{declared}"
    );
}

/// What the schema refuses and only serde's copy of a field let through is
/// refused, read from text or from a value: a field or a locator given as
/// an array, and a `kind` given by its index (issue 3324).
#[rstest]
#[case::a_value_field_as_an_array(r#"[{"kind": "css", "selector": "h1"}]"#)]
#[case::a_list_as_an_array(r#"[[{"kind": "css", "selector": "li"}]]"#)]
#[case::a_locator_as_an_array(r#"{"locator": ["css", "h1"]}"#)]
#[case::an_empty_locator_array(r#"{"locator": []}"#)]
#[case::fallbacks_of_arrays(
    r#"{"locator": {"kind": "css", "selector": "h1"}, "fallbacks": [["css", "h2"]]}"#
)]
#[case::rows_as_an_array(r#"{"rows": ["css", "li"], "fields": {}}"#)]
#[case::a_locator_kind_by_index(r#"{"locator": {"kind": 1, "selector": "//h1"}}"#)]
#[case::a_rows_kind_by_index(r#"{"rows": {"kind": 0, "selector": "li"}, "fields": {}}"#)]
#[case::a_transform_kind_by_index(
    r#"{"locator": {"kind": "css", "selector": "a"}, "transform": {"kind": 1, "locale": "fr"}}"#
)]
fn a_field_the_schema_refuses_is_refused(#[case] declared: &str) {
    let value: Value = serde_json::from_str(declared).unwrap();

    assert!(
        serde_json::from_str::<PageField>(declared).is_err(),
        "{declared}"
    );
    assert!(
        serde_json::from_value::<PageField>(value).is_err(),
        "{declared}"
    );
}

/// Each kind of locator reads back as it is written.
#[test]
fn each_locator_kind_reads_back_as_written() {
    let selector = || "h1".to_owned();
    for locator in [
        FieldLocator::Css {
            selector: selector(),
        },
        FieldLocator::Xpath {
            selector: selector(),
        },
    ] {
        // A kind added to `FieldLocator` fails to compile here until it is
        // listed above, and its read in `decode` is tested with it.
        match &locator {
            FieldLocator::Css { .. } | FieldLocator::Xpath { .. } => {}
        }
        let written = serde_json::to_string(&locator).unwrap();

        assert_eq!(
            serde_json::from_str::<FieldLocator>(&written).unwrap(),
            locator
        );
    }
}

/// A field with no `fallbacks` writes none, so its declaration reads as it
/// did before ladders (issue 1972).
#[test]
fn a_field_without_fallbacks_writes_none() {
    assert_eq!(
        serde_json::to_value(text("h1")).unwrap(),
        json!({ "locator": { "kind": "css", "selector": "h1" }, "transform": { "kind": "text" } }),
    );
}

/// A field's read finds once per rung of its ladder, its locator first, a
/// field with no `fallbacks` once, and rows once, holding each rung of each
/// of their fields, by name then rung (issue 1972).
#[test]
fn a_fields_read_finds_once_per_rung_and_rows_once() {
    let fields = BTreeMap::from([
        ("heading".to_owned(), PageField::Value(text("h1"))),
        (
            "names".to_owned(),
            PageField::List(ListField {
                list: ValueField {
                    fallbacks: vec![css("li a")],
                    ..text("li b")
                },
            }),
        ),
        (
            "offers".to_owned(),
            PageField::Rows(RowsField {
                rows: css("li"),
                fields: BTreeMap::from([
                    (
                        "price".to_owned(),
                        ValueField {
                            fallbacks: vec![css(".price")],
                            ..text(".cost")
                        },
                    ),
                    ("name".to_owned(), text("a")),
                ]),
            }),
        ),
        (
            "title".to_owned(),
            PageField::Value(ValueField {
                fallbacks: vec![css("h2"), css(".title")],
                ..text("h3")
            }),
        ),
    ]);
    let (h1, li_b, li_a, li, a, cost, price, h3, h2, title) = (
        css("h1"),
        css("li b"),
        css("li a"),
        css("li"),
        css("a"),
        css(".cost"),
        css(".price"),
        css("h3"),
        css("h2"),
        css(".title"),
    );

    let finds: Vec<FieldFind<'_>> = fields.values().flat_map(PageField::finds).collect();

    assert_eq!(
        finds,
        vec![
            FieldFind::One(&h1),
            FieldFind::List(&li_b),
            FieldFind::List(&li_a),
            FieldFind::Rows {
                rows: &li,
                fields: vec![("name", &a), ("price", &cost), ("price", &price)],
            },
            FieldFind::One(&h3),
            FieldFind::One(&h2),
            FieldFind::One(&title),
        ]
    );
}
