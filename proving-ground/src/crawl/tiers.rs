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
//! - `/scroll/<n>`, `/cursor/<n>` and `/signed/<n>`: infinite lists, each
//!   a shell whose script loads its items from a JSON API under `/api/`
//!   (the discovered-API cases, spec #579): by page number, by the cursor
//!   each answer names, or once with a token its document issued, which
//!   the API takes only once.
//!
//! `n` runs from 1 to 99, written without leading zeros; each listing holds
//! three entries named from the seed and `n`. A query `hold=<ms>`, from 1
//! to [`MAX_HOLD_MS`], holds the answer that long and names the hold in
//! `Server-Timing: hold;dur=<ms>`, so a fetch on one tier is still in
//! flight when a fetch on another starts (`fetch.tier_race`, CO-16).

use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::time::Duration;

use hyper::StatusCode;

use super::{Held, Page, Visit, canonical_number, document, name};
use crate::generator;
use crate::sync::Mutex;

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

/// The pages of each infinite list.
const LIST_PAGES: u64 = 3;

/// The items of each page of an infinite list.
const LIST_ITEMS: u64 = 2;

/// What each infinite list's script starts with: `load(url)`, which reads
/// one JSON call and adds its items to the list. The calls are synchronous,
/// so every one ended before the page's `load` event, and a tool handed
/// the page after it sees no call of its first load still in flight.
const LIST_LOADER: &str = r#"const feed = document.querySelector(".feed");
const load = (url) => {
  const call = new XMLHttpRequest();
  call.open("GET", url, false);
  call.send();
  const answer = JSON.parse(call.responseText);
  for (const item of answer.items) {
    const entry = document.createElement("li");
    entry.className = "item";
    entry.textContent = item;
    feed.append(entry);
  }
  return answer;
};
"#;

/// The script of `/scroll/<n>`: it loads its list's pages, page after
/// page, by their number, until its API names no next page.
const SCROLL_SCRIPT: &str = r"for (let page = 1; page !== null; ) {
  page = load(`/api/scroll/${feed.dataset.number}?page=${page}`).next;
}
";

/// The script of `/cursor/<n>`: it loads its list's pages, each from the
/// cursor the page before named, until one names none.
const CURSOR_SCRIPT: &str = r"for (let url = `/api/cursor/${feed.dataset.number}`; url !== null; ) {
  const next = load(url).page.next;
  url = next === null ? null : `/api/cursor/${feed.dataset.number}?after=${next}`;
}
";

/// The script of `/signed/<n>`: it loads its list once, with the token
/// its document carries.
const SIGNED_SCRIPT: &str = r"load(`/api/signed/${feed.dataset.number}?token=${feed.dataset.token}`);
";

/// What the family keeps for one `serve`: the tokens its signed pages
/// issued, and those already taken.
pub(super) struct State {
    tokens: Mutex<Tokens>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            tokens: Mutex::new(Tokens::default()),
        }
    }
}

#[derive(Default)]
struct Tokens {
    issued: u64,
    taken: BTreeSet<u64>,
}

/// The place of an infinite list's values of `kind`.
fn list_place(kind: &str) -> u64 {
    match kind {
        "scroll" => 1,
        "cursor" => 2,
        _ => 3,
    }
}

/// The items of page `page` of the infinite list `kind` number `number`.
fn list_items(seed: u64, kind: &str, number: u64, page: u64) -> Vec<String> {
    (1..=LIST_ITEMS)
        .map(|item| name(seed, &[FAMILY_PLACE, list_place(kind), number, page, item]))
        .collect()
}

/// The cursor that names page `page` of the cursor list `number`.
fn cursor(seed: u64, number: u64, page: u64) -> String {
    format!(
        "{:016x}",
        generator::draw(seed, &[FAMILY_PLACE, 4, number, page])
    )
}

/// The infinite lists (spec #579's discovered-API cases): a shell page
/// whose script loads every item from its JSON API.
fn list_shell(state: &State, kind: &str, number: u64) -> Option<Page> {
    let (script, extra) = match kind {
        "scroll" => (SCROLL_SCRIPT, String::new()),
        "cursor" => (CURSOR_SCRIPT, String::new()),
        "signed" => {
            let mut tokens = state.tokens.lock();
            tokens.issued += 1;
            (SIGNED_SCRIPT, format!(" data-token=\"{}\"", tokens.issued))
        }
        _ => return None,
    };
    let title = format!("{kind} {number}");
    Some(Page::ok(document(
        &title,
        &format!(
            "<h1>{title}</h1>\n<ul class=\"feed\" data-number=\"{number}\"{extra}></ul>\n<script>\n{LIST_LOADER}{script}</script>\n"
        ),
    )))
}

/// The JSON API of the infinite lists, at `/api/<kind>/<number>`.
fn list_api(visit: &Visit<'_>, state: &State, kind: &str, number: u64) -> Option<Page> {
    let seed = visit.seed;
    let json = |value: serde_json::Value| Page::text("application/json", value.to_string());
    Some(match kind {
        "scroll" => {
            let page = visit
                .request
                .parameter("page")
                .and_then(canonical_number)
                .unwrap_or(1);
            if (1..=LIST_PAGES).contains(&page) {
                let next = (page < LIST_PAGES).then_some(page + 1);
                json(
                    serde_json::json!({ "items": list_items(seed, kind, number, page), "next": next }),
                )
            } else {
                json(serde_json::json!({ "items": [], "next": null }))
            }
        }
        "cursor" => {
            let page = match visit.request.parameter("after") {
                None => Some(1),
                Some(after) => (2..=LIST_PAGES).find(|page| cursor(seed, number, *page) == after),
            };
            let Some(page) = page else {
                return Some(Page {
                    status: StatusCode::BAD_REQUEST,
                    ..json(serde_json::json!({ "error": "unknown cursor" }))
                });
            };
            let next = (page < LIST_PAGES).then(|| cursor(seed, number, page + 1));
            json(
                serde_json::json!({ "items": list_items(seed, kind, number, page), "page": { "next": next } }),
            )
        }
        "signed" => {
            let token = visit.request.parameter("token").and_then(canonical_number);
            let mut tokens = state.tokens.lock();
            match token {
                Some(token) if token <= tokens.issued && tokens.taken.insert(token) => {
                    json(serde_json::json!({ "items": list_items(seed, kind, number, 1) }))
                }
                _ => Page {
                    status: StatusCode::FORBIDDEN,
                    ..json(
                        serde_json::json!({ "error": "this token was never issued, or was taken" }),
                    )
                },
            }
        }
        _ => return None,
    })
}

pub(super) fn respond(visit: &Visit<'_>, state: &State) -> Page {
    let path = visit.request.path;
    if let Some((kind, number)) = path
        .strip_prefix("/api/")
        .and_then(|rest| rest.split_once('/'))
        .and_then(|(kind, number)| Some((kind, canonical_number(number)?)))
        .filter(|(_, number)| (1..=PAGES).contains(number))
    {
        return list_api(visit, state, kind, number).unwrap_or_else(Page::not_found);
    }
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
    if let Some(page) = sample_case(kind, number).or_else(|| list_shell(state, kind, number)) {
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
