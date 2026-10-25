//! The links a locator matches in a served document (ADRs 0093 and 0120),
//! decoded as the `charset` module's order reads it (ADR 0119): a locator
//! matches its CSS selector in the parsed document, each element read as
//! an `enqueueLinks` Payload function reads it: its `href` as written, its
//! text and its attributes.

/// One element a locator matched.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Matched {
    /// Its `href` attribute, as written; `None` without one.
    pub href: Option<String>,
    /// Its text content.
    pub text: String,
    /// Its attributes, in document order.
    pub attributes: Vec<(String, String)>,
}

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
        })
        .collect())
}

#[cfg(test)]
mod tests;
