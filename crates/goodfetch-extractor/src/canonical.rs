//! A page's canonical link (ADR 0120): the one rule its readers share, the
//! formats' `metadata.canonical` and the fetch tier's evidence for the
//! Frontier alike, so the two never name different URLs for one page. A
//! page's canonical is the first `<link>` in its head whose `rel` holds
//! `canonical` and that has an `href`, resolved against its base URL as
//! the URL Standard parses a link: never one in its body.

use url::Url;

/// Whether the `rel` value `rel`, a list of link types split on ASCII
/// white space, holds `canonical`, in any case.
pub fn is_canonical_link(rel: &str) -> bool {
    rel.split_ascii_whitespace()
        .any(|kind| kind.eq_ignore_ascii_case("canonical"))
}

/// The URL the canonical link's `href` names, resolved against `base`;
/// `None` when it does not parse. The URL Standard strips the C0 controls
/// and spaces at either end, and nothing more.
pub fn canonical_url(href: &str, base: &Url) -> Option<Url> {
    base.join(href).ok()
}
