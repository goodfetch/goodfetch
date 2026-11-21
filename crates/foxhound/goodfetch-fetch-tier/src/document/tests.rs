#![expect(
    clippy::disallowed_methods,
    reason = "ADR 0138: a test's own parse, on no page's path"
)]
use goodfetch_frontier::trap_links::{Seen, Sight};
use rstest::rstest;

use goodfetch_extractor::fields::{FieldElement, Found};
use goodfetch_protocol::page::read::{FieldLocator, PageField};
use serde_json::json;

use super::{
    LocatorInvalid, Matched, SelectorInvalid, check_selector, check_xpath, findings,
    first_elements_in, found, locating, matched, value_locators,
};
use goodfetch_counted_work::Row;

/// Counts no test of this file reads.
fn no_counts() -> goodfetch_counted_work::CountedWork {
    goodfetch_counted_work::CountedWork::default()
}

/// The tree of `html`.
fn tree(html: &str) -> goodfetch_counted_work::ParsedHtml {
    no_counts().parse_html(goodfetch_counted_work::Row::PageParses, html)
}

/// The value page's article, as a served document holds it.
const ARTICLE: &str = r#"<!doctype html><html><body>
<nav><h1>Shop</h1></nav>
<article><h1 class="title">Delta-Drachen</h1>
<p class="sku">Artikelnummer: <b>DK-0042</b></p>
<!-- price below --><span class="price" data-value="1099.98">1.099,98 €</span>
<a class="manual" href="manuals/delta.pdf">Handbuch</a></article>
</body></html>"#;

/// The first element each of `locators` finds in `document`, as a value
/// field's locator reads it.
fn first_elements(
    document: &goodfetch_counted_work::ParsedHtml,
    locators: &[FieldLocator],
    work: &goodfetch_counted_work::CountedWork,
) -> Result<Vec<Option<FieldElement>>, LocatorInvalid> {
    let locating = locators
        .iter()
        .map(|locator| locating(locator, work))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(first_elements_in(document, &locating))
}

fn css(selector: &str) -> FieldLocator {
    FieldLocator::Css {
        selector: selector.to_owned(),
    }
}

fn xpath(selector: &str) -> FieldLocator {
    FieldLocator::Xpath {
        selector: selector.to_owned(),
    }
}

fn field(tag: &str, text: &str, attributes: &[(&str, &str)]) -> FieldElement {
    FieldElement {
        tag: tag.to_owned(),
        text: text.to_owned(),
        attributes: attributes
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect(),
    }
}

fn element(href: Option<&str>, text: &str, attributes: &[(&str, &str)]) -> Matched {
    Matched {
        href: href.map(str::to_owned),
        text: text.to_owned(),
        attributes: attributes
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect(),
        sight: Sight::Markup(Seen::default()),
    }
}

/// A locator matches its elements in document order, each with its `href`
/// as written, entities decoded, its text and its attributes; one with no
/// `href` is matched too.
///
/// Covers: story:6-232
#[test]
fn a_locator_matches_each_element_with_its_href_text_and_attributes() {
    let html = r#"<!doctype html><html><head><base href="/shelf/"></head><body>
<nav><a href="/home">Home</a></nav>
<ul class="books">
<li><a class="book" href="book/1?a=1&amp;b=2" data-id="B1">First <em>book</em></a></li>
<li><a class="book" name="anchor">No link</a></li>
<li><a class="book" href=" mailto:shop@links.test ">Mail</a></li>
</ul></body></html>"#;

    assert_eq!(
        matched(&tree(html), "ul.books a.book", &no_counts()),
        Ok(vec![
            element(
                Some("book/1?a=1&b=2"),
                "First book",
                &[
                    ("class", "book"),
                    ("href", "book/1?a=1&b=2"),
                    ("data-id", "B1")
                ],
            ),
            element(None, "No link", &[("class", "book"), ("name", "anchor")]),
            element(
                Some(" mailto:shop@links.test "),
                "Mail",
                &[("class", "book"), ("href", " mailto:shop@links.test ")],
            ),
        ])
    );
}

#[test]
fn a_selector_that_is_no_css_is_refused() {
    assert_eq!(
        matched(&tree("<a href=x>x</a>"), "a >", &no_counts()),
        Err(SelectorInvalid("a >".to_owned()))
    );
}

/// A selector holding one of `dom_query`'s own extensions, which a browser
/// refuses, is refused on both tiers alike, so a handler's call answers
/// `invalid_params` wherever its page was served; the same text inside a
/// string or an escaped name is no extension.
///
/// Covers: story:6-232
#[rstest]
#[case::contains(r#"a:contains("Next")"#, false)]
#[case::has_text(r#"a:has-text("Next")"#, false)]
#[case::only_text("p:only-text", false)]
#[case::upper_case(r#"a:CONTAINS("Next")"#, false)]
#[case::inside_not(r#"li:not(:contains("x")) a"#, false)]
#[case::inside_has(r#"li:has(a:has-text("x"))"#, false)]
#[case::escaped_in_its_name(r#"a:\68 as-text("x")"#, false)]
#[case::in_an_attribute_value(r#"a[title=":contains(x)"]"#, true)]
#[case::an_escaped_colon_in_a_class(r".a\:contains", true)]
#[case::a_standard_pseudo_class("a:not(.ad):first-child", true)]
fn a_selector_a_browser_refuses_is_refused_on_both_tiers(
    #[case] selector: &str,
    #[case] accepted: bool,
) {
    let refused = Err(SelectorInvalid(selector.to_owned()));

    assert_eq!(
        check_selector(selector, &no_counts()).is_ok(),
        accepted,
        "{selector}"
    );
    assert_eq!(
        matched(&tree("<p><a href=x>x</a></p>"), selector, &no_counts()).is_ok(),
        accepted,
        "{selector}"
    );
    if !accepted {
        assert_eq!(check_selector(selector, &no_counts()), refused);
        assert_eq!(
            first_elements(
                &tree("<p>x</p>"),
                &[FieldLocator::Css {
                    selector: selector.to_owned()
                }],
                &no_counts()
            ),
            refused.map(|()| Vec::new()).map_err(LocatorInvalid::Css)
        );
    }
}

/// A link a served document's markup hides inside an element a button
/// names in `aria-controls`, a disclosure, is one a person reveals; one
/// hidden in an element nothing controls is not.
///
/// Covers: story:8-228
#[test]
fn a_link_hidden_in_what_a_button_controls_reads_revealable() {
    let html = r#"<button aria-expanded="false" aria-controls="other more">More</button>
<div id="more" hidden><a href="/shown">Shown</a></div>
<div id="less" hidden><a href="/kept">Kept</a></div>"#;

    let revealable: Vec<bool> = matched(&tree(html), "a", &no_counts())
        .unwrap()
        .into_iter()
        .map(|element| match element.sight {
            Sight::Markup(seen) => {
                assert!(seen.no_box);
                seen.revealable
            }
            Sight::Layout(_) => unreachable!("a served document has no layout"),
        })
        .collect();

    assert_eq!(revealable, [true, false]);
}

/// A field's `XPath` locator reads, in the served document, the first node
/// in document order it finds, as a browser's `document.evaluate` reads
/// it: the element's tag name, text and attributes; a locator whose first
/// node is no element (an attribute) or that finds none gives no element.
///
/// Covers: story:6-304
#[test]
fn an_xpath_locator_reads_the_first_element_it_finds_in_the_served_document() {
    let price = field(
        "span",
        "1.099,98 €",
        &[("class", "price"), ("data-value", "1099.98")],
    );
    let locators = [
        xpath("//article/h1"),
        xpath("//*[contains(@class, 'sku')]"),
        xpath("(//a | //span)[@data-value > 1000]"),
        xpath("//a[@class='manual']/@href"),
        xpath("//comment()/following-sibling::*[1]"),
        xpath("//table"),
    ];

    assert_eq!(
        first_elements(&tree(ARTICLE), &locators, &no_counts()),
        Ok(vec![
            Some(field("h1", "Delta-Drachen", &[("class", "title")])),
            Some(field("p", "Artikelnummer: DK-0042", &[("class", "sku")])),
            Some(price.clone()),
            None,
            Some(price),
            None,
        ])
    );
}

/// An `XPath` expression a browser refuses with no namespace resolver and
/// no variables, or one that answers a value rather than nodes, is refused
/// before any page is read, and by a served page's read alike.
///
/// Covers: story:6-304
#[rstest]
#[case::unclosed_predicate("//h1[")]
#[case::empty("")]
#[case::stray_token("//h1 h2")]
#[case::unclosed_literal("//a[@href='x]")]
#[case::prefix("//svg:rect")]
#[case::variable("//a[@id=$id]")]
#[case::unknown_function("//a[matches(., 'x')]")]
#[case::wrong_arity("//a[contains(.)]")]
#[case::unknown_axis("//a/sibling::b")]
#[case::number("count(//a)")]
#[case::string("string(//h1)")]
#[case::union_of_a_value("//a | 'b'")]
#[case::path_from_a_value("string(//h1)/b")]
#[case::count_of_a_value("//a[count('b') = 1]")]
fn an_xpath_a_browser_refuses_or_that_answers_a_value_is_refused(#[case] expression: &str) {
    assert!(
        check_xpath(expression, &no_counts()).is_err(),
        "{expression}"
    );
    assert!(
        matches!(
            first_elements(&tree(ARTICLE), &[xpath(expression)], &no_counts()),
            Err(LocatorInvalid::Xpath(_))
        ),
        "{expression}"
    );
}

/// The listing's products, as a served document holds them: the second
/// lacks a price.
const LISTING: &str = r#"<!doctype html><html><body><ul>
<li class="product"><a href="/kites/delta">Delta</a><span class="price">12</span></li>
<li class="product"><a href="/kites/box">Box</a></li>
</ul><span class="price">99</span></body></html>"#;

/// A list finds every element its locator finds, in document order, and
/// rows find, under each row's element, the first element each of its
/// fields' locators finds, an `XPath` expression read from the row: a row
/// lacking one finds none for it and keeps the others (ADR 0147).
#[test]
fn a_list_finds_every_element_and_each_row_its_fields_under_it() {
    let fields: std::collections::BTreeMap<String, PageField> = serde_json::from_value(json!({
        "names": { "list": { "locator": { "kind": "css", "selector": "li a" } } },
        "products": {
            "rows": { "kind": "xpath", "selector": "//li[@class='product']" },
            "fields": {
                "name": { "locator": { "kind": "xpath", "selector": "./a" } },
                "price": { "locator": { "kind": "css", "selector": ".price" } },
            },
        },
    }))
    .unwrap();
    let (delta, boxed) = (
        field("a", "Delta", &[("href", "/kites/delta")]),
        field("a", "Box", &[("href", "/kites/box")]),
    );

    assert_eq!(
        found(&tree(LISTING), &fields, &no_counts()),
        Ok(vec![
            Found::List(vec![delta.clone(), boxed.clone()]),
            Found::Rows(vec![
                vec![
                    Some(delta),
                    Some(field("span", "12", &[("class", "price")]))
                ],
                vec![Some(boxed), None],
            ]),
        ])
    );
}

/// The fields `json` declares, by name.
fn fields_of(json: serde_json::Value) -> std::collections::BTreeMap<String, PageField> {
    serde_json::from_value(json).unwrap()
}

/// An `XPath` locator is parsed once for the engine, kept in its cache as a
/// CSS selector's matcher is (issue 2713): a served page's `page.read`
/// reads its fields' locators on each call, and the first read of an
/// expression counts one on `F1.selector_compilations`, the second none.
#[test]
fn a_second_read_of_an_xpath_locator_parses_nothing() {
    let fields = fields_of(json!({
        "heading": { "locator": { "kind": "xpath", "selector": "//article/h1" } },
    }));
    let work = no_counts();

    let first = findings(&fields, &work).map(|findings| findings.len());
    let after_first = work.read(Row::PageSelectorCompilations);
    let second = findings(&fields, &work).map(|findings| findings.len());
    let after_second = work.read(Row::PageSelectorCompilations);

    assert_eq!(
        (first, after_first, second, after_second),
        (Ok(1), 1, Ok(1), 1)
    );
}

/// The crawl's fallback reads an `XPath` list field on each page whose
/// fields the untouched page's readers do not read: two pages parse its
/// expression once, not once a page (issue 2713).
#[test]
fn an_xpath_list_field_read_on_two_pages_is_parsed_once() {
    let fields = fields_of(json!({
        "names": { "list": { "locator": { "kind": "xpath", "selector": "//li/a" } } },
    }));
    let work = no_counts();
    let names = || {
        Found::List(vec![
            field("a", "Delta", &[("href", "/kites/delta")]),
            field("a", "Box", &[("href", "/kites/box")]),
        ])
    };

    let pages = [
        found(&tree(LISTING), &fields, &work),
        found(&tree(LISTING), &fields, &work),
    ];

    assert_eq!(
        (pages, work.read(Row::PageSelectorCompilations)),
        ([Ok(vec![names()]), Ok(vec![names()])], 1)
    );
}

/// An `XPath` locator the engine does not read is refused with the same
/// sentence on every read, its refusal kept for the engine as a valid
/// expression is: parsed once, counted once (issue 2713).
#[test]
fn an_invalid_xpath_locator_is_refused_alike_on_every_read() {
    let fields = fields_of(json!({
        "title": { "locator": { "kind": "xpath", "selector": "//h1[" } },
    }));
    let work = no_counts();

    let first = findings(&fields, &work).map(drop).unwrap_err();
    let second = findings(&fields, &work).map(drop).unwrap_err();

    assert_eq!(
        (second, work.read(Row::PageSelectorCompilations)),
        (first, 1)
    );
}

/// A field whose locator the engine does not read is refused by name, a
/// row's field as `rows/field`, before the document is read.
#[rstest]
#[case::a_value_field(
    json!({ "title": { "locator": { "kind": "css", "selector": "p[" } } }),
    "The field `title`'s `p[` is no CSS selector.",
)]
#[case::a_row_field(
    json!({ "products": {
        "rows": { "kind": "css", "selector": "li" },
        "fields": { "name": { "locator": { "kind": "xpath", "selector": "count(a)" } } },
    } }),
    "The field `products/name`'s",
)]
#[case::a_value_fields_later_rung(
    json!({ "title": {
        "locator": { "kind": "css", "selector": "h1" },
        "fallbacks": [{ "kind": "css", "selector": "h2" }, { "kind": "css", "selector": "p[" }],
    } }),
    "The field `title`'s `p[` is no CSS selector.",
)]
#[case::a_row_fields_later_rung(
    json!({ "products": {
        "rows": { "kind": "css", "selector": "li" },
        "fields": { "name": {
            "locator": { "kind": "css", "selector": "a" },
            "fallbacks": [{ "kind": "xpath", "selector": "count(a)" }],
        } },
    } }),
    "The field `products/name`'s `count(a)` is no XPath",
)]
fn a_field_whose_locator_the_engine_does_not_read_is_refused_by_name(
    #[case] declared: serde_json::Value,
    #[case] refused: &str,
) {
    let fields: std::collections::BTreeMap<String, PageField> =
        serde_json::from_value(declared).unwrap();

    let sentence = found(&tree(LISTING), &fields, &no_counts()).unwrap_err();

    assert!(sentence.starts_with(refused), "{sentence}");
}

/// A field's locator ladder (issue 1972) expands into the flat list of
/// finds the tree already reads: one per rung of a value field or a list,
/// in order, and each rung of a row's fields under each row, so the
/// extractor reads the first rung that found an element.
#[test]
fn a_ladder_finds_once_per_rung_in_order() {
    let fields: std::collections::BTreeMap<String, PageField> = serde_json::from_value(json!({
        "names": { "list": {
            "locator": { "kind": "css", "selector": "li b" },
            "fallbacks": [{ "kind": "css", "selector": "li a" }],
        } },
        "products": {
            "rows": { "kind": "css", "selector": "li.product" },
            "fields": {
                "name": { "locator": { "kind": "css", "selector": "a" } },
                "price": {
                    "locator": { "kind": "css", "selector": ".cost" },
                    "fallbacks": [{ "kind": "xpath", "selector": "./span[@class='price']" }],
                },
            },
        },
        "total": {
            "locator": { "kind": "css", "selector": "aside" },
            "fallbacks": [{ "kind": "xpath", "selector": "/html/body/span" }],
        },
    }))
    .unwrap();
    let (delta, boxed) = (
        field("a", "Delta", &[("href", "/kites/delta")]),
        field("a", "Box", &[("href", "/kites/box")]),
    );
    let price = |text: &str| field("span", text, &[("class", "price")]);

    assert_eq!(
        found(&tree(LISTING), &fields, &no_counts()),
        Ok(vec![
            Found::List(Vec::new()),
            Found::List(vec![delta.clone(), boxed.clone()]),
            Found::Rows(vec![
                vec![Some(delta), None, Some(price("12"))],
                vec![Some(boxed), None, None],
            ]),
            Found::One(None),
            Found::One(Some(price("99"))),
        ])
    );
}

/// The locators an untouched page's readers read for value fields are
/// each field's rungs in order (issue 1972), a field with no `fallbacks`
/// giving its one locator, as before.
#[test]
fn value_fields_give_their_rungs_and_one_locator_each_without_fallbacks() {
    let fields: std::collections::BTreeMap<String, PageField> = serde_json::from_value(json!({
        "name": {
            "locator": { "kind": "css", "selector": "h2" },
            "fallbacks": [{ "kind": "css", "selector": "h1" }],
        },
        "price": { "locator": { "kind": "xpath", "selector": "//span" } },
    }))
    .unwrap();

    assert_eq!(
        value_locators(&fields),
        Some(vec![css("h2"), css("h1"), xpath("//span")]),
    );
}
