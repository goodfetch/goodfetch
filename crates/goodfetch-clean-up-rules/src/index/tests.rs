//! How a selector's text is keyed, and which rules an element is tested
//! against.
#![expect(
    clippy::disallowed_methods,
    reason = "ADR 0138: a test's own parse, on no page's path"
)]

use goodfetch_counted_work::{CountedWork, Row};
use rstest::rstest;

use super::{Index, Key, Operator, Value, keys};

fn id(name: &str) -> Key {
    Key::Id(name.to_owned())
}

fn class(name: &str) -> Key {
    Key::Class(name.to_owned())
}

fn attribute(name: &str) -> Key {
    Key::Attribute(name.to_owned(), None)
}

fn valued(name: &str, operator: Operator, text: &str) -> Key {
    Key::Attribute(
        name.to_owned(),
        Some(Value {
            operator,
            text: text.to_owned(),
        }),
    )
}

fn tag(name: &str) -> Key {
    Key::Tag(name.to_owned())
}

#[rstest]
#[case::a_class(".ad-slot", vec![class("ad-slot")])]
#[case::an_id("#ad-top", vec![id("ad-top")])]
#[case::a_tag("amp-ad", vec![tag("amp-ad")])]
#[case::an_attribute("[data-ad]", vec![attribute("data-ad")])]
#[case::an_attribute_equal_to(r#"[title="Ad"]"#, vec![valued("title", Operator::Equals, "ad")])]
#[case::an_attribute_with_a_prefix(r#"[href^="http://ads.example/"]"#, vec![valued("href", Operator::Prefix, "http://ads.example/")])]
#[case::an_attribute_with_a_suffix("[src$='.gif']", vec![valued("src", Operator::Suffix, ".gif")])]
#[case::an_attribute_holding_a_substring(r#"[href*="&maxads="]"#, vec![valued("href", Operator::Contains, "&maxads=")])]
#[case::an_attribute_holding_a_word("[class~=promo]", vec![valued("class", Operator::Contains, "promo")])]
#[case::an_attribute_dash_match("[lang|=en]", vec![valued("lang", Operator::Prefix, "en")])]
#[case::an_attribute_with_a_flag(r#"[href^="//Go.Example/" i]"#, vec![valued("href", Operator::Prefix, "//go.example/")])]
#[case::an_escaped_attribute_value(r#"[title="a\"b"]"#, vec![valued("title", Operator::Equals, "a\"b")])]
#[case::a_valued_attribute_before_a_bare_one(r#"a[href][target="_blank"]"#, vec![valued("target", Operator::Equals, "_blank")])]
#[case::the_first_valued_attribute(r#"a[href*="x"][href*="y"]"#, vec![valued("href", Operator::Contains, "x")])]
#[case::an_attribute_with_no_value_read("[title=]", vec![attribute("title")])]
#[case::the_id_before_the_class("div.card#top", vec![id("top")])]
#[case::the_class_before_the_attribute("a.sponsored[href]", vec![class("sponsored")])]
#[case::the_attribute_before_the_tag("a[href]", vec![attribute("href")])]
#[case::a_valued_attribute_before_the_tag(r#"a[href^="http"]"#, vec![valued("href", Operator::Prefix, "http")])]
#[case::the_first_class_of_two(".a.b", vec![class("a")])]
#[case::the_rightmost_compound_of_a_descendant("#page .card span", vec![tag("span")])]
#[case::the_rightmost_compound_of_a_child(".card > .label", vec![class("label")])]
#[case::the_rightmost_compound_of_a_sibling("h2 + .ad", vec![class("ad")])]
#[case::the_rightmost_compound_of_a_later_sibling("h2~p", vec![tag("p")])]
#[case::each_member_of_a_list(".a, #b > span , [c]", vec![class("a"), tag("span"), attribute("c")])]
#[case::a_pseudo_class_narrows(".card:not(.kept):first-child", vec![class("card")])]
#[case::a_pseudo_classes_arguments_key_nothing("div:has(> .ad, #x)", vec![tag("div")])]
#[case::a_pseudo_class_alone(":not(.kept)", vec![Key::Universal])]
#[case::the_universal_selector("*", vec![Key::Universal])]
#[case::the_universal_selector_with_a_class("*.ad", vec![class("ad")])]
#[case::folded_to_lower_case("DIV.Ad-Slot", vec![class("ad-slot")])]
#[case::a_folded_tag("ARTICLE", vec![tag("article")])]
#[case::an_escaped_class(r".ad\:slot", vec![class("ad:slot")])]
#[case::an_escaped_id(r"#\31 st", vec![id("1st")])]
#[case::a_namespaced_tag("svg|a", vec![Key::Universal])]
#[case::any_namespaces_tag("*|a.ad", vec![Key::Universal])]
#[case::a_namespaced_attribute("[xlink|href]", vec![Key::Universal])]
#[case::any_namespaces_attribute("[*|href]", vec![Key::Universal])]
#[case::a_pseudo_element("div::before", vec![Key::Universal])]
#[case::a_pseudo_element_spelled_as_a_class("p:first-line", vec![Key::Universal])]
#[case::a_member_keyed_beside_one_that_is_not("svg|a, .ad", vec![Key::Universal, class("ad")])]
#[case::a_dot_with_no_class(". ad", vec![Key::Universal])]
#[case::a_number("12", vec![Key::Universal])]
#[case::nothing_after_a_combinator(".card >", vec![Key::Universal])]
#[case::nothing("", vec![Key::Universal])]
fn a_selector_is_keyed_by_its_rightmost_compound(
    #[case] selector: &str,
    #[case] expected: Vec<Key>,
) {
    assert_eq!(keys(selector), expected);
}

/// The rules, among `selectors`, that the element `element` is tested
/// against.
fn candidates_of(selectors: &[&str], element: &str) -> Vec<String> {
    let index = Index::of(selectors.iter().copied());
    let work = CountedWork::default();
    let document = work.parse_html(Row::PageParses, &format!("<body>{element}</body>"));
    let body = document
        .root()
        .descendants_it()
        .find(|node| node.node_name().is_some_and(|name| &*name == "body"))
        .expect("the page has a body");
    let node = body
        .first_element_child()
        .expect("the body holds the element");
    let mut candidates = Vec::new();
    index.candidates(&node, &mut candidates);
    candidates
        .into_iter()
        .map(|at| selectors[at].to_owned())
        .collect()
}

#[rstest]
#[case::an_equal_value(r#"[href="/sites/16/2"]"#, true)]
#[case::an_equal_value_of_another_case(r#"[href="/SITES/16/2"]"#, true)]
#[case::another_value(r#"[href="/sites/16/3"]"#, false)]
#[case::a_prefix(r#"a[href^="/sites/"]"#, true)]
#[case::another_prefix(r#"a[href^="https://ads.example/"]"#, false)]
#[case::a_suffix(r#"[href$="16/2"]"#, true)]
#[case::another_suffix(r#"[href$=".gif"]"#, false)]
#[case::a_substring(r#"[href*="es/1"]"#, true)]
#[case::another_substring(r#"[href*="&maxads="]"#, false)]
#[case::a_word("[href~=Sites]", true)]
#[case::a_dash_match("[href|='/sites']", true)]
#[case::another_attributes_value(r#"[title="/sites/16/2"]"#, false)]
#[case::the_attribute_alone("[href]", true)]
fn an_element_is_tested_against_the_attribute_rules_its_value_could_match(
    #[case] selector: &str,
    #[case] tested: bool,
) {
    let candidates = candidates_of(&[selector], r#"<a href="/Sites/16/2">A page</a>"#);

    assert_eq!(
        candidates,
        if tested {
            vec![selector.to_owned()]
        } else {
            vec![]
        }
    );
}
