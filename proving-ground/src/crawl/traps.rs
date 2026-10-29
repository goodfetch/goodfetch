//! The `traps` family on `traps.test`: a shop of items in three categories,
//! each item linked at two URLs of the same page that the fixed URL rules
//! (ADR 0120) make one (a fragment, `utm_*`, `gclid`, `fbclid`,
//! `;jsessionid=`, a percent-encoded unreserved character, parameters in
//! another order); two crawler traps, an events calendar whose months page
//! on without end and an archive whose relative "older" link nests without
//! end; and honeypot links, hidden from people in four ways and disallowed
//! by `robots.txt`; and `/hidden-links`, which no page links, a page of
//! every way a link hides from people (ADR 0035's trap links): the four,
//! an `opacity` of 0, a link clipped to nothing and two invisible
//! `nofollow` links, each under `/hp/`, beside three links a person sees,
//! one plain, one `nofollow` and one in an ad (EasyList's `##.ad-slot`);
//! and `/cards`, which no page links either, a page whose markup hides
//! some links to pages a person also reaches (spec #600's trap link
//! cases): item 3's card links it only through an image link hidden from
//! assistive technology (`aria-hidden`), an off-canvas menu marked
//! `hidden` links category 1 before the visible menu does, and items 1
//! and 2 are each linked twice, by the hidden image link and by the
//! visible title link, in either order.
//!
//! The family records a `duplicate` finding for each page a task fetches
//! again, a `trap` finding for each trap request past the task's allowance,
//! and a `honeypot` finding for each honeypot request, which also flags the
//! address that made it: the task's host refuses it from then on.

use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::net::IpAddr;

use super::{Counts, Page, Visit, canonical_number, crawl_task, document, name, numbered};
use crate::generator;
use crate::sync::Mutex;
use crate::task::{Finding, Record, Task};

pub(super) const HOST: &str = "traps.test";
pub(super) const FAMILY: &str = "traps";

/// The generator place of the family's values.
const FAMILY_PLACE: u64 = 22;
const HONEYPOT_PLACE: u64 = 1;
const TOKEN_PLACE: u64 = 2;
/// The page number `/hidden-links` draws its honeypots' tokens at, past
/// every category's.
const HIDDEN_LINKS_PAGE: u64 = 1000;

const CATEGORIES: u64 = 3;
const ITEMS_PER_CATEGORY: u64 = 10;
const ITEMS: u64 = CATEGORIES * ITEMS_PER_CATEGORY;
/// The requests a task's crawl may make to the two traps together.
const TRAP_ALLOWANCE: u64 = 30;
/// The most addresses flagged per `serve` (STYLE.md: every buffer is
/// bounded); past it, a honeypot is still recorded but flags no one more.
const FLAGGED_LIMIT: usize = 4096;

/// What the family keeps: the pages each task host fetched, the trap
/// requests it made, and the addresses it flagged.
#[derive(Default)]
pub(super) struct State {
    fetched: Mutex<BTreeSet<(String, String)>>,
    traps: Counts,
    flagged: Mutex<BTreeSet<(String, IpAddr)>>,
}

fn item_id(number: u64) -> String {
    format!("I-{number:04}")
}

fn item_name(seed: u64, number: u64) -> String {
    name(seed, &[FAMILY_PLACE, number])
}

pub(super) fn respond(visit: &Visit<'_>, state: &State) -> Page {
    let request = visit.request;
    let flag = (request.host.to_owned(), request.peer);
    if state.flagged.lock().contains(&flag) {
        return Page::blocked();
    }
    let path = request.path;
    if path == "/robots.txt" {
        return Page::text(
            "text/plain; charset=utf-8",
            "User-agent: *\nDisallow: /hp/\n".to_owned(),
        );
    }
    if path == "/hidden-links" {
        return Page::ok(hidden_links(visit.seed));
    }
    if path == "/cards" {
        return Page::ok(cards(visit.seed));
    }
    if path.starts_with("/hp/") {
        visit.find(Finding::Honeypot {
            target: request.target(),
        });
        let mut flagged = state.flagged.lock();
        if flagged.len() < FLAGGED_LIMIT {
            flagged.insert(flag);
        }
        return Page::ok(document(
            "Offers",
            "<h1>Offers</h1>\n<p>No offers today.</p>\n",
        ));
    }
    if let Some(page) = trap(path) {
        if state.traps.add(request.host) > TRAP_ALLOWANCE {
            visit.find(Finding::Trap {
                target: request.target(),
            });
        }
        return Page::ok(page);
    }
    let Some((page, html)) = page(visit.seed, path) else {
        return Page::not_found();
    };
    if !state
        .fetched
        .lock()
        .insert((request.host.to_owned(), page.clone()))
    {
        visit.find(Finding::Duplicate {
            page,
            target: request.target(),
        });
    }
    Page::ok(html)
}

/// A page that is no trap, with the name it has under every URL of it.
fn page(seed: u64, path: &str) -> Option<(String, String)> {
    if path == "/" {
        return Some(("/".to_owned(), home(seed)));
    }
    if let Some(number) = path.strip_prefix("/categories/").and_then(canonical_number) {
        return (1..=CATEGORIES)
            .contains(&number)
            .then(|| (path.to_owned(), category(seed, number)));
    }
    let item = path.strip_prefix("/items/")?;
    // The fixed rules: `;jsessionid=` dropped, unreserved characters decoded.
    let item = item
        .split(';')
        .next()
        .unwrap_or_default()
        .replace("%2D", "-")
        .replace("%2d", "-");
    let number = numbered(&item, "I-", 4, ITEMS)?;
    let id = item_id(number);
    let name = item_name(seed, number);
    let category = (number - 1) / ITEMS_PER_CATEGORY + 1;
    Some((
        format!("/items/{id}"),
        document(
            &name,
            &format!(
                "<main>\n<article class=\"item\" data-id=\"{id}\">\n<h1 class=\"name\">{name}</h1>\n<p>Item {id}.</p>\n</article>\n<a href=\"/categories/{category}\">Back to the category</a>\n</main>\n"
            ),
        ),
    ))
}

/// The four honeypot links of a page, each hidden from people its own way.
fn honeypots(seed: u64, page: u64) -> String {
    let token = |form: u64| {
        generator::draw(seed, &[FAMILY_PLACE, HONEYPOT_PLACE, page, form]) & 0xFFFF_FFFF
    };
    format!(
        "<a href=\"/hp/{:08x}\" hidden>Special offer</a>\n\
        <a href=\"/hp/{:08x}\" style=\"display:none\">Staff</a>\n\
        <a href=\"/hp/{:08x}\" style=\"visibility:hidden\">Archive index</a>\n\
        <div aria-hidden=\"true\" style=\"position:absolute;left:-10000px\"><a href=\"/hp/{:08x}\" tabindex=\"-1\">Catalogue</a></div>\n",
        token(1),
        token(2),
        token(3),
        token(4),
    )
}

/// The page of `/hidden-links`: the honeypot links of every way a link
/// hides, and the three a person sees, to items 1, 2 and 3.
fn hidden_links(seed: u64) -> String {
    let token = |form: u64| {
        generator::draw(
            seed,
            &[FAMILY_PLACE, HONEYPOT_PLACE, HIDDEN_LINKS_PAGE, form],
        ) & 0xFFFF_FFFF
    };
    let mut body = String::from("<main>\n<h1>Hidden links</h1>\n");
    body.push_str(&honeypots(seed, HIDDEN_LINKS_PAGE));
    write!(
        body,
        "<a href=\"/hp/{:08x}\" style=\"opacity:0\">Members</a>\n\
        <span style=\"position:absolute;width:1px;height:1px;overflow:hidden;clip:rect(0,0,0,0)\"><a href=\"/hp/{:08x}\">Wholesale</a></span>\n\
        <a href=\"/hp/{:08x}\" rel=\"nofollow\" style=\"display:none\">Login</a>\n\
        <a href=\"/hp/{:08x}\" rel=\"nofollow noopener\" style=\"position:absolute;top:-5000px\">Feeds</a>\n\
        <ul>\n<li><a href=\"/items/I-0001\">The first item</a></li>\n\
        <li><a href=\"/items/I-0002\" rel=\"nofollow\">The second item</a></li>\n</ul>\n\
        <div class=\"ad-slot\"><a href=\"/items/I-0003\">The third item, in an ad</a></div>\n</main>\n",
        token(5),
        token(6),
        token(7),
        token(8),
    )
    .expect("invariant: writing to a String succeeds");
    document("Hidden links", &body)
}

/// The page of `/cards`: item 3's card, which links it only through an
/// image link hidden from assistive technology; an off-canvas menu marked
/// `hidden` linking category 1; the cards of items 1 and 2, each linking
/// its item through that hidden image link and through its visible title,
/// item 1's image first and item 2's title first; then the visible menu,
/// linking category 1 again.
fn cards(seed: u64) -> String {
    let image = |number: u64| {
        format!(
            "<a href=\"/items/{}\" aria-hidden=\"true\" tabindex=\"-1\"><span class=\"picture\">Picture</span></a>",
            item_id(number)
        )
    };
    let title = |number: u64| {
        format!(
            "<h3><a href=\"/items/{}\">{}</a></h3>",
            item_id(number),
            item_name(seed, number)
        )
    };
    let body = format!(
        "<main>\n<h1>Cards</h1>\n<div class=\"card\">{}</div>\n<nav hidden><a href=\"/categories/1\">Category 1</a></nav>\n<div class=\"card\">{}{}</div>\n<div class=\"card\">{}{}</div>\n<nav><a href=\"/categories/1\">Category 1</a></nav>\n</main>\n",
        image(3),
        image(1),
        title(1),
        title(2),
        image(2),
    );
    document("Cards", &body)
}

fn home(seed: u64) -> String {
    let mut body = String::from("<main>\n<h1>Shop</h1>\n<ul class=\"categories\">\n");
    for number in 1..=CATEGORIES {
        writeln!(
            body,
            "<li><a href=\"/categories/{number}\">Category {number}</a></li>"
        )
        .expect("invariant: writing to a String succeeds");
    }
    body.push_str("</ul>\n");
    body.push_str(&honeypots(seed, 0));
    body.push_str(
        "<nav><a href=\"/calendar/2026-01\">Events</a> <a href=\"/archive/\">Archive</a></nav>\n</main>\n",
    );
    document("Shop", &body)
}

/// A category page: each of its items at two URLs of the same page.
fn category(seed: u64, number: u64) -> String {
    let mut body = format!("<main>\n<h1>Category {number}</h1>\n<ul class=\"items\">\n");
    let first = (number - 1) * ITEMS_PER_CATEGORY + 1;
    for item in first..first + ITEMS_PER_CATEGORY {
        let id = item_id(item);
        let token = generator::draw(seed, &[FAMILY_PLACE, TOKEN_PLACE, item]);
        let (one, other) = match (item - 1) % 6 {
            0 => (format!("/items/{id}"), format!("/items/{id}#reviews")),
            1 => (
                format!("/items/{id}"),
                format!("/items/{id}?utm_source=category&amp;utm_medium=list"),
            ),
            2 => (
                format!("/items/{id}?gclid={token:016x}"),
                format!("/items/{id}?fbclid={:016x}", token.rotate_left(17)),
            ),
            3 => (
                format!("/items/{id};jsessionid={token:016X}"),
                format!("/items/{id}"),
            ),
            4 => (
                format!("/items/{}", id.replace('-', "%2D")),
                format!("/items/{id}"),
            ),
            _ => (
                format!("/items/{id}?view=full&amp;lang=en"),
                format!("/items/{id}?lang=en&amp;view=full"),
            ),
        };
        writeln!(
            body,
            "<li class=\"item\"><a href=\"{one}\">{}</a> <a href=\"{other}\">Details</a></li>",
            item_name(seed, item)
        )
        .expect("invariant: writing to a String succeeds");
    }
    body.push_str("</ul>\n");
    body.push_str(&honeypots(seed, number));
    body.push_str("<a href=\"/\">Shop</a>\n</main>\n");
    document(&format!("Category {number}"), &body)
}

/// A trap page, if `path` is one: an empty month of the events calendar, or
/// a level of the archive.
fn trap(path: &str) -> Option<String> {
    if let Some(month) = path.strip_prefix("/calendar/") {
        let (year, month) = month.split_once('-')?;
        let canonical = |text: &str, width: usize| {
            (text.len() == width && text.bytes().all(|b| b.is_ascii_digit()))
                .then(|| text.parse::<u32>().ok())
                .flatten()
        };
        let (year, month) = (canonical(year, 4)?, canonical(month, 2)?);
        if !(1..=9999).contains(&year) || !(1..=12).contains(&month) {
            return None;
        }
        let previous = if month == 1 {
            (year - 1, 12)
        } else {
            (year, month - 1)
        };
        let next = if month == 12 {
            (year + 1, 1)
        } else {
            (year, month + 1)
        };
        let link = |(year, month): (u32, u32), text: &str| {
            if (1..=9999).contains(&year) {
                format!("<a href=\"/calendar/{year:04}-{month:02}\">{text}</a>")
            } else {
                String::new()
            }
        };
        return Some(document(
            "Events",
            &format!(
                "<main>\n<h1>Events, {year:04}-{month:02}</h1>\n<p>No events this month.</p>\n<nav>{} {}</nav>\n</main>\n",
                link(previous, "Previous month"),
                link(next, "Next month")
            ),
        ));
    }
    let rest = path.strip_prefix("/archive/")?;
    let mut levels = rest;
    while let Some(deeper) = levels.strip_prefix("more/") {
        levels = deeper;
    }
    levels.is_empty().then(|| {
        document(
            "Archive",
            "<main>\n<h1>Archive</h1>\n<p>Nothing older here.</p>\n<a href=\"more/\">Older</a>\n</main>\n",
        )
    })
}

pub(super) fn tasks(seed: u64, port: Option<u16>) -> Vec<Task> {
    vec![crawl_task(
        (HOST, FAMILY),
        1,
        port,
        "/",
        "Crawl the shop from its start page and return one Record per item, with the fields \
            `id` and `name`. Items are linked at several URLs of one page, the events calendar \
            and the archive page on without end, and some links are hidden from people. A crawl \
            must fetch each page once, may make at most 30 requests to the calendar and the \
            archive together, and must never follow a hidden link.",
        (1..=ITEMS)
            .map(|number| {
                Record::from([("id", item_id(number)), ("name", item_name(seed, number))])
            })
            .collect(),
    )]
}
