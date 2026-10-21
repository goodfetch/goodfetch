//! The one parse of every URL a crawl meets (ADR 0120; pitfall U07): seeds,
//! enqueues, `href`s, sitemap entries, `Location`s and Hook rewrites go
//! through the WHATWG URL Standard, as the `url` crate implements it, once,
//! in the Frontier, so every fetch tier receives the URL a browser would
//! request: its host lowercased and IDNA-encoded, an IPv4 host in its
//! dotted form, its default port dropped, a backslash read as a slash,
//! tabs and newlines removed, and its other characters percent-encoded as
//! the Standard says. A crawl fetches only `http` and `https`.

use goodfetch_protocol::foxhound::frontier::NotAddedReason;
use url::Url;

/// The schemes a crawl fetches.
const FETCHED: [&str; 2] = ["http", "https"];

/// Parses `input` as the WHATWG URL Standard does, against `base` when
/// given: `invalid_url` when the parser refuses it, or when it is relative
/// and has no base; `unsupported_scheme` for a URL the crawl cannot fetch.
///
/// # Errors
///
/// The reason a request to `input` is not added.
pub(crate) fn parse(input: &str, base: Option<&Url>) -> Result<Url, NotAddedReason> {
    let url = Url::options()
        .base_url(base)
        .parse(input)
        .map_err(|_| NotAddedReason::InvalidUrl)?;
    if FETCHED.contains(&url.scheme()) {
        Ok(url)
    } else {
        Err(NotAddedReason::UnsupportedScheme)
    }
}

/// The base URL of a document reached at `document`, whose first `<base>`
/// element has the `href` `base_href`, as HTML sets it: the `href` resolved
/// against the document's URL, or the document's URL itself when there is
/// none, when it does not parse, or when it is a `data:` or `javascript:`
/// URL; `None` when the document's own URL does not parse.
pub fn document_base(document: &str, base_href: Option<&str>) -> Option<Url> {
    let document = Url::parse(document).ok()?;
    Some(
        base_href
            .and_then(|href| document.join(href).ok())
            .filter(|base| !["data", "javascript"].contains(&base.scheme()))
            .unwrap_or(document),
    )
}

#[cfg(test)]
mod tests;
