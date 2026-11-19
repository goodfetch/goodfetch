//! What a browser page's read of its fields answers (ADR 0147).

use std::collections::BTreeMap;

use goodfetch_extractor::fields::{FieldElement, Found};
use goodfetch_protocol::page::read::PageField;
use rstest::rstest;
use serde_json::{Value, json};

use super::{Answer, answer_of, argument};

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

/// A field with no `fallbacks` sends its one locator, as before; a ladder
/// (issue 1972) sends one entry per rung under the field's name, a row's
/// field one per rung under its rows, so the page function reads every
/// rung as it reads one locator.
#[test]
fn a_ladder_sends_one_entry_per_rung_and_a_field_without_one_its_locator() {
    let css = |selector: &str| json!({ "kind": "css", "selector": selector });
    let fields = declared(json!({
        "names": { "list": { "locator": css("li b"), "fallbacks": [css("li a")] } },
        "offers": {
            "rows": css("tr"),
            "fields": {
                "price": { "locator": css(".cost"), "fallbacks": [css(".price")] },
                "seller": { "locator": css(".seller") },
            },
        },
        "title": { "locator": css("h1") },
    }));

    assert_eq!(
        argument(&fields),
        json!({ "fields": [
            { "name": "names", "kind": "list", "locator": css("li b") },
            { "name": "names", "kind": "list", "locator": css("li a") },
            { "name": "offers", "kind": "rows", "locator": css("tr"), "fields": [
                { "name": "price", "locator": css(".cost") },
                { "name": "price", "locator": css(".price") },
                { "name": "seller", "locator": css(".seller") },
            ] },
            { "name": "title", "kind": "one", "locator": css("h1") },
        ] })
    );
}

/// The page's answer to a ladder holds one find per rung, which the
/// extractor reads; an answer holding one find for a field of two rungs is
/// one the page never gives.
#[test]
fn a_ladders_answer_holds_one_find_per_rung() {
    let css = |selector: &str| json!({ "kind": "css", "selector": selector });
    let fields = declared(json!({
        "title": { "locator": css("h2"), "fallbacks": [css("h1")] },
    }));
    let answer = |found: Value| json!({ "baseUrl": "https://example.test/", "found": found });

    assert_eq!(
        (
            answer_of(
                &answer(json!([null, answered_element("h1", "Title")])),
                &fields
            ),
            answer_of(&answer(json!([answered_element("h1", "Title")])), &fields),
        ),
        (
            Some(Answer::Found {
                base_url: "https://example.test/".to_owned(),
                found: vec![Found::One(None), Found::One(Some(element("h1", "Title")))],
            }),
            None,
        )
    );
}
