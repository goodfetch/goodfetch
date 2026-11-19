//! The links a locator matches in a served document (ADRs 0093 and 0120),
//! decoded as the `charset` module's order reads it (ADR 0119): a locator
//! matches its CSS selector in the parsed document, each element read as
//! an `enqueueLinks` Payload function reads it: its `href` as written, its
//! text and its attributes, with what its markup and its holders' state
//! of whether a person sees it (the Frontier's `trap_links`). On the
//! browser tier, the page itself reads them, with what its layout shows.
//! A page's fields, which `page.read` and a code-free crawl read (ADRs
//! 0123 and 0147), are found alike: the first element a value field's
//! locator finds, every element a list's finds, and the first element
//! each of a row's fields finds among that row's descendants, each read
//! as its tag name, text and attributes, by a CSS selector or an `XPath`
//! 1.0 expression (the `xpath` module) in the served document; on the
//! browser tier the page finds them with `goodfetch_page::fields`.

use std::collections::{BTreeMap, HashSet};

use goodfetch_counted_work::{CountedWork, ParsedHtml, Row};
use goodfetch_extractor::fields::{self as declared, FieldElement, Found};
use goodfetch_frontier::trap_links::{Markup, Seen, Sight};
use goodfetch_protocol::page::read::{FieldFind, FieldLocator, PageField};
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

/// A selector that is no CSS selector the engine reads.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("`{0}` is no CSS selector")]
pub struct SelectorInvalid(pub String);

/// Every element of `document`, a served page's tree, that `selector`
/// matches, in document order, as [`matched_in`] reads them: a handler's
/// own read of a page, on its kept tree (spec #1577's #1918); a crawl
/// request's own document is read once, by [`matched_in`] as its untouched
/// tree is read.
///
/// # Errors
///
/// When `selector` is no CSS selector.
pub(crate) fn matched(
    document: &ParsedHtml,
    selector: &str,
    work: &CountedWork,
) -> Result<Vec<Matched>, SelectorInvalid> {
    let matcher = matcher(selector, work)?;
    let controls = work.literal_selector(Row::PageSelectorCompilations, CONTROLS);
    Ok(matched_in(document, &matcher, &controls))
}

/// What names the ids a button or a tab shows and hides: `aria-controls`.
pub(crate) const CONTROLS: &str = "[aria-controls]";

/// Every element of `document` that `matcher` matches, in document order,
/// with what its markup states of whether a person sees it, `controls`
/// being [`CONTROLS`]'s matcher.
pub(crate) fn matched_in(
    document: &ParsedHtml,
    matcher: &dom_query::Matcher,
    controls: &dom_query::Matcher,
) -> Vec<Matched> {
    let controlled = controlled_ids(
        document
            .select_matcher(controls)
            .nodes()
            .iter()
            .filter_map(|control| control.attr("aria-controls")),
    );
    let selection = document.select_matcher(matcher);
    selection
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
                    controlled: element
                        .attr("id")
                        .is_some_and(|id| controlled.contains(id.as_ref())),
                    attributes: attributes(&element),
                })
                .collect();
            judged(
                chain,
                node.attr("href").map(|href| href.to_string()),
                node.text().to_string(),
            )
        })
        .collect()
}

/// Each element of `matched`, which the streaming path read, as
/// [`matched_in`] reads it on the tree, `controls` being what
/// [`CONTROLS`] matched.
pub(crate) fn matched_streamed(
    matched: Vec<goodfetch_extractor::Streamed>,
    controls: &[goodfetch_extractor::Streamed],
) -> Vec<Matched> {
    let controlled = controlled_ids(
        controls
            .iter()
            .filter_map(|control| control.held.first()?.attr("aria-controls")),
    );
    matched
        .into_iter()
        .map(|streamed| {
            let href = streamed
                .held
                .first()
                .and_then(|own| own.attr("href"))
                .map(str::to_owned);
            let chain = streamed
                .held
                .into_iter()
                .map(|held| Held {
                    controlled: held.attr("id").is_some_and(|id| controlled.contains(id)),
                    name: held.name,
                    attributes: held.attributes,
                })
                .collect();
            judged(chain, href, streamed.text)
        })
        .collect()
}

/// The ids elements name in `aria-controls`, each of `named`: what a
/// button or a tab shows and hides.
fn controlled_ids<T: AsRef<str>>(named: impl Iterator<Item = T>) -> HashSet<String> {
    named
        .flat_map(|ids| {
            ids.as_ref()
                .split_ascii_whitespace()
                .map(str::to_owned)
                .collect::<Vec<_>>()
        })
        .collect()
}

/// The element `chain` starts, then each element holding it outward, with
/// its `href` and text, and what its markup states of whether a person
/// sees it.
fn judged(chain: Vec<Held>, href: Option<String>, text: String) -> Matched {
    let markup = Seen::in_markup(chain.iter().map(|held| Markup {
        name: &held.name,
        attributes: &held.attributes,
        controlled: held.controlled,
    }));
    Matched {
        href,
        text,
        attributes: chain
            .into_iter()
            .next()
            .map(|own| own.attributes)
            .unwrap_or_default(),
        sight: Sight::Markup(markup),
    }
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
pub(crate) fn matcher(
    selector: &str,
    work: &CountedWork,
) -> Result<dom_query::Matcher, SelectorInvalid> {
    let invalid = || SelectorInvalid(selector.to_owned());
    let matcher = work
        .selector(Row::PageSelectorCompilations, selector)
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

/// A field's locator, read once: a CSS selector's matcher or a checked
/// `XPath` expression.
#[derive(Clone, Debug)]
pub(crate) enum Locating {
    Css(dom_query::Matcher),
    Xpath(xpath::Expression),
}

/// `locator`, read once.
///
/// # Errors
///
/// When `locator` is no CSS selector or `XPath` expression the engine
/// reads.
pub(crate) fn locating(
    locator: &FieldLocator,
    work: &CountedWork,
) -> Result<Locating, LocatorInvalid> {
    Ok(match locator {
        FieldLocator::Css { selector } => Locating::Css(matcher(selector, work)?),
        FieldLocator::Xpath { selector } => Locating::Xpath(xpath::Expression::parse(selector)?),
    })
}

/// How one declared field finds its elements, its locators read once
/// (ADR 0147).
#[derive(Clone, Debug)]
pub(crate) enum Finding {
    /// A value field: the first element its locator finds.
    One(Locating),
    /// A list: every element its locator finds.
    List(Locating),
    /// Rows: every element the first locator finds, and in each, the first
    /// element each of its fields' locators finds, in the order of their
    /// names.
    Rows(Locating, Vec<Locating>),
}

/// How each of `fields` finds its elements, in the order of their names,
/// each field checked as the engine reads it on every tier: its locators,
/// then what the extractor's `fields::check` refuses. A field finds once
/// per find its read makes (`PageField::finds`), one per rung of a
/// locator ladder (issue 1972).
///
/// # Errors
///
/// The sentence refusing the first field the engine could never read.
pub(crate) fn findings(
    fields: &BTreeMap<String, PageField>,
    work: &CountedWork,
) -> Result<Vec<Finding>, String> {
    let mut findings = Vec::with_capacity(fields.len());
    for (name, field) in fields {
        findings.extend(field_findings(name, field, work)?);
        declared::check(name, field, work)?;
    }
    Ok(findings)
}

/// How the field `name` finds its elements, one finding per find its read
/// makes, its locators read through `work`'s cache and nothing else of it
/// checked.
///
/// # Errors
///
/// The sentence refusing a locator the engine does not read.
fn field_findings(
    name: &str,
    field: &PageField,
    work: &CountedWork,
) -> Result<Vec<Finding>, String> {
    let locate = |name: &str, locator: &FieldLocator| {
        locating(locator, work).map_err(|invalid| format!("The field `{name}`'s {invalid}."))
    };
    field
        .finds()
        .into_iter()
        .map(|find| {
            Ok(match find {
                FieldFind::One(locator) => Finding::One(locate(name, locator)?),
                FieldFind::List(locator) => Finding::List(locate(name, locator)?),
                FieldFind::Rows { rows, fields } => Finding::Rows(
                    locate(name, rows)?,
                    fields
                        .into_iter()
                        .map(|(row_field, locator)| locate(&format!("{name}/{row_field}"), locator))
                        .collect::<Result<_, _>>()?,
                ),
            })
        })
        .collect()
}

/// Refuses `fields` when the engine could never read one of them, as
/// [`findings`] checks them, compiling through `work`: a crawl's start
/// checks its fields on the engine's Counted work, where its Run then
/// finds them kept (#2605).
///
/// # Errors
///
/// The sentence refusing the first field the engine could never read.
pub fn check_fields(
    fields: &BTreeMap<String, PageField>,
    work: &CountedWork,
) -> Result<(), String> {
    findings(fields, work).map(drop)
}

/// Each find of `fields`' reads (`PageField::finds`), in the order of
/// their names, when every one is a value field: what an untouched page's
/// readers find as they read it (the `untouched` module), one locator per
/// rung of each field's ladder (issue 1972); `None` when one is a list or
/// rows.
#[must_use]
pub fn value_locators(fields: &BTreeMap<String, PageField>) -> Option<Vec<FieldLocator>> {
    fields
        .values()
        .flat_map(PageField::finds)
        .map(|find| match find {
            FieldFind::One(locator) => Some(locator.clone()),
            FieldFind::List(_) | FieldFind::Rows { .. } => None,
        })
        .collect()
}

/// What each of a crawl's `fields` finds in `document`, a served page's
/// tree, as [`found_in`] reads it: the crawl checked its fields as it
/// started ([`check_fields`]), so a page reads only their locators.
///
/// # Errors
///
/// The sentence refusing the first locator the engine does not read.
pub(crate) fn found(
    document: &ParsedHtml,
    fields: &BTreeMap<String, PageField>,
    work: &CountedWork,
) -> Result<Vec<Found>, String> {
    let mut findings = Vec::with_capacity(fields.len());
    for (name, field) in fields {
        findings.extend(field_findings(name, field, work)?);
    }
    Ok(found_in(document, &findings))
}

/// What each of `findings` finds in `document`, in order, as a browser
/// reads it: a CSS selector's matches, or the nodes an `XPath`
/// expression finds that are elements, in document order; a row's fields
/// found among its descendants, an `XPath` expression read from the row.
pub(crate) fn found_in(document: &ParsedHtml, findings: &[Finding]) -> Vec<Found> {
    let mut finder = Finder {
        document,
        xpath: None,
    };
    findings
        .iter()
        .map(|finding| match finding {
            Finding::One(locating) => Found::One(finder.first(locating, None).map(field_element)),
            Finding::List(locating) => Found::List(
                finder
                    .all(locating, None)
                    .into_iter()
                    .map(field_element)
                    .collect(),
            ),
            Finding::Rows(rows, fields) => Found::Rows(
                finder
                    .all(rows, None)
                    .into_iter()
                    .map(|row| {
                        fields
                            .iter()
                            .map(|field| finder.first(field, Some(row)).map(field_element))
                            .collect()
                    })
                    .collect(),
            ),
        })
        .collect()
}

/// The first element of `document` that each of `locating` finds, in
/// order, as [`found_in`] reads a value field's; `None` for one that finds
/// none.
pub(crate) fn first_elements_in(
    document: &ParsedHtml,
    locating: &[Locating],
) -> Vec<Option<FieldElement>> {
    let mut finder = Finder {
        document,
        xpath: None,
    };
    locating
        .iter()
        .map(|locating| finder.first(locating, None).map(field_element))
        .collect()
}

/// What finds elements in one document: its `XPath` tree, built once, at
/// the first `XPath` locator.
struct Finder<'a> {
    document: &'a ParsedHtml,
    xpath: Option<xpath::Document>,
}

impl<'a> Finder<'a> {
    /// Every element `locating` finds in the document, or among the
    /// descendants of `within`, in document order.
    fn all(
        &mut self,
        locating: &Locating,
        within: Option<dom_query::NodeRef<'a>>,
    ) -> Vec<dom_query::NodeRef<'a>> {
        match locating {
            Locating::Css(matcher) => match within {
                None => self.document.select_matcher(matcher).nodes().to_vec(),
                Some(row) => dom_query::Selection::from(row)
                    .select_matcher(matcher)
                    .nodes()
                    .to_vec(),
            },
            Locating::Xpath(expression) => {
                let document = self.document;
                self.xpath
                    .get_or_insert_with(|| xpath::Document::of(document))
                    .elements(expression, within.map(|row| row.id))
                    .into_iter()
                    .filter_map(|id| document.tree.get(&id))
                    .collect()
            }
        }
    }

    /// The first element `locating` finds in the document, or among the
    /// descendants of `within`.
    fn first(
        &mut self,
        locating: &Locating,
        within: Option<dom_query::NodeRef<'a>>,
    ) -> Option<dom_query::NodeRef<'a>> {
        match locating {
            Locating::Css(matcher) => match within {
                None => self
                    .document
                    .select_matcher(matcher)
                    .nodes()
                    .first()
                    .copied(),
                Some(row) => dom_query::Selection::from(row)
                    .select_matcher(matcher)
                    .nodes()
                    .first()
                    .copied(),
            },
            Locating::Xpath(expression) => {
                let document = self.document;
                self.xpath
                    .get_or_insert_with(|| xpath::Document::of(document))
                    .first_element(expression, within.map(|row| row.id))
                    .and_then(|id| document.tree.get(&id))
            }
        }
    }
}

/// `node` as a field reads it: its tag name, lower-cased, its text and its
/// attributes.
fn field_element(node: dom_query::NodeRef<'_>) -> FieldElement {
    FieldElement {
        tag: node
            .node_name()
            .map(|name| name.to_ascii_lowercase())
            .unwrap_or_default(),
        text: node.text().to_string(),
        attributes: attributes(&node),
    }
}

/// The field element the streaming path read as `streamed`, as
/// [`found_in`] reads one on the tree.
pub(crate) fn field_streamed(streamed: goodfetch_extractor::Streamed) -> Option<FieldElement> {
    let own = streamed.held.into_iter().next()?;
    Some(FieldElement {
        tag: own.name,
        text: streamed.text,
        attributes: own.attributes,
    })
}

/// Refuses `selector` when it is no CSS selector the engine reads on both
/// tiers, compiling it through `work`: a crawl's start checks a field's
/// selector on the engine's Counted work, where its Run then finds it kept
/// (#2605).
///
/// # Errors
///
/// When `selector` is no CSS selector a browser reads.
pub fn check_selector(selector: &str, work: &CountedWork) -> Result<(), SelectorInvalid> {
    matcher(selector, work).map(drop)
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
