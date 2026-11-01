//! The `links` family on `links.test` (pitfalls U08 and U14): links a crawl
//! resolves or finds wrongly.
//!
//! - `links/1`: a shelf at `/guide/` whose `<base href="/library/shelves/">`
//!   makes its relative links `book/<n>` name `/library/shelves/book/<n>`.
//! - `links/2`: `/old/shelf` moves (`301`) to `/new/shelf/`, whose relative
//!   links `book/<n>` resolve against the URL it moved to, naming
//!   `/new/shelf/book/<n>`.
//! - `links/3`: rooms linked four ways from `/`: plain `<a href>`s, and
//!   buttons whose `onclick` navigates (`location.assign`), opens a popup
//!   (`window.open`) or pushes a history entry (`history.pushState`) to a
//!   path the script builds from its parts, so the page spells out no URL
//!   and a click requests nothing until the crawl follows it.
//!
//! - `links.test/answers/` (no task): links, in order, `/answers/1`, the
//!   same page with a fragment, an anchor with no `href`, a mail address,
//!   the off-site `http://elsewhere.test/`, a URL that does not parse
//!   (`http://[::1/`), and `2` and `3` relative to it, the last carrying
//!   `data-isbn`: every answer an `enqueueLinks` call gives (ADR 0093).
//!   `/answers/<n>` (1 to 3) is a page with its ID.
//! - `links.test/shift-jis/` (no task): a page written in `Shift_JIS`,
//!   labelled so, linking `search?q=日本&k=한`, 한 written as the character
//!   reference `&#54620;` since `Shift_JIS` has no 한, so a link's query is
//!   percent-encoded as a browser encodes it (ADR 0120);
//!   `/shift-jis/search` answers any query.
//!
//! A request for what a wrong base makes of a relative link (`/guide/book/…`,
//! `/old/book/…`) answers `404` and records a `wrong_base` finding.

use std::fmt::Write as _;

use encoding_rs::SHIFT_JIS;
use hyper::StatusCode;

use super::{Page, Visit, crawl_task, document, name, numbered};
use crate::task::{Finding, Record, Task};

pub(super) const HOST: &str = "links.test";
pub(super) const FAMILY: &str = "links";

/// The generator place of the family's values.
const FAMILY_PLACE: u64 = 26;

const BOOKS: u64 = 8;
const ROOMS: u64 = 12;

/// Where each shelf's books are, and what a wrong base makes of their
/// links.
const BASE_BOOKS: &str = "/library/shelves/book/";
const BASE_WRONG: &str = "/guide/book/";
const MOVED_BOOKS: &str = "/new/shelf/book/";
const MOVED_WRONG: &str = "/old/book/";

/// How a room of `links/3` is linked, by its number.
#[derive(Clone, Copy)]
enum Linked {
    Anchor,
    Navigation,
    Popup,
    PushState,
}

fn linked(number: u64) -> Linked {
    match (number - 1) % 6 {
        0 | 1 => Linked::Anchor,
        2 | 3 => Linked::Navigation,
        4 => Linked::Popup,
        _ => Linked::PushState,
    }
}

fn book_id(ordinal: u64, number: u64) -> String {
    format!("B-{ordinal}-{number:02}")
}

fn room_id(number: u64) -> String {
    format!("R-{number:02}")
}

fn title(seed: u64, ordinal: u64, number: u64) -> String {
    name(seed, &[FAMILY_PLACE, ordinal, number])
}

pub(super) fn respond(visit: &Visit<'_>) -> Page {
    let request = visit.request;
    let ordinal = request
        .host
        .strip_suffix(HOST)
        .and_then(|rest| rest.strip_suffix('.'))
        .and_then(|ordinal| ordinal.parse::<u64>().ok());
    let path = request.path;
    for wrong in [BASE_WRONG, MOVED_WRONG] {
        if path.starts_with(wrong) {
            visit.find(Finding::WrongBase {
                target: request.target(),
            });
            return Page::not_found();
        }
    }
    match (ordinal, path) {
        (Some(1), "/guide/") => Page::ok(shelf(Some("/library/shelves/"))),
        (Some(1), _) => book(visit.seed, 1, path, BASE_BOOKS),
        (Some(2), "/old/shelf") => Page::redirect(StatusCode::MOVED_PERMANENTLY, "/new/shelf/"),
        (Some(2), "/new/shelf/") => Page::ok(shelf(None)),
        (Some(2), _) => book(visit.seed, 2, path, MOVED_BOOKS),
        (Some(3), "/") => Page::ok(rooms(visit.seed)),
        (None, "/answers/") => Page::ok(answers()),
        (None, "/shift-jis/") => shift_jis(&document(
            "日本",
            "<main>\n<h1>日本</h1>\n<p><a href=\"search?q=日本&amp;k=한\">日本</a></p>\n</main>\n",
        )),
        (None, "/shift-jis/search") => {
            shift_jis(&document("検索", "<main>\n<h1>検索</h1>\n</main>\n"))
        }
        (None, _) => path
            .strip_prefix("/answers/")
            .and_then(super::canonical_number)
            .filter(|number| (1..=3).contains(number))
            .map_or_else(Page::not_found, |number| {
                Page::ok(article(&format!("A-{number}"), &format!("Answer {number}")))
            }),
        (Some(3), _) => path
            .strip_prefix("/rooms/")
            .and_then(|id| numbered(id, "R-", 2, ROOMS))
            .map_or_else(Page::not_found, |number| {
                Page::ok(article(&room_id(number), &title(visit.seed, 3, number)))
            }),
        _ => Page::not_found(),
    }
}

/// `links.test/answers/`: a link for each answer of an `enqueueLinks` call.
fn answers() -> String {
    document(
        "Answers",
        "<main>\n<h1>Answers</h1>\n<ul class=\"links\">\n\
<li><a href=\"/answers/1\">One</a></li>\n\
<li><a href=\"/answers/1#top\">One again</a></li>\n\
<li><a name=\"here\">No link</a></li>\n\
<li><a href=\"mailto:shop@links.test\">Mail</a></li>\n\
<li><a href=\"http://elsewhere.test/\">Elsewhere</a></li>\n\
<li><a href=\"http://[::1/\">Broken</a></li>\n\
<li><a href=\"2\">Two</a></li>\n\
<li><a href=\"3\" data-isbn=\"978-3\">Three</a></li>\n\
</ul>\n</main>\n",
    )
}

/// `html` written in `Shift_JIS`, a character it has none for as its
/// numeric character reference, labelled so, its `<meta>` dropped.
fn shift_jis(html: &str) -> Page {
    let html = html.replacen("<meta charset=\"utf-8\">", "", 1);
    let (bytes, _, _) = SHIFT_JIS.encode(&html);
    Page {
        content_type: "text/html; charset=Shift_JIS",
        body: bytes.into_owned().into(),
        ..Page::ok(String::new())
    }
}

/// A shelf linking its books relatively, `book/<n>`, under `base` if any.
fn shelf(base: Option<&str>) -> String {
    let mut body = String::from("<main>\n<h1>Shelf</h1>\n<ul class=\"books\">\n");
    for number in 1..=BOOKS {
        writeln!(body, "<li><a href=\"book/{number}\">Book {number}</a></li>")
            .expect("invariant: writing to a String succeeds");
    }
    body.push_str("</ul>\n</main>\n");
    let page = document("Shelf", &body);
    match base {
        Some(base) => page.replacen(
            "<meta charset=\"utf-8\">",
            &format!("<meta charset=\"utf-8\"><base href=\"{base}\">"),
            1,
        ),
        None => page,
    }
}

/// Book `<n>` of task `ordinal`'s shelf, at `<books><n>`.
fn book(seed: u64, ordinal: u64, path: &str, books: &str) -> Page {
    path.strip_prefix(books)
        .and_then(super::canonical_number)
        .filter(|number| (1..=BOOKS).contains(number))
        .map_or_else(Page::not_found, |number| {
            Page::ok(article(
                &book_id(ordinal, number),
                &title(seed, ordinal, number),
            ))
        })
}

fn article(id: &str, title: &str) -> String {
    document(
        title,
        &format!(
            "<main>\n<article class=\"entry\" data-id=\"{id}\">\n<h1 class=\"title\">{title}</h1>\n</article>\n</main>\n"
        ),
    )
}

/// `links/3`'s start page: each room linked as [`linked`] says.
fn rooms(seed: u64) -> String {
    let mut body = String::from(
        "<main>\n<h1>Rooms</h1>\n<p class=\"status\" role=\"status\"></p>\n<ul class=\"rooms\">\n",
    );
    for number in 1..=ROOMS {
        let (id, title) = (room_id(number), title(seed, 3, number));
        // The script builds the path from its parts, so the page never
        // spells out a script-only room's URL.
        let parts = format!("['', 'rooms', '{id}'].join('/')");
        let item = match linked(number) {
            Linked::Anchor => format!("<a href=\"/rooms/{id}\">{title}</a>"),
            Linked::Navigation => {
                format!(
                    "<button type=\"button\" onclick=\"location.assign({parts})\">{title}</button>"
                )
            }
            Linked::Popup => {
                format!("<button type=\"button\" onclick=\"window.open({parts})\">{title}</button>")
            }
            Linked::PushState => format!(
                "<button type=\"button\" onclick=\"history.pushState(null, '', {parts}); document.querySelector('.status').textContent = 'Room selected'\">{title}</button>"
            ),
        };
        writeln!(body, "<li>{item}</li>").expect("invariant: writing to a String succeeds");
    }
    body.push_str("</ul>\n</main>\n");
    document("Rooms", &body)
}

pub(super) fn tasks(seed: u64, port: Option<u16>) -> Vec<Task> {
    let books = |ordinal| -> Vec<Record> {
        (1..=BOOKS)
            .map(|number| {
                Record::from([
                    ("id", book_id(ordinal, number)),
                    ("title", title(seed, ordinal, number)),
                ])
            })
            .collect()
    };
    vec![
        crawl_task(
            (HOST, FAMILY),
            1,
            port,
            "/guide/",
            "Crawl the books this shelf links and return one Record per book, with the fields \
                `id` and `title`. The shelf's links are relative to its `<base href>`.",
            books(1),
        ),
        crawl_task(
            (HOST, FAMILY),
            2,
            port,
            "/old/shelf",
            "Crawl the books this shelf links and return one Record per book, with the fields \
                `id` and `title`. The shelf has moved; its links are relative to the URL it \
                moved to.",
            books(2),
        ),
        crawl_task(
            (HOST, FAMILY),
            3,
            port,
            "/",
            "Crawl the rooms this page links and return one Record per room, with the fields \
                `id` and `title`. Some rooms are linked only from the page's scripts: a button \
                that navigates, one that opens a popup and one that pushes a history entry.",
            (1..=ROOMS)
                .map(|number| {
                    Record::from([("id", room_id(number)), ("title", title(seed, 3, number))])
                })
                .collect(),
        ),
    ]
}
