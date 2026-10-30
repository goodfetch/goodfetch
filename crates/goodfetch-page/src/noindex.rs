//! Whether a document is marked `noindex` (ADR 0035): a `noindex` or
//! `none` directive in its `X-Robots-Tag` header or in a robots `meta`
//! element's `content`, which the Page check reports with a failure, and
//! which a crawl reads as a decoy. A value that names a user agent first,
//! such as `googlebot: nofollow, noindex`, speaks to that crawler alone, in
//! full, and is not read.

use std::collections::BTreeMap;

/// The header that carries a response's robots directives, lower-cased as
/// a response's headers are.
const HEADER: &str = "x-robots-tag";

/// Whether `headers`, a response's, names lower-cased, or `robots`, the
/// `content` of each robots `meta` element of its document, mark it
/// `noindex`.
#[must_use]
pub fn marked(headers: &BTreeMap<String, String>, robots: &[String]) -> bool {
    headers
        .get(HEADER)
        .is_some_and(|value| value.lines().any(says_noindex))
        || robots.iter().any(|content| says_noindex(content))
}

/// The robots directives that carry a value after a `:`, which are no
/// user agent's name.
const VALUED: [&str; 4] = [
    "max-image-preview",
    "max-snippet",
    "max-video-preview",
    "unavailable_after",
];

/// Whether the robots directives `directives`, comma-separated, say
/// `noindex` or `none` to every crawler. A value that begins with a user
/// agent's name and a `:`, as in `googlebot: nofollow, noindex`, speaks to
/// that crawler in full, so none of it is read.
fn says_noindex(directives: &str) -> bool {
    let named = directives.split_once(':').is_some_and(|(first, _)| {
        let first = first.trim();
        !first.contains(',')
            && !VALUED
                .iter()
                .any(|valued| valued.eq_ignore_ascii_case(first))
    });
    !named
        && directives.split(',').any(|directive| {
            let directive = directive.trim();
            !directive.contains(':')
                && (directive.eq_ignore_ascii_case("noindex")
                    || directive.eq_ignore_ascii_case("none"))
        })
}

#[cfg(test)]
mod tests;
