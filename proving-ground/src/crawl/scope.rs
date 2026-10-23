//! The `scope` family (pitfall U15): one site on `scope.test` with a `www`
//! host and a subdomain, beside a sister domain and an off-site host, so a
//! crawl's scope is judged by the hosts the Proving ground received.
//!
//! - `scope.test`'s `/` links, by absolute URLs on the port the request
//!   named, a page on its own host (`/about`), on its `www` host
//!   (`www.scope.test/news`), on its subdomain (`shop.scope.test/cart`),
//!   on its own host over `https` (`/account`, the one address serving TLS
//!   too), on the sister domain (`scope-sister.test/sister`) and off site
//!   (`elsewhere.test/offsite`).
//! - `www.scope.test/news` links back to `scope.test/about` and to
//!   `shop.scope.test/cart`.
//! - `scope.test/moved`, linked from nowhere, redirects (`302`) off site to
//!   `elsewhere.test/offsite`, for a crawl's redirect leaving its scope.
//! - `scope.test/cached`, linked from nowhere, may be kept by a browser's
//!   cache for an hour (`Cache-Control: max-age=3600`), so a second load
//!   the cache serves never reaches the Proving ground: the interception's
//!   cases read it (ADR 0085).
//!
//! Every other page is an article with its ID; any other path is `404`.
//! No task names the family: the engine's scope cases read it.

use std::fmt::Write as _;

use hyper::StatusCode;

use super::{Page, Visit, document, name};

pub(super) const FAMILY: &str = "scope";

/// The site's host, the family's host.
pub(super) const HOST: &str = "scope.test";

/// Every host name of the family.
pub(super) const HOSTS: [&str; 5] = [
    HOST,
    "www.scope.test",
    "shop.scope.test",
    "scope-sister.test",
    "elsewhere.test",
];

/// The generator place of the family's values.
const FAMILY_PLACE: u64 = 33;

/// The generator place of the cached page's title, after the pages'.
const CACHED_PLACE: u64 = 7;

/// Each page but the index: its scheme, host and path, and its ID.
const PAGES: [(&str, &str, &str, &str); 6] = [
    ("http", HOST, "/about", "S-ABOUT"),
    ("http", "www.scope.test", "/news", "S-NEWS"),
    ("http", "shop.scope.test", "/cart", "S-CART"),
    ("https", HOST, "/account", "S-ACCOUNT"),
    ("http", "scope-sister.test", "/sister", "S-SISTER"),
    ("http", "elsewhere.test", "/offsite", "S-OFFSITE"),
];

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
    let link = |scheme: &str, host: &str, path: &str| format!("{scheme}://{host}{port}{path}");
    match (request.host, request.path) {
        (HOST, "/") => Page::ok(index(
            &PAGES
                .iter()
                .map(|(scheme, host, path, _)| link(scheme, host, path))
                .collect::<Vec<_>>(),
        )),
        (HOST, "/moved") => Page::html(
            StatusCode::FOUND,
            document("Moved", "<p>This page has moved.</p>\n"),
        )
        .header("location", link("http", "elsewhere.test", "/offsite")),
        (HOST, "/cached") => {
            let title = name(visit.seed, &[FAMILY_PLACE, CACHED_PLACE]);
            Page::ok(article("S-CACHED", &title, &[]))
                .header("cache-control", "max-age=3600".to_owned())
        }
        (host, path) => PAGES
            .iter()
            .zip(1_u64..)
            .find(|(page, _)| page.1 == host && page.2 == path)
            .map_or_else(Page::not_found, |((_, _, _, id), place)| {
                let links = if *id == "S-NEWS" {
                    vec![
                        link("http", HOST, "/about"),
                        link("http", "shop.scope.test", "/cart"),
                    ]
                } else {
                    Vec::new()
                };
                let title = name(visit.seed, &[FAMILY_PLACE, place]);
                Page::ok(article(id, &title, &links))
            }),
    }
}

/// The site's index, linking `links`.
fn index(links: &[String]) -> String {
    let mut body = String::from("<main>\n<h1>Scope</h1>\n<ul class=\"links\">\n");
    for link in links {
        writeln!(body, "<li><a href=\"{link}\">{link}</a></li>")
            .expect("invariant: writing to a String succeeds");
    }
    body.push_str("</ul>\n</main>\n");
    document("Scope", &body)
}

/// A page with its ID and title, linking `links`.
fn article(id: &str, title: &str, links: &[String]) -> String {
    let mut body = format!(
        "<main>\n<article class=\"entry\" data-id=\"{id}\">\n<h1 class=\"title\">{title}</h1>\n"
    );
    for link in links {
        writeln!(body, "<p><a href=\"{link}\">{link}</a></p>")
            .expect("invariant: writing to a String succeeds");
    }
    body.push_str("</article>\n</main>\n");
    document(title, &body)
}
