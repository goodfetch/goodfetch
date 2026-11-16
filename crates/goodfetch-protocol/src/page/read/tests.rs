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
        matched: None,
        transform: FieldTransform::Text,
    }
}

#[rstest]
#[case::a_value_field(
    json!({ "locator": { "kind": "css", "selector": "h1" } }),
    PageField::Value(text("h1")),
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
