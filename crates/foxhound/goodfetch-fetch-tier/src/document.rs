//! The links a locator matches in a served document (ADRs 0093 and 0120),
//! decoded as the `charset` module's order reads it (ADR 0119): a locator
//! matches its CSS selector in the parsed document, each element read as
//! an `enqueueLinks` Payload function reads it: its `href` as written, its
//! text and its attributes, with what its markup and its holders' state
//! of whether a person sees it (the Frontier's `trap_links`). On the
//! browser tier, the page itself reads them, with what its layout shows.
//! A code-free crawl's fields (ADR 0123) read the first element each
//! field's locator finds, alike: its tag name, text and attributes, by a
//! CSS selector or an `XPath` 1.0 expression (the `xpath` module) in the
//! served document; on the browser tier the page reads them with
//! [`FIELDS_READ`].

use std::collections::HashSet;

use goodfetch_counted_work::{CountedWork, Row};
use goodfetch_frontier::trap_links::{Markup, Seen, Sight};
use goodfetch_protocol::foxhound::crawl::FieldLocator;
use serde_json::Value;

use crate::xpath::{self, XpathInvalid};

/// One element a locator matched.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Matched {
    /// Its `href` attribute, as written; `None` without one.
    pub href: Option<String>,
    /// Its text content.
    pub text: String,
    /// Its attributes, in document order.
    pub attributes: Vec<(String, String)>,
    /// What a person sees of it: as a browser page's layout shows it, or
    /// as a served document's markup states it.
    pub sight: Sight,
}

/// The first element a code-free crawl's field's locator found in a page
/// (ADR 0123).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FieldElement {
    /// Its tag name, lower-cased.
    pub tag: String,
    /// Its text content.
    pub text: String,
    /// Its attributes, in document order.
    pub attributes: Vec<(String, String)>,
}

/// The page function that reads the first element each of `locators`
/// finds, called with `{ locators }`, each `{ kind, selector }` as a
/// field's locator is written; answers `{ baseUrl, elements }`, each
/// element `null` or `{ tag, text, attributes }`.
pub(crate) const FIELDS_READ: &str = include_str!("document/fields.js");

/// A selector that is no CSS selector the engine reads.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("`{0}` is no CSS selector")]
pub struct SelectorInvalid(pub String);

/// Every element of `html` that `selector` matches, in document order.
///
/// # Errors
///
/// When `selector` is no CSS selector.
pub(crate) fn matched(
    html: &str,
    selector: &str,
    work: &CountedWork,
) -> Result<Vec<Matched>, SelectorInvalid> {
    let matcher = matcher(selector, work)?;
    let document = work.parse_html(Row::PageParses, html);
    // The ids an element names in `aria-controls`: what a button or a tab
    // shows and hides.
    let controlled: HashSet<String> = document
        .select_matcher(&work.literal_selector(Row::PageSelectorCompilations, "[aria-controls]"))
        .nodes()
        .iter()
        .filter_map(|control| control.attr("aria-controls"))
        .flat_map(|ids| {
            ids.split_ascii_whitespace()
                .map(str::to_owned)
                .collect::<Vec<_>>()
        })
        .collect();
    let selection = document.select_matcher(&matcher);
    Ok(selection
        .nodes()
        .iter()
        .map(|node| {
            // The element, then each element holding it outward.
            let chain: Vec<Held> = std::iter::once(*node)
                .chain(node.ancestors_it(None))
                .map(|element| Held {
                    name: element
                        .node_name()
                        .map(|name| name.to_ascii_lowercase())
                        .unwrap_or_default(),
                    attributes: attributes(&element),
                    controlled: element
                        .attr("id")
                        .is_some_and(|id| controlled.contains(id.as_ref())),
                })
                .collect();
            let markup = Seen::in_markup(chain.iter().map(|held| Markup {
                name: &held.name,
                attributes: &held.attributes,
                controlled: held.controlled,
            }));
            Matched {
                href: node.attr("href").map(|href| href.to_string()),
                text: node.text().to_string(),
                attributes: chain
                    .into_iter()
                    .next()
                    .map(|own| own.attributes)
                    .unwrap_or_default(),
                sight: Sight::Markup(markup),
            }
        })
        .collect())
}

/// One element of a served document as its markup states it: its tag name,
/// lower-cased, its attributes, and whether an element names it in
/// `aria-controls`.
struct Held {
    name: String,
    attributes: Vec<(String, String)>,
    controlled: bool,
}

/// The pseudo-classes `dom_query` reads beyond CSS, which a browser
/// refuses: `:contains()`, `:has-text()` and `:only-text`.
const EXTENSIONS: [&str; 3] = ["contains", "has-text", "only-text"];

/// The matcher of `selector`, refused when a browser refuses it too: one
/// naming one of `dom_query`'s [`EXTENSIONS`], so both tiers read one set
/// of selectors.
///
/// # Errors
///
/// When `selector` is no CSS selector a browser reads.
fn matcher(selector: &str, work: &CountedWork) -> Result<dom_query::Matcher, SelectorInvalid> {
    let invalid = || SelectorInvalid(selector.to_owned());
    let matcher = work
        .compile_selector(Row::PageSelectorCompilations, selector)
        .ok_or_else(invalid)?;
    let mut input = cssparser::ParserInput::new(selector);
    if extended(&mut cssparser::Parser::new(&mut input)) {
        return Err(invalid());
    }
    Ok(matcher)
}

/// Whether what `parser` reads, a selector `dom_query` parsed, names one
/// of [`EXTENSIONS`] after a `:`, at any depth: a string, an attribute's
/// value or an escaped `:` names none, and an escaped name reads
/// unescaped, as `dom_query` reads it.
fn extended(parser: &mut cssparser::Parser<'_, '_>) -> bool {
    let mut found = false;
    let mut after_colon = false;
    while let Ok(token) = parser.next_including_whitespace_and_comments() {
        let named = match token {
            cssparser::Token::Ident(name) | cssparser::Token::Function(name) => Some(name),
            _ => None,
        };
        found |= after_colon
            && named.is_some_and(|name| {
                EXTENSIONS
                    .iter()
                    .any(|extension| extension.eq_ignore_ascii_case(name))
            });
        after_colon = matches!(token, cssparser::Token::Colon);
        let block = matches!(
            token,
            cssparser::Token::Function(_)
                | cssparser::Token::ParenthesisBlock
                | cssparser::Token::SquareBracketBlock
                | cssparser::Token::CurlyBracketBlock
        );
        if block {
            let nested: Result<bool, cssparser::ParseError<'_, ()>> =
                parser.parse_nested_block(|inner| Ok(extended(inner)));
            found |= nested.unwrap_or(false);
        }
    }
    found
}

/// The attributes of `element`, in document order.
fn attributes(element: &dom_query::NodeRef<'_>) -> Vec<(String, String)> {
    element
        .attrs()
        .iter()
        .map(|attribute| {
            (
                attribute.name.local.to_string(),
                attribute.value.to_string(),
            )
        })
        .collect()
}

/// A field's locator the engine does not read.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub(crate) enum LocatorInvalid {
    #[error(transparent)]
    Css(#[from] SelectorInvalid),
    #[error(transparent)]
    Xpath(#[from] XpathInvalid),
}

/// The first element of `html` that each of `locators` finds, in order:
/// a CSS selector's first match, or an `XPath` expression's first node in
/// document order when it is an element, as a browser reads both; `None`
/// for one that finds none.
///
/// # Errors
///
/// When one of `locators` is no CSS selector or `XPath` expression the
/// engine reads.
pub(crate) fn first_elements(
    html: &str,
    locators: &[FieldLocator],
    work: &CountedWork,
) -> Result<Vec<Option<FieldElement>>, LocatorInvalid> {
    let document = work.parse_html(Row::PageParses, html);
    // Built once, at the first `XPath` locator.
    let mut held: Option<xpath::Document> = None;
    locators
        .iter()
        .map(|locator| {
            let found = match locator {
                FieldLocator::Css { selector } => {
                    let matcher = matcher(selector, work)?;
                    document.select_matcher(&matcher).nodes().first().copied()
                }
                FieldLocator::Xpath { selector } => {
                    let expression = xpath::Expression::parse(selector)?;
                    held.get_or_insert_with(|| xpath::Document::of(&document))
                        .first_element(&expression)
                        .and_then(|id| document.tree.get(&id))
                }
            };
            Ok(found.map(|node| FieldElement {
                tag: node
                    .node_name()
                    .map(|name| name.to_ascii_lowercase())
                    .unwrap_or_default(),
                text: node.text().to_string(),
                attributes: node
                    .attrs()
                    .iter()
                    .map(|attribute| {
                        (
                            attribute.name.local.to_string(),
                            attribute.value.to_string(),
                        )
                    })
                    .collect(),
            }))
        })
        .collect()
}

/// What a browser page answered [`FIELDS_READ`]: its base URL, and the
/// first element each locator found; `None` for an answer it never gives.
pub(crate) fn browser_fields(answer: &Value) -> Option<(String, Vec<Option<FieldElement>>)> {
    let text = |value: &Value, name: &str| value.get(name)?.as_str().map(str::to_owned);
    let elements = answer
        .get("elements")?
        .as_array()?
        .iter()
        .map(|element| {
            if element.is_null() {
                return Some(None);
            }
            Some(Some(FieldElement {
                tag: text(element, "tag")?,
                text: text(element, "text")?,
                attributes: element
                    .get("attributes")?
                    .as_array()?
                    .iter()
                    .map(|pair| {
                        let at = |index: usize| pair.get(index)?.as_str().map(str::to_owned);
                        Some((at(0)?, at(1)?))
                    })
                    .collect::<Option<_>>()?,
            }))
        })
        .collect::<Option<_>>()?;
    Some((text(answer, "baseUrl")?, elements))
}

/// Refuses `selector` when it is no CSS selector the engine reads on both
/// tiers.
///
/// # Errors
///
/// When `selector` is no CSS selector a browser reads.
pub fn check_selector(selector: &str) -> Result<(), SelectorInvalid> {
    // A crawl's start refuses it here: no page's work, so counted nowhere.
    matcher(selector, &CountedWork::default()).map(drop)
}

/// Refuses `expression` when it is no `XPath` 1.0 expression the engine
/// reads to nodes on both tiers.
///
/// # Errors
///
/// When a browser refuses `expression` with no namespace resolver and no
/// variables, or it answers a value rather than nodes.
pub fn check_xpath(expression: &str) -> Result<(), XpathInvalid> {
    xpath::Expression::parse(expression).map(drop)
}

/// What a browser page answered the Frontier's `trap_links::READ`: its
/// URL, base URL and encoding's name, and each element it matched; `None`
/// for an answer it never gives.
pub(crate) fn browser_matched(answer: &Value) -> Option<(String, String, String, Vec<Matched>)> {
    let text = |value: &Value, name: &str| value.get(name)?.as_str().map(str::to_owned);
    let elements = answer
        .get("elements")?
        .as_array()?
        .iter()
        .map(|element| {
            Some(Matched {
                href: text(element, "href"),
                text: text(element, "text")?,
                attributes: element
                    .get("attributes")?
                    .as_array()?
                    .iter()
                    .map(|pair| {
                        let at = |index: usize| pair.get(index)?.as_str().map(str::to_owned);
                        Some((at(0)?, at(1)?))
                    })
                    .collect::<Option<_>>()?,
                sight: Sight::Layout(Seen::read(element.get("seen")?)?),
            })
        })
        .collect::<Option<_>>()?;
    Some((
        text(answer, "url")?,
        text(answer, "baseUrl")?,
        text(answer, "encoding")?,
        elements,
    ))
}

#[cfg(test)]
mod tests;
