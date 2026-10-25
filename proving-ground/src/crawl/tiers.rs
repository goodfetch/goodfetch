//! The `tiers` family on `tiers.test` (ADRs 0020 and 0093; CO-15; no
//! task): one host whose pages need a browser on some paths and not on
//! others, so a crawl learns a tier per Label and per path segment.
//!
//! - `/products/<n>`: a static listing, its products in its own markup as
//!   `li.product`, the same on every tier.
//! - `/blog/<n>`: a script-rendered listing, its posts added by its script
//!   as `li.item`, so only a browser sees them.
//! - `/mixed/<n>`: static for an odd `n`, script-rendered for an even one.
//! - `/states`: a static page whose one `h1` is visible and whose other
//!   elements are hidden by their own markup (a `hidden` attribute, an
//!   ancestor's inline `display: none`, an inline `visibility:hidden`),
//!   with a link to `/products/2`, so a document tier answers questions
//!   and waits from markup (spec #582's served-call table).
//!
//! `n` runs from 1 to 99, written without leading zeros; each listing holds
//! three entries named from the seed and `n`.

use std::fmt::Write as _;

use super::{Page, Visit, canonical_number, document, name};

pub(super) const HOST: &str = "tiers.test";
pub(super) const FAMILY: &str = "tiers";

/// The generator place of the family's values.
const FAMILY_PLACE: u64 = 40;

/// The highest page number of each path.
const PAGES: u64 = 99;

/// The entries of each listing.
const ENTRIES: u64 = 3;

/// The script that adds a script-rendered listing's entries, from the
/// names its `data-names` holds.
const SCRIPT: &str = r#"const list = document.querySelector(".items");
for (const name of list.dataset.names.split("|")) {
  const entry = document.createElement("li");
  entry.className = "item";
  entry.textContent = name;
  list.append(entry);
}
"#;

/// The states page's body: one visible heading, elements hidden by their
/// own markup in each way a document tier reads, and a link to a static
/// listing.
const STATES: &str = r#"<h1>states</h1>
<p id="hidden-attribute" hidden>hidden by its attribute</p>
<div style="color: gray; display : none"><span id="in-display-none">inside display:none</span></div>
<p id="visibility-hidden" style="visibility:hidden">hidden by its visibility</p>
<p><a id="next" href="/products/2">products 2</a></p>
"#;

pub(super) fn respond(visit: &Visit<'_>) -> Page {
    let path = visit.request.path;
    if path == "/states" {
        return Page::ok(document("states", STATES));
    }
    let scripted = |kind: &str, number: u64| match kind {
        "products" => Some(false),
        "blog" => Some(true),
        "mixed" => Some(number.is_multiple_of(2)),
        _ => None,
    };
    let Some((kind, number)) = path
        .strip_prefix('/')
        .and_then(|rest| rest.split_once('/'))
        .and_then(|(kind, number)| Some((kind, canonical_number(number)?)))
        .filter(|(_, number)| (1..=PAGES).contains(number))
    else {
        return Page::not_found();
    };
    let Some(scripted) = scripted(kind, number) else {
        return Page::not_found();
    };
    let names: Vec<String> = (1..=ENTRIES)
        .map(|entry| name(visit.seed, &[FAMILY_PLACE, number, entry]))
        .collect();
    let title = format!("{kind} {number}");
    Page::ok(if scripted {
        document(
            &title,
            &format!(
                "<h1>{title}</h1>\n<ul class=\"items\" data-names=\"{}\"></ul>\n<script>\n{SCRIPT}</script>\n",
                names.join("|")
            ),
        )
    } else {
        let mut entries = String::new();
        for name in &names {
            let _ = writeln!(entries, "<li class=\"product\">{name}</li>");
        }
        document(
            &title,
            &format!("<h1>{title}</h1>\n<ul class=\"products\">\n{entries}</ul>\n"),
        )
    })
}
