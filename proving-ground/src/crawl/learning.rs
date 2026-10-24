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
//! - `/calendar/<yyyy>/<mm>`: an events calendar whose months, all empty
//!   and alike, page on without end, each linking the next month and the
//!   home page; `/agenda/<yyyy>/<mm>` is the same for the 100 months from
//!   January 2026.
//! - `/feed?offset=<n>`: 200 rows paged 20 at a time by `offset`, each
//!   page linking the next offset and the cursor of the rows after it,
//!   `?after=r<n>`, which pages on by cursor alone; `/list?offset=<n>` is
//!   the same listing by offset alone, and `/pair?offset=<n>` holds two of
//!   its pages.
//! - `/`: the home page, which links nothing.

use std::fmt::Write as _;

use super::{Page, Visit, canonical_number, document};

pub(super) const HOST: &str = "learning.test";
/// A second host of the family, of the same registrable domain.
pub(super) const SHOP_HOST: &str = "shop.learning.test";
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
    if let Some(pairs) = path.strip_prefix("/shop/").and_then(canonical_number) {
        return shop(pairs, request.parameter("n"), request.parameter("ref"))
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
