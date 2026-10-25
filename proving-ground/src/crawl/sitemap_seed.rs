//! The `sitemap-seed` family (ADRs 0093 and 0120; pitfall U20): a site on
//! `sitemap-seed.test` whose sitemaps a crawl's sitemap seed reads, beside
//! a partner host its robots.txt cross-submits a sitemap on and a third
//! host a domain policy can refuse, so a crawl is judged by the requests
//! the Proving ground received. Sitemaps name absolute URLs on the port the
//! request named.
//!
//! - `sitemap-seed.test/robots.txt` disallows `/private/` for every
//!   crawler and names one sitemap, `sitemap-partner.test/cross.xml`.
//! - `/sitemap-index.xml` is an index of two sitemaps: `/sitemaps/books.xml`
//!   and, relative, `sitemaps/more.xml`.
//! - `/sitemaps/books.xml` lists `/books/1`, `/books/2`, `/authors/1`,
//!   `/about` and the partner's `/books/9`; `/sitemaps/more.xml` lists
//!   `../books/3` (relative to itself), `/books/1` again and `/authors/2`.
//! - `/polite-index.xml` is an index of `/sitemaps/more.xml`, of
//!   `/private/hidden.xml`, which robots.txt disallows and lists
//!   `/books/99`, and of `sitemap-blocked.test/sitemap.xml`, which lists
//!   `/books/7`.
//! - `sitemap-partner.test/cross.xml` lists `sitemap-seed.test/books/4`
//!   and the partner's own `/books/5`; the partner has no robots.txt.
//! - Every `/books/<n>`, `/authors/<n>` and `/about` is an article with its
//!   ID, on either host; any other path is `404`.
//!
//! No task names the family: the engine's sitemap seed cases read it.

use std::fmt::Write as _;

use super::{Page, Visit, canonical_number, document, name};

pub(super) const FAMILY: &str = "sitemap-seed";

/// The site's host, the family's host.
pub(super) const HOST: &str = "sitemap-seed.test";

/// The partner host the site's robots.txt cross-submits a sitemap on.
const PARTNER: &str = "sitemap-partner.test";

/// The host a crawl's domain policy can refuse.
const BLOCKED: &str = "sitemap-blocked.test";

/// Every host name of the family.
pub(super) const HOSTS: [&str; 3] = [HOST, PARTNER, BLOCKED];

/// The generator place of the family's values.
const FAMILY_PLACE: u64 = 34;

/// Whether `host` is one of the family's.
pub(super) fn serves(host: &str) -> bool {
    HOSTS.contains(&host)
}

pub(super) fn respond(visit: &Visit<'_>) -> Page {
    let request = visit.request;
    let port = request
        .authority
        .rsplit_once(':')
        .map(|(_, port)| format!(":{port}"))
        .unwrap_or_default();
    let on = |host: &str, path: &str| format!("http://{host}{port}{path}");
    match (request.host, request.path) {
        (HOST, "/robots.txt") => Page::text(
            "text/plain; charset=utf-8",
            format!(
                "User-agent: *\nDisallow: /private/\n\nSitemap: {}\n",
                on(PARTNER, "/cross.xml")
            ),
        ),
        (HOST, "/sitemap-index.xml") => index(&[
            on(HOST, "/sitemaps/books.xml"),
            "sitemaps/more.xml".to_owned(),
        ]),
        (HOST, "/sitemaps/books.xml") => urlset(&[
            on(HOST, "/books/1"),
            on(HOST, "/books/2"),
            on(HOST, "/authors/1"),
            on(HOST, "/about"),
            on(PARTNER, "/books/9"),
        ]),
        (HOST, "/sitemaps/more.xml") => urlset(&[
            "../books/3".to_owned(),
            on(HOST, "/books/1"),
            on(HOST, "/authors/2"),
        ]),
        (HOST, "/polite-index.xml") => index(&[
            on(HOST, "/sitemaps/more.xml"),
            on(HOST, "/private/hidden.xml"),
            on(BLOCKED, "/sitemap.xml"),
        ]),
        (HOST, "/private/hidden.xml") => urlset(&[on(HOST, "/books/99")]),
        (PARTNER, "/cross.xml") => urlset(&[on(HOST, "/books/4"), on(PARTNER, "/books/5")]),
        (BLOCKED, "/sitemap.xml") => urlset(&[on(HOST, "/books/7")]),
        (_, path) => page(visit.seed, path),
    }
}

/// An article for `/books/<n>`, `/authors/<n>` or `/about`, else `404`.
fn page(seed: u64, path: &str) -> Page {
    let numbered = |prefix: &str| path.strip_prefix(prefix).and_then(canonical_number);
    let (id, place) = if path == "/about" {
        ("SS-ABOUT".to_owned(), 0)
    } else if let Some(number) = numbered("/books/") {
        (format!("SS-BOOK-{number}"), 2 * number + 1)
    } else if let Some(number) = numbered("/authors/") {
        (format!("SS-AUTHOR-{number}"), 2 * number + 2)
    } else {
        return Page::not_found();
    };
    let title = name(seed, &[FAMILY_PLACE, place]);
    Page::ok(document(
        &title,
        &format!(
            "<main>\n<article class=\"entry\" data-id=\"{id}\">\n<h1 class=\"title\">{title}</h1>\n</article>\n</main>\n"
        ),
    ))
}

fn xml(text: String) -> Page {
    Page::text("application/xml", text)
}

/// A sitemap index of `sitemaps`, written as given.
fn index(sitemaps: &[String]) -> Page {
    let mut text = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<sitemapindex xmlns=\"http://www.sitemaps.org/schemas/sitemap/0.9\">\n",
    );
    for sitemap in sitemaps {
        writeln!(text, "<sitemap><loc>{sitemap}</loc></sitemap>")
            .expect("invariant: writing to a String succeeds");
    }
    text.push_str("</sitemapindex>\n");
    xml(text)
}

/// A sitemap of `entries`, written as given.
fn urlset(entries: &[String]) -> Page {
    let mut text = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<urlset xmlns=\"http://www.sitemaps.org/schemas/sitemap/0.9\">\n",
    );
    for entry in entries {
        writeln!(text, "<url><loc>{entry}</loc></url>")
            .expect("invariant: writing to a String succeeds");
    }
    text.push_str("</urlset>\n");
    xml(text)
}
