//! The field declaration `page.read` and the code-free crawl take (ADR 0147).

use rstest::rstest;
use serde_json::{Value, json};

use super::{FieldLocator, FieldTransform, ListField, PageField, RowsField, ValueField};

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
/// did before ladders (issue 1972); its rungs are its locator alone.
#[test]
fn a_field_without_fallbacks_writes_none_and_its_rungs_are_its_locator() {
    let field = text("h1");
    let laddered = ValueField {
        fallbacks: vec![css("h2")],
        ..text("h1")
    };

    assert_eq!(
        (
            serde_json::to_value(&field).unwrap(),
            field.rungs().collect::<Vec<_>>(),
            laddered.rungs().collect::<Vec<_>>(),
        ),
        (
            json!({ "locator": { "kind": "css", "selector": "h1" }, "transform": { "kind": "text" } }),
            vec![&css("h1")],
            vec![&css("h1"), &css("h2")],
        )
    );
}
