//! How a selector's text is keyed.

use rstest::rstest;

use super::{Key, keys};

fn id(name: &str) -> Key {
    Key::Id(name.to_owned())
}

fn class(name: &str) -> Key {
    Key::Class(name.to_owned())
}

fn attribute(name: &str) -> Key {
    Key::Attribute(name.to_owned())
}

fn tag(name: &str) -> Key {
    Key::Tag(name.to_owned())
}

#[rstest]
#[case::a_class(".ad-slot", vec![class("ad-slot")])]
#[case::an_id("#ad-top", vec![id("ad-top")])]
#[case::a_tag("amp-ad", vec![tag("amp-ad")])]
#[case::an_attribute("[data-ad]", vec![attribute("data-ad")])]
#[case::an_attribute_with_a_value(r#"[href^="http://ads.example/"]"#, vec![attribute("href")])]
#[case::an_attribute_dash_match("[lang|=en]", vec![attribute("lang")])]
#[case::the_id_before_the_class("div.card#top", vec![id("top")])]
#[case::the_class_before_the_attribute("a.sponsored[href]", vec![class("sponsored")])]
#[case::the_attribute_before_the_tag("a[href]", vec![attribute("href")])]
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
