//! The field declaration `page.read` and the code-free crawl take (ADR 0147).

use std::collections::BTreeMap;

use rstest::rstest;
use serde_json::{Value, json};

use super::{FieldFind, FieldLocator, FieldTransform, ListField, PageField, RowsField, ValueField};

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
