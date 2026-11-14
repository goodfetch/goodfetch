//! The media types a Label's handler takes (`crawl.content_types`, ADR
//! 0119; pitfall H16), and the type of a response a crawl request's fetch
//! judges by them. Pure: no I/O and no task.
//!
//! - **A response's type** is its `Content-Type` essence; with none, the
//!   type Chromium's sniffer gives its body (the `sniffing` module); with
//!   neither a type nor a body, as a listed redirect often has, an empty
//!   HTML document, so it reaches the handler. A `Content-Type` naming no
//!   type counts as none, so its body is sniffed (ADR 0119; the WHATWG
//!   MIME Sniffing standard, section 7; Chromium's `IsUnknownMimeType`):
//!   an empty one, one with no slash, `*/*`, `unknown/unknown` and
//!   `application/unknown`.
//! - **Which responses are judged:** those that would reach the handler, a
//!   2xx or a status `crawl.handle_statuses` lists ([`Accepts`]). Another
//!   type ends the request `skipped` with `content_type`, the driver
//!   dropping the body unread once the head named the type.
//! - **Documents:** only HTML and XHTML are; any other accepted type
//!   reaches the handler with a page that holds no document.

use std::collections::BTreeSet;
use std::sync::Arc;

use serde_json::Value;

use crate::charset::content_type;
use crate::sniffing::sniff;

/// HTML's essence.
const HTML: &str = "text/html";

/// XHTML's essence.
const XHTML: &str = "application/xhtml+xml";

/// One media range of `crawl.content_types`.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Range {
    /// `*/*`.
    Any,
    /// `type/*`: this type, any subtype.
    Type(String),
    /// `type/subtype`.
    Exact(String),
}

/// The media types a Label's handler takes: `text/html` and
/// `application/xhtml+xml` by default. Shared, not copied: each attempt's
/// hand-over holds its Label's or the Run's (#2243).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct ContentTypes(Arc<[Range]>);

impl Default for ContentTypes {
    fn default() -> Self {
        Self(Arc::from([
            Range::Exact(HTML.to_owned()),
            Range::Exact(XHTML.to_owned()),
        ]))
    }
}

impl ContentTypes {
    /// The media ranges `value` lists, as `crawl.content_types` sets them.
    ///
    /// # Errors
    ///
    /// Why `value` is no list of media ranges, as a phrase that follows the
    /// value: it is no list, or it holds an item that is no text, or one
    /// that is no `type/subtype`, `type/*` or `*/*`.
    pub fn of(value: &Value) -> Result<Self, String> {
        let Some(items) = value.as_array() else {
            return Err("is no list of media types".to_owned());
        };
        items
            .iter()
            .map(|item| {
                item.as_str().and_then(range).ok_or_else(|| {
                    format!("holds {item}, which is no `type/subtype`, `type/*` or `*/*`")
                })
            })
            .collect::<Result<_, _>>()
            .map(Self)
    }

    /// Whether a response whose type is `essence`, lower-cased, is taken.
    pub fn accepts(&self, essence: &str) -> bool {
        let kind = essence.split('/').next().unwrap_or_default();
        self.0.iter().any(|range| match range {
            Range::Any => true,
            Range::Type(taken) => taken == kind,
            Range::Exact(taken) => taken == essence,
        })
    }
}

/// The media range `text` names, in any case; `None` when it is none.
fn range(text: &str) -> Option<Range> {
    let text = text.trim().to_ascii_lowercase();
    let (kind, subtype) = text.split_once('/')?;
    let token = |part: &str| {
        !part.is_empty()
            && part
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"!#$&-^_.+".contains(&byte))
    };
    match (kind, subtype) {
        ("*", "*") => Some(Range::Any),
        (kind, "*") if token(kind) => Some(Range::Type(kind.to_owned())),
        (kind, subtype) if token(kind) && token(subtype) => Some(Range::Exact(text.clone())),
        _ => None,
    }
}

/// What a crawl request's fetch judges its document's response by: its
/// Label's `crawl.content_types`, and the statuses `crawl.handle_statuses`
/// hands to the handler besides the 2xx.
#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Accepts {
    pub(crate) types: ContentTypes,
    pub(crate) handled: BTreeSet<u16>,
}

impl Accepts {
    /// The type of a response of `status` whose type is `essence`, when it
    /// would reach the handler and its Label does not take it; `None`
    /// otherwise.
    pub(crate) fn refused(&self, status: u16, essence: &str) -> Option<String> {
        let reaches = (200..300).contains(&status) || self.handled.contains(&status);
        (reaches && !self.types.accepts(essence)).then(|| essence.to_owned())
    }
}

/// The essences a `Content-Type` names no type by, beside an empty one and
/// one with no slash: its body is sniffed.
const UNKNOWN: [&str; 3] = ["*/*", "unknown/unknown", "application/unknown"];

/// `header`, a response's `Content-Type`, when it names a type; `None` when
/// it names none, as [`UNKNOWN`] and a value with no slash do, so the
/// response's body is sniffed.
pub(crate) fn declared(header: Option<&str>) -> Option<&str> {
    header.filter(|value| {
        let (essence, _) = content_type(value);
        essence.contains('/') && !UNKNOWN.contains(&essence.as_str())
    })
}

/// The type of a response whose `Content-Type` is `header`, its body
/// `body` and its URL's path `path`: its essence, lower-cased, and its
/// charset, if it names one. With no `Content-Type`, or one naming no type
/// ([`declared`]), the type Chromium's sniffer gives the body, and
/// `text/html` for no body at all.
pub(crate) fn typed(header: Option<&str>, body: &[u8], path: &str) -> (String, Option<String>) {
    match declared(header) {
        Some(value) => {
            let (essence, charset) = content_type(value);
            (essence, charset.map(str::to_owned))
        }
        None if body.is_empty() => (HTML.to_owned(), None),
        None => (sniff(body, path).to_owned(), None),
    }
}

/// Whether a response of the type `essence` is a document: HTML or XHTML.
pub(crate) fn is_document(essence: &str) -> bool {
    essence == HTML || essence == XHTML
}

#[cfg(test)]
mod tests;
