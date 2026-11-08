//! A fetched page's `rel=canonical` (ADR 0120), which the Frontier reads
//! as evidence and never as a merge: the first `<link rel="canonical">`
//! in its `<head>` with an `href`, read by the extractor's rule for
//! `metadata.canonical`, or else the first `Link` header field value with
//! `rel="canonical"`, resolved against the URL it was fetched at.

use goodfetch_counted_work::{CountedWork, ParsedHtml, Row};
use goodfetch_extractor::{canonical_url, is_canonical_link};
use url::Url;

/// The URL `html`'s head, or else its `headers` (names lower-cased), name
/// as its canonical; `None` when they name none that parses. `html` is
/// parsed for this call alone; a crawl request's own document is read
/// once, by [`in_head`] on its untouched tree.
#[expect(
    clippy::disallowed_methods,
    reason = "ADR 0138: a document no untouched tree read, parsed for this call alone"
)]
pub(crate) fn canonical(
    html: &str,
    headers: &[(String, String)],
    base: &Url,
    url: &Url,
    work: &CountedWork,
) -> Option<Url> {
    let document = work.parse_html(Row::PageParses, html);
    let link = work.literal_selector(Row::PageSelectorCompilations, LINK);
    in_head(&document, &link, base).or_else(|| in_headers(headers, url))
}

/// What names a document's head links, among them its canonical.
pub(crate) const LINK: &str = "head link[rel][href]";

/// The URL the first `<link rel="canonical">` with an `href` in
/// `document`'s head names, resolved against `base`, `link` being
/// [`LINK`]'s matcher; `None` when it names none that parses.
pub(crate) fn in_head(document: &ParsedHtml, link: &dom_query::Matcher, base: &Url) -> Option<Url> {
    document
        .select_matcher(link)
        .nodes()
        .iter()
        .find(|link| link.attr("rel").is_some_and(|rel| is_canonical_link(&rel)))
        .and_then(|link| link.attr("href"))
        .and_then(|href| canonical_url(&href, base))
}

/// The URL the first `Link` header field value with `rel="canonical"`
/// among `headers` (names lower-cased) names, resolved against `url`.
pub(crate) fn in_headers(headers: &[(String, String)], url: &Url) -> Option<Url> {
    headers
        .iter()
        .filter(|(name, _)| name == "link")
        .flat_map(|(_, value)| links(value))
        .find_map(|(target, rel)| {
            is_canonical_link(rel)
                .then(|| url.join(target).ok())
                .flatten()
        })
}

/// Each link of a `Link` field value (RFC 8288): its target, between `<`
/// and `>`, and its `rel` parameter, unquoted.
fn links(value: &str) -> Vec<(&str, &str)> {
    let mut found = Vec::new();
    let mut rest = value;
    while let Some(start) = rest.find('<') {
        let Some(end) = rest[start..].find('>') else {
            break;
        };
        let target = &rest[start + 1..start + end];
        let after = &rest[start + end + 1..];
        let parameters = after.split_once('<').map_or(after, |(head, _)| head);
        let rel = parameters
            .split(';')
            .filter_map(|parameter| parameter.split_once('='))
            .find(|(name, _)| name.trim().eq_ignore_ascii_case("rel"))
            .map_or("", |(_, value)| {
                value.trim().trim_end_matches(',').trim().trim_matches('"')
            });
        found.push((target, rel));
        rest = after;
    }
    found
}

#[cfg(test)]
mod tests;
