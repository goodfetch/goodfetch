//! The links a locator matches in a served document (ADRs 0093 and 0120),
//! decoded as the `charset` module's order reads it (ADR 0119): a locator
//! matches its CSS selector in the parsed document, each element read as
//! an `enqueueLinks` Payload function reads it: its `href` as written, its
//! text and its attributes. On the browser tier, the page itself reads
//! them, with what a person sees of each (the Frontier's `trap_links`).
//! A code-free crawl's fields (ADR 0123) read the first element each
//! field's locator finds, alike: its tag name, text and attributes; on the
//! browser tier the page reads them with [`FIELDS_READ`], CSS selectors and
//! `XPath` expressions alike.

use goodfetch_frontier::trap_links::Seen;
use serde_json::Value;

/// One element a locator matched.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Matched {
    /// Its `href` attribute, as written; `None` without one.
    pub href: Option<String>,
    /// Its text content.
    pub text: String,
    /// Its attributes, in document order.
    pub attributes: Vec<(String, String)>,
    /// What a person sees of it, as a browser page read it; `None` from a
    /// served document, which has no layout.
    pub seen: Option<Seen>,
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
pub(crate) fn matched(html: &str, selector: &str) -> Result<Vec<Matched>, SelectorInvalid> {
    let matcher =
        dom_query::Matcher::new(selector).map_err(|_| SelectorInvalid(selector.to_owned()))?;
    let document = dom_query::Document::from(html);
    let selection = document.select_matcher(&matcher);
    Ok(selection
        .nodes()
        .iter()
        .map(|node| Matched {
            href: node.attr("href").map(|href| href.to_string()),
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
            seen: None,
        })
        .collect())
}

/// The first element of `html` that each of `selectors`, CSS selectors,
/// matches, in order; `None` for one that matches none.
///
/// # Errors
///
/// When one of `selectors` is no CSS selector.
pub(crate) fn first_elements(
    html: &str,
    selectors: &[String],
) -> Result<Vec<Option<FieldElement>>, SelectorInvalid> {
    let document = dom_query::Document::from(html);
    selectors
        .iter()
        .map(|selector| {
            let matcher =
                dom_query::Matcher::new(selector).map_err(|_| SelectorInvalid(selector.clone()))?;
            let selection = document.select_matcher(&matcher);
            Ok(selection.nodes().first().map(|node| FieldElement {
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

/// Refuses `selector` when it is no CSS selector the engine reads.
///
/// # Errors
///
/// When `selector` is no CSS selector.
pub fn check_selector(selector: &str) -> Result<(), SelectorInvalid> {
    dom_query::Matcher::new(selector)
        .map(|_| ())
        .map_err(|_| SelectorInvalid(selector.to_owned()))
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
                seen: Some(Seen::read(element.get("seen")?)?),
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
