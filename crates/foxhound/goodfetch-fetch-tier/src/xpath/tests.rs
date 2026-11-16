#![expect(
    clippy::disallowed_types,
    reason = "ADR 0140: a test's own parse, on no Shared path"
)]
use proptest::prelude::*;
use rstest::rstest;

use super::{Document, Expression};

/// A document whose elements each carry an `id`, so a test names the
/// element an expression finds first. Expected answers are what Chrome's
/// `document.evaluate(…, FIRST_ORDERED_NODE_TYPE)` answers for the same
/// markup.
const PAGE: &str = r##"<!DOCTYPE html><html id="html" lang="de"><head id="head"><title id="title">Katalog</title></head>
<body id="body">
<ul id="list"><li id="one" class="item a">Eins</li><li id="two" class="item">Zwei <b id="bold">fett</b></li><li id="three" class="item last" data-n="3">Drei</li></ul>
<p id="price" data-price=" 12.50 ">Preis: 12,50</p>
<p id="spaced">  viel   Platz  </p>
<!-- a remark --><div id="after-remark"></div>
<template id="template"><p id="inside-template">x</p></template>
<svg id="svg" viewBox="0 0 1 1"><rect id="rect"/><a id="svg-link" xlink:href="#x"/></svg>
<section id="section" xmlns:x="urn:x"><p id="german">Hallo</p></section>
<math id="math" xml:lang="en-GB"><mi id="mi">x</mi></math>
</body></html>"##;

/// The `id` of the first element in document order `expression` finds in
/// [`PAGE`].
fn first(expression: &str) -> Option<String> {
    let parsed = Expression::parse(expression).expect("the expression is read");
    let document = dom_query::Document::from(PAGE);
    let id = Document::of(&document).first_element(&parsed, None)?;
    document.tree.get(&id)?.attr("id").map(|id| id.to_string())
}

/// Each axis reaches what `XPath` 1.0 says it does, a reverse axis's
/// positions counting from the nearest node; a step's result is in
/// document order whatever the axis.
#[rstest]
#[case::child("/html/body/ul/li", Some("one"))]
#[case::descendant("//b", Some("bold"))]
#[case::descendant_or_self("//li[@id='two']/descendant-or-self::*", Some("two"))]
#[case::parent("//b/..", Some("two"))]
#[case::ancestor_nearest_first("//b/ancestor::*[1]", Some("two"))]
#[case::ancestor_or_self("//b/ancestor-or-self::ul", Some("list"))]
#[case::following_sibling("//li[1]/following-sibling::li", Some("two"))]
#[case::preceding_sibling_nearest_first("//li[3]/preceding-sibling::li[1]", Some("two"))]
#[case::following("//li[@id='two']/following::*", Some("three"))]
#[case::preceding_excludes_ancestors("//b/preceding::*[1]", Some("one"))]
#[case::attribute_then_parent("//@data-n/..", Some("three"))]
#[case::self_axis("//b/self::b", Some("bold"))]
#[case::namespace_axis_is_empty("//section/namespace::*/..", None)]
#[case::position("//li[position() = 2]", Some("two"))]
#[case::last("//li[last()]", Some("three"))]
#[case::nested_predicates("//li[@class][2]", Some("two"))]
#[case::filter_in_document_order("(//li | //p)[4]", Some("price"))]
#[case::path_from_a_filter("(//ul)[1]/li[2]/b", Some("bold"))]
fn each_axis_and_predicate_reaches_what_xpath_says(
    #[case] expression: &str,
    #[case] expected: Option<&str>,
) {
    assert_eq!(first(expression).as_deref(), expected, "{expression}");
}

/// As in a browser's HTML document, a name test with no prefix finds an
/// HTML element or an HTML element's attribute in any letter case, finds
/// no element of another namespace, and an SVG attribute only as written;
/// `*` finds every element; a `<template>`'s contents are no children of
/// it; `xmlns` attributes are on no axis.
#[rstest]
#[case::upper_case_element("//LI", Some("one"))]
#[case::upper_case_attribute("//li[@DATA-N]", Some("three"))]
#[case::svg_element_by_name("//rect", None)]
#[case::svg_element_by_star("//*[@id = 'svg']/*", Some("rect"))]
#[case::svg_attribute_as_written("//*[@viewBox]", Some("svg"))]
#[case::svg_attribute_in_another_case("//*[@viewbox]", None)]
#[case::xlink_attribute_has_a_namespace("//*[@href]", None)]
#[case::xlink_attribute_by_name("//*[@*[name() = 'xlink:href']]", Some("svg-link"))]
#[case::template_contents_are_no_children("//template/p", None)]
#[case::template_contents_are_not_in_the_document("//*[@id='inside-template']", None)]
#[case::xmlns_attribute_on_an_html_element("//*[@*[local-name() = 'xmlns:x']]", Some("section"))]
#[case::doctype_is_a_node_only_node_finds("/node()[1]/self::*", None)]
#[case::comment("//comment()/following-sibling::div", Some("after-remark"))]
#[case::text("//text()[. = 'Drei']/..", Some("three"))]
fn names_are_matched_as_a_browser_matches_them_in_html(
    #[case] expression: &str,
    #[case] expected: Option<&str>,
) {
    assert_eq!(first(expression).as_deref(), expected, "{expression}");
}

/// Comparisons follow §3.4: a node-set compares by any of its nodes, a
/// number by `number()`, `!=` with NaN holds, and relational operators
/// compare numbers.
#[rstest]
#[case::string_equal("//li[. = 'Eins']", Some("one"))]
#[case::node_set_against_number("//p[@data-price = 12.5]", Some("price"))]
#[case::node_set_against_node_set("//li[@id = //b/../@id]", Some("two"))]
#[case::not_equal_any_node("//ul[li != 'Eins']", Some("list"))]
#[case::node_set_against_boolean("//li[@data-n = true()]", Some("three"))]
#[case::empty_node_set_against_false("//li[@nothing = false()]", Some("one"))]
#[case::nan_differs("//li[number(.) != number(.)]", Some("one"))]
#[case::nan_equals_nothing("//li[number(.) = number(.)]", None)]
#[case::relational("//li[@data-n >= 3]", Some("three"))]
#[case::relational_on_strings("//li['10' > '9']", Some("one"))]
#[case::arithmetic("//li[position() = 7 mod 4 - 1 * 1]", Some("two"))]
#[case::division("//li[1 div 0 > 10000]", Some("one"))]
#[case::negation("//li[-(-3) = position()]", Some("three"))]
#[case::or_and("//li[@id = 'x' or @id = 'two' and true()]", Some("two"))]
fn comparisons_and_arithmetic_follow_xpath(
    #[case] expression: &str,
    #[case] expected: Option<&str>,
) {
    assert_eq!(first(expression).as_deref(), expected, "{expression}");
}

/// The core function library answers as §4 says.
#[rstest]
#[case::count("//ul[count(li) = 3]", Some("list"))]
#[case::id("id('three two')", Some("two"))]
#[case::id_of_nodes("id(//li[1]/@id)", Some("one"))]
#[case::local_name("//*[local-name() = 'mi']", Some("mi"))]
#[case::namespace_uri_of_html(
    "//li[namespace-uri() = 'http://www.w3.org/1999/xhtml']",
    Some("one")
)]
#[case::name_of_html_is_lower_case("//*[name() = 'li']", Some("one"))]
#[case::string_value("//li[string() = 'Zwei fett']", Some("two"))]
#[case::concat("//li[concat('Ei', 'n', 's') = .]", Some("one"))]
#[case::starts_with("//li[starts-with(., 'Zw')]", Some("two"))]
#[case::contains("//li[contains(@class, 'last')]", Some("three"))]
#[case::substring_before("//p[substring-before(., ':') = 'Preis']", Some("price"))]
#[case::substring_after("//p[substring-after(., ': ') = '12,50']", Some("price"))]
#[case::substring("//li[substring('12345', 1.5, 2.6) = '234']", Some("one"))]
#[case::substring_to_the_end("//li[substring('12345', 0) = '12345']", Some("one"))]
#[case::substring_nan("//li[substring('12345', 0 div 0, 3) = '']", Some("one"))]
#[case::string_length("//li[string-length() = 4]", Some("one"))]
#[case::string_length_of_characters("//li[string-length('über') = 4]", Some("one"))]
#[case::normalize_space("//p[normalize-space() = 'viel Platz']", Some("spaced"))]
#[case::translate("//li[translate(., 'EINS', 'eins') = 'eins']", Some("one"))]
#[case::translate_removes("//li[translate('abc', 'b', '') = 'ac']", Some("one"))]
#[case::boolean_not("//li[not(@data-n)][2]", Some("two"))]
#[case::number_of_text("//p[number(@data-price) = 12.5]", Some("price"))]
#[case::number_refuses_a_plus("//li[string(number('+1')) = 'NaN']", Some("one"))]
#[case::sum("//ul[sum(li/@data-n) = 3]", Some("list"))]
#[case::floor_ceiling("//li[floor(2.5) = 2 and ceiling(2.5) = 3]", Some("one"))]
#[case::round_half_up("//li[round(2.5) = 3 and round(-2.5) = -2]", Some("one"))]
#[case::number_shown_as_integer("//li[string(3 * 1) = '3']", Some("one"))]
#[case::number_shown_as_decimal("//li[string(1 div 4) = '0.25']", Some("one"))]
#[case::infinity_shown("//li[string(-1 div 0) = '-Infinity']", Some("one"))]
#[case::lang_of_foreign_content("//*[local-name() = 'mi'][lang('en')]", Some("mi"))]
#[case::lang_of_another_language("//*[local-name() = 'mi'][lang('de')]", None)]
#[case::lang_ignores_html_lang("//li[lang('de')]", None)]
fn core_functions_answer_as_xpath_says(#[case] expression: &str, #[case] expected: Option<&str>) {
    assert_eq!(first(expression).as_deref(), expected, "{expression}");
}

/// Expressions nest at most 100 levels deep, so a deeper one is refused
/// rather than running out of stack, and a long flat one is read.
#[test]
fn a_deeply_nested_expression_is_refused_and_a_long_flat_one_is_read() {
    let deep = format!("//li[{}1{}]", "(".repeat(200), ")".repeat(200));
    let long = format!("//li[{}]", vec!["1"; 10_000].join(" + "));

    assert!(Expression::parse(&deep).is_err());
    assert!(Expression::parse(&long).is_ok());
}

/// The tokens a generated expression is made of: enough of `XPath`'s
/// grammar that many sequences read as expressions.
const TOKENS: [&str; 40] = [
    "/",
    "//",
    "li",
    "*",
    "@",
    "id",
    "[",
    "]",
    "(",
    ")",
    "|",
    ".",
    "..",
    "::",
    "child",
    "ancestor",
    "following-sibling",
    "text()",
    "node()",
    "comment()",
    "1",
    "2.5",
    "'Eins'",
    "=",
    "!=",
    "<",
    ">=",
    "+",
    "-",
    "div",
    "mod",
    "and",
    "or",
    "count",
    "position()",
    "last()",
    "contains",
    ",",
    "not",
    "$x",
];

/// Every element an expression finds, from the root or from an element as
/// its context node, in document order, as Chrome's
/// `document.evaluate(…, row, null, ORDERED_NODE_SNAPSHOT_TYPE)` holds
/// them; a node that is no element is left out.
#[rstest]
#[case::every_item_from_the_root("//li", None, &["one", "two", "three"])]
#[case::relative_to_an_element("li[@class]", Some("list"), &["one", "two", "three"])]
#[case::a_rows_own_descendant("b", Some("two"), &["bold"])]
#[case::none_under_another_row("b", Some("one"), &[])]
#[case::text_is_no_element("text()", Some("one"), &[])]
fn every_element_an_expression_finds_from_its_context_node(
    #[case] expression: &str,
    #[case] from: Option<&str>,
    #[case] expected: &[&str],
) {
    let parsed = Expression::parse(expression).expect("the expression is read");
    let document = dom_query::Document::from(PAGE);
    let from = from.map(|id| {
        document
            .root()
            .descendants()
            .into_iter()
            .find(|node| node.attr("id").as_deref() == Some(id))
            .expect("the element is in the page")
            .id
    });
    let found: Vec<String> = Document::of(&document)
        .elements(&parsed, from)
        .iter()
        .filter_map(|id| document.tree.get(id)?.attr("id").map(|id| id.to_string()))
        .collect();

    assert_eq!(found, expected);
}

proptest! {
    /// No text, however malformed, makes reading it or evaluating what it
    /// reads panic.
    #[test]
    fn reading_and_evaluating_any_expression_never_panics(
        tokens in proptest::collection::vec(proptest::sample::select(TOKENS.as_slice()), 0..24),
        spaced in any::<bool>(),
    ) {
        let text = tokens.join(if spaced { " " } else { "" });
        let document = dom_query::Document::from(PAGE);
        if let Ok(expression) = Expression::parse(&text) {
            Document::of(&document).elements(&expression, None);
        }
    }

    /// No text of arbitrary characters makes reading it panic.
    #[test]
    fn reading_any_text_never_panics(text in any::<String>()) {
        drop(Expression::parse(&text));
    }
}
