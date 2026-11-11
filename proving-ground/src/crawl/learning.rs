//! The `learning` family on `learning.test` and `shop.learning.test` (ADR
//! 0005; spec #581's thresholds; no task): pages whose crawl teaches a
//! Frontier its rules, each threshold met on one path and one short of it
//! on another.
//!
//! - `/shop/<pairs>?n=<k>`: item `k`, the same page with or without
//!   `ref`. Item `k` up to `<pairs>` links itself with `&ref=<k>`, which
//!   links item `k + 1`, so a crawl fetches `<pairs>` pairs of URLs
//!   differing only by `ref`; the last pair links item 9 with `&ref=late`,
//!   which links item 9 without it, which links itself. Every link reads
//!   "More", so an item's text is the same under each of its URLs.
//! - `/stall` and `/stall?n=<k>`: a stall whose index links, in order,
//!   items 1 to 3, each without `ref` and then with `&ref=<k>`, three
//!   pairs that read alike, then items 7 and 8 the same way. Item 7 reads
//!   alike under both and links item 1 with `&ref=5`; item 8 reads
//!   otherwise with `ref`, a contradicting pair for a crawl that fetches
//!   both.
//! - `/calendar/<yyyy>/<mm>`: an events calendar whose months, all empty
//!   and alike, page on without end, each linking the next month and the
//!   home page; `/agenda/<yyyy>/<mm>` is the same for the 100 months from
//!   January 2026.
//! - `/feed?offset=<n>`: 200 rows paged 20 at a time by `offset`, each
//!   page linking the next offset and the cursor of the rows after it,
//!   `?after=r<n>`, which pages on by cursor alone; `/list?offset=<n>` is
//!   the same listing by offset alone, and `/pair?offset=<n>` holds two of
//!   its pages.
//! - `/forms/`: equivalent forms (ADR 0120), linking in order the pages
//!   below. `/forms/slash/<k>` and `/forms/slash/<k>/`, for `k` from 1 to
//!   3, are one guide, so two pairs teach the trailing slash's form and
//!   the third pair's slashed URL, still pending, is merged.
//!   `/forms/docs/<k>/index.html`, for `k` from 1 to 3, names its folder
//!   as its `rel=canonical` and links it, so three pages teach the
//!   `index.html` form. `/forms/news/<k>`, for `k` from 1 to 3, each name
//!   `/forms/` as their canonical, as a mis-templated site does, and are
//!   three pages all the same. `/forms/split/<k>`, for `k` from 1 to 3,
//!   reads otherwise on `www.learning.test` than on `learning.test`; the
//!   index links the first two on both hosts, and each of those links the
//!   next.
//! - `/`: the home page, which links nothing.

use std::fmt::Write as _;

use super::{Page, Visit, canonical_number, document};

pub(super) const HOST: &str = "learning.test";
/// A second host of the family, of the same registrable domain.
pub(super) const SHOP_HOST: &str = "shop.learning.test";
/// The `www.` host of the family's host, whose split pages read
/// otherwise.
pub(super) const WWW_HOST: &str = "www.learning.test";
pub(super) const FAMILY: &str = "learning";

/// The rows of a listing, and how many a page holds.
const ROWS: u64 = 200;
const PAGE_ROWS: u64 = 20;

/// The last month of the agenda, counted from January 2026.
const AGENDA_MONTHS: u64 = 100;

/// The item a shop's last pair links.
const LATE_ITEM: u64 = 9;

pub(super) fn respond(visit: &Visit<'_>) -> Page {
    let request = visit.request;
    let path = request.path;
    if path == "/" {
        return Page::ok(document(
            "Club",
            &article("Club", "Welcome to the club.", ""),
        ));
    }
    if let Some(form) = path.strip_prefix("/forms/") {
        return forms(form, request.host, request.authority).unwrap_or_else(Page::not_found);
    }
    if let Some(pairs) = path.strip_prefix("/shop/").and_then(canonical_number) {
        return shop(pairs, request.parameter("n"), request.parameter("ref"))
            .unwrap_or_else(Page::not_found);
    }
    if path == "/stall" {
        return stall(request.parameter("n"), request.parameter("ref"))
            .unwrap_or_else(Page::not_found);
    }
    for (prefix, last) in [("/calendar/", None), ("/agenda/", Some(AGENDA_MONTHS))] {
        if let Some(month) = path.strip_prefix(prefix) {
            return calendar(prefix, month, last).unwrap_or_else(Page::not_found);
        }
    }
    let listing = match path {
        "/feed" => Some((true, ROWS)),
        "/list" => Some((false, ROWS)),
        "/pair" => Some((false, 2 * PAGE_ROWS)),
        _ => None,
    };
    if let Some((cursor, rows)) = listing {
        let start = match (request.parameter("offset"), request.parameter("after")) {
            (Some(offset), None) => canonical_number(offset),
            (None, Some(after)) if cursor => after.strip_prefix('r').and_then(canonical_number),
            _ => None,
        };
        let by_cursor = request.parameter("after").is_some();
        return start
            .filter(|start| start % PAGE_ROWS == 0 && *start < rows)
            .map_or_else(Page::not_found, |start| {
                Page::ok(rows_page(path, start, rows, cursor, by_cursor))
            });
    }
    Page::not_found()
}

/// A page whose main content is a heading, `text` and the links `links`.
fn article(heading: &str, text: &str, links: &str) -> String {
    format!(
        "<main>\n<article>\n<h1>{heading}</h1>\n<p>{text} The club meets every weekend on \
         the field behind the old mill, whatever the wind, and this page lists what it \
         offers.</p>\n{links}</article>\n</main>\n"
    )
}

fn link(href: &str, text: &str) -> String {
    format!("<p><a href=\"{href}\">{text}</a></p>\n")
}

/// Item `n` of the shop of `pairs` pairs, as `ref` asks for it.
fn shop(pairs: u64, n: Option<&str>, reference: Option<&str>) -> Option<Page> {
    let number = canonical_number(n?)?;
    if !(1..=pairs).contains(&number) && number != LATE_ITEM {
        return None;
    }
    let base = format!("/shop/{pairs}?n=");
    // Every link reads alike, so the page's text is the same whichever
    // URL of it a crawl fetched.
    let target = match (number == LATE_ITEM, reference) {
        (false, None) => format!("{base}{number}&amp;ref={number}"),
        (false, Some(_)) if number < pairs => format!("{base}{}", number + 1),
        (false, Some(_)) => format!("{base}{LATE_ITEM}&amp;ref=late"),
        (true, _) => format!("{base}{LATE_ITEM}"),
    };
    let links = link(&target, "More");
    let heading = format!("Item {number}");
    Some(Page::ok(document(
        &heading,
        &article(
            &heading,
            &format!("Item {number} of the club shop."),
            &links,
        ),
    )))
}

/// The stall's items: three whose pairs read alike, one a pair of which
/// stays pending behind them, and one that reads otherwise with `ref`.
const STALL_ITEMS: [u64; 5] = [1, 2, 3, STALL_PENDING, STALL_SPLIT];
const STALL_PENDING: u64 = 7;
const STALL_SPLIT: u64 = 8;

/// The stall's index, or its item `n` as `ref` asks for it.
fn stall(n: Option<&str>, reference: Option<&str>) -> Option<Page> {
    let Some(n) = n else {
        let mut links = String::new();
        for item in STALL_ITEMS {
            links.push_str(&link(&format!("/stall?n={item}"), "Item"));
            links.push_str(&link(&format!("/stall?n={item}&amp;ref={item}"), "Item"));
        }
        return reference.is_none().then(|| {
            Page::ok(document(
                "Stall",
                &article("Stall", "Every item of the club stall.", &links),
            ))
        });
    };
    let number = canonical_number(n).filter(|number| STALL_ITEMS.contains(number))?;
    let text = if number == STALL_SPLIT && reference.is_some() {
        format!("Item {number} of the club stall, at the members' price.")
    } else {
        format!("Item {number} of the club stall.")
    };
    let links = if number == STALL_PENDING {
        link("/stall?n=1&amp;ref=5", "More")
    } else {
        String::new()
    };
    let heading = format!("Item {number}");
    Some(Page::ok(document(
        &heading,
        &article(&heading, &text, &links),
    )))
}

/// How many pages each part of `/forms/` holds.
const FORM_PAGES: u64 = 3;

/// The page at `/forms/<form>` on `host`, reached at `authority`.
fn forms(form: &str, host: &str, authority: &str) -> Option<Page> {
    if form.is_empty() {
        let bare = authority.strip_prefix("www.").unwrap_or(authority);
        let mut links = String::new();
        for page in 1..=FORM_PAGES {
            links.push_str(&link(&format!("/forms/slash/{page}"), "Guide"));
            links.push_str(&link(&format!("/forms/slash/{page}/"), "Guide"));
        }
        for page in 1..=FORM_PAGES {
            links.push_str(&link(&format!("/forms/docs/{page}/index.html"), "Docs"));
        }
        for page in 1..=FORM_PAGES {
            links.push_str(&link(&format!("/forms/news/{page}"), "News"));
        }
        for page in 1..FORM_PAGES {
            links.push_str(&link(&format!("http://{bare}/forms/split/{page}"), "Split"));
            links.push_str(&link(
                &format!("http://www.{bare}/forms/split/{page}"),
                "Split",
            ));
        }
        return Some(Page::ok(document(
            "Forms",
            &article("Forms", "Every form of the club's pages.", &links),
        )));
    }
    let (part, rest) = form.split_once('/')?;
    match part {
        "slash" => {
            let page = canonical_number(rest.strip_suffix('/').unwrap_or(rest))?;
            (1..=FORM_PAGES).contains(&page).then(|| {
                let heading = format!("Guide {page}");
                Page::ok(document(
                    &heading,
                    &article(&heading, &format!("Guide {page} to the club."), ""),
                ))
            })
        }
        "docs" => {
            let (page, file) = rest.split_once('/')?;
            let page = canonical_number(page).filter(|page| (1..=FORM_PAGES).contains(page))?;
            let folder = format!("/forms/docs/{page}/");
            (file.is_empty() || file == "index.html").then(|| {
                let heading = format!("Docs {page}");
                Page::ok(canonical(
                    &document(
                        &heading,
                        &article(&heading, "The club's docs.", &link(&folder, "This folder")),
                    ),
                    &folder,
                ))
            })
        }
        "news" => {
            let page = canonical_number(rest).filter(|page| (1..=FORM_PAGES).contains(page))?;
            let heading = format!("News {page}");
            Some(Page::ok(canonical(
                &document(
                    &heading,
                    &article(&heading, &format!("The club's news, number {page}."), ""),
                ),
                "/forms/",
            )))
        }
        "split" => {
            let page = canonical_number(rest).filter(|page| (1..=FORM_PAGES).contains(page))?;
            let side = if host.starts_with("www.") {
                "the www site"
            } else {
                "the bare site"
            };
            let links = if page < FORM_PAGES {
                link(&format!("/forms/split/{}", page + 1), "Next")
            } else {
                String::new()
            };
            let heading = format!("Split {page}");
            Some(Page::ok(document(
                &heading,
                &article(&heading, &format!("Page {page} of {side}."), &links),
            )))
        }
        _ => None,
    }
}

/// `html` with a `<link rel="canonical">` to `href` at the end of its head.
fn canonical(html: &str, href: &str) -> String {
    html.replacen(
        "</head>",
        &format!("<link rel=\"canonical\" href=\"{href}\"></head>"),
        1,
    )
}

/// A month of the calendar under `prefix`, which ends at the month `last`
/// counted from January 2026, if it does.
fn calendar(prefix: &str, month: &str, last: Option<u64>) -> Option<Page> {
    let (year, month) = month.split_once('/')?;
    let (year, month) = (canonical_year(year)?, canonical_month(month)?);
    if year < 2026 {
        return None;
    }
    let index = (year - 2026) * 12 + month - 1;
    if last.is_some_and(|last| index >= last) {
        return None;
    }
    let (next_year, next_month) = if month == 12 {
        (year + 1, 1)
    } else {
        (year, month + 1)
    };
    let mut links = link("/", "Home");
    if last.is_none_or(|last| index + 1 < last) && next_year <= 9999 {
        links.push_str(&link(
            &format!("{prefix}{next_year:04}/{next_month:02}"),
            "Next month",
        ));
    }
    Some(Page::ok(document(
        "Events",
        &article("Events", "No events this month.", &links),
    )))
}

fn canonical_year(text: &str) -> Option<u64> {
    (text.len() == 4).then(|| canonical_number(text)).flatten()
}

fn canonical_month(text: &str) -> Option<u64> {
    let month = (text.len() == 2 && text.bytes().all(|b| b.is_ascii_digit()))
        .then(|| text.parse::<u64>().ok())
        .flatten()?;
    (1..=12).contains(&month).then_some(month)
}

/// The rows of the listing at `path` from `start`, its `rows` rows long,
/// linking the next page by offset unless `by_cursor`, and by cursor when
/// the listing has one.
fn rows_page(path: &str, start: u64, rows: u64, cursor: bool, by_cursor: bool) -> String {
    let end = (start + PAGE_ROWS).min(rows);
    let mut text = String::new();
    for row in start..end {
        write!(text, "Row {row}. ").expect("invariant: writing to a String succeeds");
    }
    let mut links = String::new();
    if end < rows {
        if !by_cursor {
            links.push_str(&link(&format!("{path}?offset={end}"), "Next page"));
        }
        if cursor {
            links.push_str(&link(&format!("{path}?after=r{end}"), "More rows"));
        }
    }
    let heading = format!("Rows from {start}");
    document(&heading, &article(&heading, text.trim_end(), &links))
}
