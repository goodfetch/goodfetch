//! The rung site on `rungs.localhost` (ADR 0020; no task): pages of three
//! kinds, each needing one rung of the ladder `fetch.tier: auto` climbs,
//! apart by their paths' first segment, so a crawl's pages per tier can be
//! checked against the kinds.
//!
//! - `/static/<n>`, **static:** its Record in its own markup, the same to
//!   every client.
//! - `/identity/<n>`, **browser identity:** the same markup, answered only
//!   on a connection whose TLS fingerprint is the pinned browser's
//!   ([`PINNED_BROWSER_JA4`]); any other client, and any
//!   request over plain text, gets a `403` that holds no Record. No script
//!   is needed, so a request sent from inside that browser reads it.
//! - `/scripted/<n>`, **script-rendered:** an empty `article.item` its
//!   inline script fills from its `data-id` and `data-name`, so its Record
//!   exists only once the script ran.
//!
//! `n` runs from 1 to [`PAGES_PER_KIND`], written without leading zeros.
//! `/` is a static page that links every one of them and holds no Record.
//! `/manifest.json`, which no page links, is the site's manifest: every
//! page with its `path`, its `kind` and the `records` a right reading
//! gives, each a pure function of the seed.
//!
//! The host is under `localhost`, as the `comparison` family's are, so a
//! browser and the system's resolver both take it to loopback.

use std::fmt::Write as _;

use serde::Serialize;

use super::{Page, Visit, document, name};
use crate::client_hello::ClientHello;
use crate::task::Record;

pub(super) const HOST: &str = "rungs.localhost";
pub(super) const FAMILY: &str = "rungs";

/// The generator place of the family's values.
const FAMILY_PLACE: u64 = 41;

/// The pages of each kind: past `fetch.sample.count`'s 3, so a Label has
/// pages left once its samples chose its tier.
const PAGES_PER_KIND: u64 = 6;

/// The JA4s, without what a resumed handshake adds, of the Chrome for
/// Testing build Collie pins (156.0.8078.4, `goodfetch-collie`'s
/// `pins.json`), measured on the Proving ground: as Collie starts it, with
/// `--disable-field-trial-config`, and with the field-trial configuration
/// built into the binary, which adds one extension (`0x12e0`). JA4 sorts
/// the extensions, so Chrome's shuffling of them changes neither. They move
/// with the pin: the site's test of its browser-identity page fails when
/// the pinned build sends another.
const PINNED_BROWSER_JA4: [&str; 2] = [
    "t13d1517h2_8daaf6152771_cb7bf5808d99",
    "t13d1518h2_8daaf6152771_4980c97edce0",
];

/// Whether a connection's TLS fingerprint is the pinned browser's.
fn of_the_pinned_browser(hello: &ClientHello) -> bool {
    PINNED_BROWSER_JA4.contains(&hello.ja4_base.as_str())
}

/// Where the manifest is served.
const MANIFEST: &str = "/manifest.json";

/// The script that fills each of a script-rendered page's items from its
/// own attributes.
const SCRIPT: &str = r#"for (const item of document.querySelectorAll(".item")) {
  const name = document.createElement("h1");
  name.className = "name";
  name.textContent = item.dataset.name;
  const id = document.createElement("p");
  id.className = "id";
  id.textContent = item.dataset.id;
  item.append(name, id);
}
"#;

/// What a page needs of the client that reads it.
#[derive(Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum Kind {
    /// Nothing: every client reads the same markup.
    Static,
    /// The pinned browser's TLS fingerprint.
    BrowserIdentity,
    /// Its script run.
    ScriptRendered,
}

impl Kind {
    const ALL: [Self; 3] = [Self::Static, Self::BrowserIdentity, Self::ScriptRendered];

    /// The first path segment of the kind's pages, which also opens their
    /// Records' IDs.
    fn segment(self) -> &'static str {
        match self {
            Self::Static => "static",
            Self::BrowserIdentity => "identity",
            Self::ScriptRendered => "scripted",
        }
    }

    /// The generator place of the kind's values.
    fn place(self) -> u64 {
        match self {
            Self::Static => 1,
            Self::BrowserIdentity => 2,
            Self::ScriptRendered => 3,
        }
    }
}

/// One page as the manifest lists it.
#[derive(Serialize)]
struct Listed {
    path: String,
    kind: Kind,
    /// The Records a right reading of the page gives.
    records: Vec<Record>,
}

/// The site's manifest.
#[derive(Serialize)]
struct Manifest {
    pages: Vec<Listed>,
}

/// Every page of the site: the home page, then each kind's.
fn pages(seed: u64) -> Vec<Listed> {
    let home = Listed {
        path: "/".to_owned(),
        kind: Kind::Static,
        records: Vec::new(),
    };
    let numbered = Kind::ALL.into_iter().flat_map(|kind| {
        (1..=PAGES_PER_KIND).map(move |number| {
            let segment = kind.segment();
            Listed {
                path: format!("/{segment}/{number}"),
                kind,
                records: vec![Record::from([
                    ("id", format!("{segment}-{number}")),
                    ("name", name(seed, &[FAMILY_PLACE, kind.place(), number])),
                ])],
            }
        })
    });
    std::iter::once(home).chain(numbered).collect()
}

/// The home page: a link to every other page.
fn home(pages: &[Listed]) -> String {
    let mut links = String::new();
    for page in pages.iter().filter(|page| page.path != "/") {
        let _ = writeln!(links, "<li><a href=\"{0}\">{0}</a></li>", page.path);
    }
    document(
        "Rungs",
        &format!("<h1>Rungs</h1>\n<ul class=\"pages\">\n{links}</ul>\n"),
    )
}

/// A page whose Records are in its markup.
fn in_markup(page: &Listed) -> String {
    let mut items = String::new();
    for record in &page.records {
        let _ = writeln!(
            items,
            "<article class=\"item\">\n<h1 class=\"name\">{}</h1>\n<p class=\"id\">{}</p>\n</article>",
            record["name"], record["id"]
        );
    }
    document(&page.path, &items)
}

/// A page whose Records only its script writes.
fn script_rendered(page: &Listed) -> String {
    let mut shells = String::new();
    for record in &page.records {
        let _ = writeln!(
            shells,
            "<article class=\"item\" data-id=\"{}\" data-name=\"{}\"></article>",
            record["id"], record["name"]
        );
    }
    document(
        &page.path,
        &format!("{shells}<script>\n{SCRIPT}</script>\n"),
    )
}

/// What a browser-identity page answers any client but the pinned browser.
fn refused() -> Page {
    Page::html(
        hyper::StatusCode::FORBIDDEN,
        document(
            "Access denied",
            "<h1>Access denied</h1>\n<p>This page answers only the browser it expects.</p>\n",
        ),
    )
}

pub(super) fn respond(visit: &Visit<'_>) -> Page {
    let pages = pages(visit.seed);
    let path = visit.request.path;
    if path == MANIFEST {
        let manifest = serde_json::to_string(&Manifest { pages })
            .expect("invariant: the manifest's own values serialize");
        return Page::text("application/json", manifest);
    }
    let Some(page) = pages.iter().find(|page| page.path == path) else {
        return Page::not_found();
    };
    if path == "/" {
        return Page::ok(home(&pages));
    }
    match page.kind {
        Kind::Static => Page::ok(in_markup(page)),
        Kind::BrowserIdentity => {
            if visit.request.tls.is_some_and(of_the_pinned_browser) {
                Page::ok(in_markup(page))
            } else {
                refused()
            }
        }
        Kind::ScriptRendered => Page::ok(script_rendered(page)),
    }
}
