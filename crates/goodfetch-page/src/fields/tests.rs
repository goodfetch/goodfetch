//! What a browser page's read of its fields answers (ADR 0147).

use std::collections::BTreeMap;

use goodfetch_extractor::fields::{FieldElement, Found};
use goodfetch_protocol::page::read::PageField;
use rstest::rstest;
use serde_json::{Value, json};

use super::{Answer, answer_of};

fn declared(fields: Value) -> BTreeMap<String, PageField> {
    serde_json::from_value(fields).unwrap()
}

fn element(tag: &str, text: &str) -> FieldElement {
    FieldElement {
        tag: tag.to_owned(),
        text: text.to_owned(),
        attributes: vec![("class".to_owned(), "x".to_owned())],
    }
}

fn answered_element(tag: &str, text: &str) -> Value {
    json!({ "tag": tag, "text": text, "attributes": [["class", "x"]] })
}

#[test]
fn each_field_takes_what_its_kind_finds_in_the_order_of_their_names() {
    let fields = declared(json!({
        "title": { "locator": { "kind": "css", "selector": "h1" } },
        "tags": { "list": { "locator": { "kind": "css", "selector": "li" } } },
        "offers": {
            "rows": { "kind": "css", "selector": "tr" },
            "fields": {
                "price": { "locator": { "kind": "css", "selector": ".price" } },
                "seller": { "locator": { "kind": "css", "selector": ".seller" } },
            },
        },
        "absent": { "locator": { "kind": "xpath", "selector": "//aside" } },
    }));
    let answer = json!({
        "baseUrl": "https://example.test/",
        "found": [
            null,
            [[answered_element("td", "9"), null]],
            [answered_element("li", "a"), answered_element("li", "b")],
            answered_element("h1", "Title"),
        ],
    });

    assert_eq!(
        answer_of(&answer, &fields),
        Some(Answer::Found {
            base_url: "https://example.test/".to_owned(),
            found: vec![
                Found::One(None),
                Found::Rows(vec![vec![Some(element("td", "9")), None]]),
                Found::List(vec![element("li", "a"), element("li", "b")]),
                Found::One(Some(element("h1", "Title"))),
            ],
        })
    );
}

#[rstest]
#[case::a_css_selector(
    json!({ "field": "title", "kind": "css", "selector": "h1[", "message": "not valid" }),
    "The field `title`'s `h1[` is no CSS selector."
)]
#[case::an_xpath_expression(
    json!({ "field": "offers/price", "kind": "xpath", "selector": "count(//td)", "message": "The result is not a node set." }),
    "The field `offers/price`'s `count(//td)` is no XPath 1.0 expression that finds nodes: The result is not a node set."
)]
fn a_locator_the_page_refuses_is_refused_as_a_served_document_refuses_it(
    #[case] invalid: Value,
    #[case] sentence: &str,
) {
    let fields = declared(json!({ "title": { "locator": { "kind": "css", "selector": "h1[" } } }));

    assert_eq!(
        answer_of(&json!({ "invalid": invalid }), &fields),
        Some(Answer::Invalid(sentence.to_owned()))
    );
}

#[rstest]
#[case::fewer_answers_than_fields(json!({ "baseUrl": "https://example.test/", "found": [] }))]
#[case::a_list_holding_null(json!({ "baseUrl": "https://example.test/", "found": [[null]] }))]
#[case::no_base_url(json!({ "found": [[]] }))]
fn an_answer_the_page_never_gives_reads_as_none(#[case] answer: Value) {
    let fields = declared(json!({
        "tags": { "list": { "locator": { "kind": "css", "selector": "li" } } },
    }));

    assert_eq!(answer_of(&answer, &fields), None);
}
