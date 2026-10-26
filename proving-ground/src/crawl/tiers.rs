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
//! - `/routes`: a hash-routed app (pitfall U05): its script writes the
//!   view its URL's fragment names, `#/a` as `view a`, into its `h1.view`,
//!   and again on each `hashchange`, with links to `#/a` and `#/b`; its
//!   markup holds no view, so only a browser sees one, and every fragment
//!   is the same document to the server.
//! - `/linked/<n>`: a listing whose three links, to `/products/<3n-2>`
//!   to `/products/<3n>`, only its script adds (from its `data-targets`),
//!   so a document tier finds none (the sample comparison's cases, spec
//!   #579).
//! - `/moving/<n>`: a page whose script moves the browser to
//!   `/products/<n>` at once (`location.replace`), a redirect only a
//!   browser follows (ADR 0118).
//! - `/pictured/<n>`: a static page that loads an image,
//!   `/images/<n>.gif`, and a font, `/fonts/tiers-<n>.woff2`, both `404`,
//!   so a test sees whether a browser asked for them.
//! - `/painted/<n>`: a page whose script paints a canvas black and writes
//!   the alpha of its first pixel into its `h1.painted`, `alpha 255`
//!   where the browser paints and `alpha 0` in a browser without pixels,
//!   such as Lightpanda's light engine, so the light-browser tier's sample
//!   differs from Chrome's there (ADR 0020).
//!
//! `n` runs from 1 to 99, written without leading zeros; each listing holds
//! three entries named from the seed and `n`. A query `hold=<ms>`, from 1
//! to [`MAX_HOLD_MS`], holds the answer that long and names the hold in
//! `Server-Timing: hold;dur=<ms>`, so a fetch on one tier is still in
//! flight when a fetch on another starts (`fetch.tier_race`, CO-16).

use std::fmt::Write as _;
use std::time::Duration;

use super::{Held, Page, Visit, canonical_number, document, name};

pub(super) const HOST: &str = "tiers.test";
pub(super) const FAMILY: &str = "tiers";

/// The generator place of the family's values.
const FAMILY_PLACE: u64 = 40;

/// The highest page number of each path.
const PAGES: u64 = 99;

/// The entries of each listing.
const ENTRIES: u64 = 3;

/// The longest hold a query asks for that is honoured.
const MAX_HOLD_MS: u64 = 5_000;

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

/// The hash-routed app's body: an empty view its script fills from the
/// URL's fragment, and links to two routes.
const ROUTES: &str = r##"<h1 class="view"></h1>
<p><a href="#/a">a</a> <a href="#/b">b</a></p>
<script>
const show = () => {
  document.querySelector(".view").textContent = "view " + location.hash.replace(/^#\//, "");
};
show();
addEventListener("hashchange", show);
</script>
"##;

/// The script that adds a script-linked listing's links, from the paths
/// its `data-targets` holds, so its markup holds none.
const LINKS_SCRIPT: &str = r#"const list = document.querySelector(".links");
for (const target of list.dataset.targets.split("|")) {
  const entry = document.createElement("li");
  const link = document.createElement("a");
  link.setAttribute("href", target);
  link.textContent = target;
  entry.append(link);
  list.append(entry);
}
"#;

/// The script that writes the alpha of a canvas's first pixel once it is
/// painted black: 255 where pixels are painted, 0 where they are not.
const PAINT_SCRIPT: &str = r##"const canvas = document.createElement("canvas");
canvas.width = 2;
canvas.height = 2;
const context = canvas.getContext("2d");
context.fillStyle = "#000";
context.fillRect(0, 0, 2, 2);
document.querySelector(".painted").textContent = "alpha " + context.getImageData(0, 0, 1, 1).data[3];
"##;

/// The pages of the sample comparison's cases (spec #579): a listing whose
/// links only its script adds, one that a script moves to a static
/// listing at once, a static listing that loads an image and a font, and
/// one whose text only painted pixels give.
fn sample_case(kind: &str, number: u64) -> Option<Page> {
    let title = format!("{kind} {number}");
    let body = match kind {
        "linked" => {
            let targets: Vec<String> = (1..=ENTRIES)
                .map(|entry| format!("/products/{}", (number - 1) * ENTRIES + entry))
                .collect();
            format!(
                "<h1>{title}</h1>\n<ul class=\"links\" data-targets=\"{}\"></ul>\n<script>\n{LINKS_SCRIPT}</script>\n",
                targets.join("|")
            )
        }
        "moving" => format!(
            "<script>location.replace(\"/products/{number}\");</script>\n<h1>{title}</h1>\n"
        ),
        "pictured" => format!(
            "<style>@font-face {{ font-family: \"tiers\"; src: url(\"/fonts/tiers-{number}.woff2\"); }} h1 {{ font-family: \"tiers\"; }}</style>\n<h1>{title}</h1>\n<img src=\"/images/{number}.gif\" alt=\"{title}\">\n"
        ),
        "painted" => format!(
            "<h1>{title}</h1>\n<p class=\"painted\"></p>\n<script>\n{PAINT_SCRIPT}</script>\n"
        ),
        _ => return None,
    };
    Some(Page::ok(document(&title, &body)))
}

pub(super) fn respond(visit: &Visit<'_>) -> Page {
    let path = visit.request.path;
    if path == "/states" {
        return Page::ok(document("states", STATES));
    }
    if path == "/routes" {
        return Page::ok(document("routes", ROUTES));
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
    if let Some(page) = sample_case(kind, number) {
        return page;
    }
    let Some(scripted) = scripted(kind, number) else {
        return Page::not_found();
    };
    let names: Vec<String> = (1..=ENTRIES)
        .map(|entry| name(visit.seed, &[FAMILY_PLACE, number, entry]))
        .collect();
    let title = format!("{kind} {number}");
    let page = Page::ok(if scripted {
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
    });
    let hold = visit
        .request
        .parameter("hold")
        .and_then(|hold| hold.parse::<u64>().ok())
        .filter(|hold| (1..=MAX_HOLD_MS).contains(hold));
    match hold {
        None => page,
        Some(hold) => Page {
            held: Some(Held::after(Duration::from_millis(hold))),
            ..page.header("server-timing", format!("hold;dur={hold}"))
        },
    }
}
