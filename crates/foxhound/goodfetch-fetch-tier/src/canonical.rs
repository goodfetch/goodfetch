//! A fetched page's `rel=canonical` (ADR 0120), which the Frontier reads
//! as evidence and never as a merge: the first `<link rel="canonical">`
//! in its `<head>` with an `href`, resolved against its base URL as HTML
//! resolves a link's, or else the first `Link` header field value with
//! `rel="canonical"`, resolved against the URL it was fetched at.

use goodfetch_counted_work::{CountedWork, Row};
use url::Url;

/// The URL `html`'s head, or else its `headers` (names lower-cased), name
/// as its canonical; `None` when they name none that parses.
pub(crate) fn canonical(
    html: &str,
    headers: &[(String, String)],
    base: &Url,
    url: &Url,
    work: &CountedWork,
) -> Option<Url> {
    let document = work.parse_html(Row::PageParses, html);
    let in_head = document
        .select("head link[rel][href]")
        .nodes()
        .iter()
        .find(|link| link.attr("rel").is_some_and(|rel| has_canonical(&rel)))
        .and_then(|link| link.attr("href"))
        .and_then(|href| base.join(href.trim()).ok());
    in_head.or_else(|| {
        headers
            .iter()
            .filter(|(name, _)| name == "link")
            .flat_map(|(_, value)| links(value))
            .find_map(|(target, rel)| has_canonical(rel).then(|| url.join(target).ok()).flatten())
    })
}

/// Whether the `rel` value `rel`, a list of link types, holds
/// `canonical`, in any case.
fn has_canonical(rel: &str) -> bool {
    rel.split_ascii_whitespace()
        .any(|kind| kind.eq_ignore_ascii_case("canonical"))
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
