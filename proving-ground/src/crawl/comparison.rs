//! The comparison's own sites (ADR 0138's matrix, #1903), beside
//! `scale.test`'s: the same tree of pages as `scale.test/sites/<size>/`,
//! of any size up to the `scale` family's item count, on host names under
//! `localhost`, which a browser and the system's resolver both take to
//! loopback, so every tool compared reaches them with no resolver of its
//! own.
//!
//! - The multi-domain site, for the polite scenario: page `n` of
//!   `/sites/<size>/<n>` lives on `polite-<d>.localhost`, `d` being `n`
//!   modulo [`DOMAINS`] (from 1), each its own registrable domain, so a
//!   crawl that waits between requests to one domain can still keep many
//!   domains busy. Every link is an absolute URL on the page's host and the
//!   port the request named. Each host's `/robots.txt` disallows
//!   `/sites/*/private`, which every page links on its own host, so a crawl
//!   that does not obey it fetches that page, which is no page of the site.
//!   A page asked for on another domain's host is `404`.
//! - The script-rendered site, for the headless browser scenario, on
//!   `rendered.localhost`: each page's links to its children are added by
//!   its script from the first and last child its list names, so its markup
//!   holds no link but home's, and a crawl that runs no script finds no
//!   page past the first.
//! - The realistic-page site, for #2316's scenario of pages of realistic
//!   size, on `realistic.localhost`: see [`realistic`].
//!
//! No task names them, and none declares a Feature.

mod realistic;

use super::scale::{Links, site_place, tree_page};
use super::{Page, Visit, document};

pub(super) const FAMILY: &str = "comparison";

/// The domains of the multi-domain site (#2283): ten pages on each at
/// the polite scenario's 10,000.
const DOMAINS: u64 = 1000;

/// The script-rendered site's host.
const RENDERED_HOST: &str = "rendered.localhost";

/// The multi-domain site's robots.txt: every path allowed but the private
/// page each page links.
const ROBOTS: &str = "User-agent: *\nDisallow: /sites/*/private\n";

/// The script that adds a script-rendered page's links to its children,
/// from the first and last its list names, under the site's size its URL
/// names.
const LINKS_SCRIPT: &str = r#"const list = document.querySelector(".pages");
const size = location.pathname.split("/")[2];
for (let child = Number(list.dataset.first); child <= Number(list.dataset.last); child += 1) {
  const entry = document.createElement("li");
  const link = document.createElement("a");
  link.setAttribute("href", "/sites/" + size + "/" + child);
  link.textContent = "Page " + child;
  entry.append(link);
  list.append(entry);
}
"#;

/// The host of the multi-domain site's domain `domain`, from 1.
fn polite_host(domain: u64) -> String {
    format!("polite-{domain}.localhost")
}

/// The multi-domain site's domain that holds page `page`.
fn domain_of(page: u64) -> u64 {
    (page - 1) % DOMAINS + 1
}

/// The domain whose host `host` is, if it is one of the multi-domain
/// site's.
fn polite_domain(host: &str) -> Option<u64> {
    let domain: u64 = host
        .strip_prefix("polite-")?
        .strip_suffix(".localhost")?
        .parse()
        .ok()?;
    (polite_host(domain) == host && (1..=DOMAINS).contains(&domain)).then_some(domain)
}

/// Whether `host` is one of the family's.
pub(super) fn serves(host: &str) -> bool {
    host == RENDERED_HOST || host == realistic::HOST || polite_domain(host).is_some()
}

/// Every host name of the family.
pub(super) fn hosts() -> impl Iterator<Item = String> {
    (1..=DOMAINS)
        .map(polite_host)
        .chain([RENDERED_HOST.to_owned(), realistic::HOST.to_owned()])
}

pub(super) fn respond(visit: &Visit<'_>) -> Page {
    let request = visit.request;
    if request.host == realistic::HOST {
        return realistic::respond(visit.seed, request.path);
    }
    let rest = request.path.strip_prefix("/sites/");
    if request.host == RENDERED_HOST {
        return match rest.and_then(site_place) {
            Some((size, page)) => Page::ok(tree_page(
                visit.seed,
                size,
                page,
                &Links::Script(LINKS_SCRIPT),
            )),
            None => Page::not_found(),
        };
    }
    let Some(domain) = polite_domain(request.host) else {
        return Page::not_found();
    };
    if request.path == "/robots.txt" {
        return Page::text("text/plain; charset=utf-8", ROBOTS.to_owned());
    }
    if let Some(size) = rest.and_then(|rest| rest.strip_suffix("/private")) {
        return if site_place(&format!("{size}/1")).is_some() {
            Page::ok(document("Private", "<main>\n<h1>Private</h1>\n</main>\n"))
        } else {
            Page::not_found()
        };
    }
    match rest.and_then(site_place) {
        Some((size, page)) if domain_of(page) == domain => {
            let port = request
                .authority
                .rsplit_once(':')
                .map_or("80", |(_, port)| port);
            let private = format!("/sites/{size}/private");
            let host = |page| polite_host(domain_of(page));
            Page::ok(tree_page(
                visit.seed,
                size,
                page,
                &Links::Hosts {
                    host: &host,
                    port,
                    private: &private,
                },
            ))
        }
        _ => Page::not_found(),
    }
}
