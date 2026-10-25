//! A served document's text and the links a locator matches in it (ADRs
//! 0093, 0119 and 0120). Its bytes are decoded in a BOM's encoding, else
//! the `Content-Type` charset's, a WHATWG Encoding label, else as UTF-8,
//! a byte that is none replaced, the encoding kept, since a link's query
//! is percent-encoded in it; `fetch.encoding`, Chrome's `<meta>` prescan
//! and the escalation of a body only a guess decodes are #764's. A locator
//! matches its CSS selector in the parsed document, each element read as
//! an `enqueueLinks` Payload function reads it: its `href` as written, its
//! text and its attributes.

use encoding_rs::Encoding;

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

/// `body`, the bytes of a document whose response had the `Content-Type`
/// `content_type`, decoded, with the encoding that decoded it.
pub(crate) fn decode(body: &[u8], content_type: Option<&str>) -> (String, &'static Encoding) {
    let labelled = content_type
        .and_then(charset)
        .and_then(|label| Encoding::for_label(label.as_bytes()))
        .unwrap_or(encoding_rs::UTF_8);
    // `decode` reads a BOM first, whatever the label says.
    let (text, encoding, _) = labelled.decode(body);
    (text.into_owned(), encoding)
}

/// The `charset` parameter of the `Content-Type` value `content_type`,
/// unquoted.
fn charset(content_type: &str) -> Option<&str> {
    content_type.split(';').skip(1).find_map(|parameter| {
        let (name, value) = parameter.split_once('=')?;
        name.trim()
            .eq_ignore_ascii_case("charset")
            .then(|| value.trim().trim_matches('"'))
    })
}

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
