//! The `scale` family on `scale.test` (story 28): 4,194,304 item pages,
//! `/items/1` to `/items/4194304`, each generated on request from the seed
//! and its number, never stored. Items come in sections of 256: a section
//! page `/sections/<n>` links its items, and its sitemap index
//! `/sections/<n>/sitemap.xml` names two sitemaps of 128 items each. The
//! whole family is listed by `/sitemap.xml`, a sitemap index of 128 sitemaps
//! of 32,768 items each, within the protocol's 50,000. Sitemaps name
//! absolute URLs on the host and port the request named. Beside them, a
//! site of any size up to the item count, `/sites/<size>/1` to
//! `/sites/<size>/<size>`, a tree whose pages link only each other: the
//! 1,000-page site is what Spider's comparison crawls (ADR 0134).

use std::fmt::Write as _;

use super::{Page, Visit, canonical_number, crawl_task, document, name};
use crate::generator;
use crate::task::{Record, Task};

pub(super) const HOST: &str = "scale.test";
pub(super) const FAMILY: &str = "scale";

/// The generator places of the family's values.
const FAMILY_PLACE: u64 = 25;
const LISTED_PLACE: u64 = 1;
const SITEMAPPED_PLACE: u64 = 2;

const ITEMS: u64 = 1 << 22;
const SECTION: u64 = 256;
const SECTIONS: u64 = ITEMS / SECTION;
/// Items per sitemap of the whole family's index.
const SITEMAP: u64 = 1 << 15;
const SITEMAPS: u64 = ITEMS / SITEMAP;
/// The pages each page of a site links below it.
const SITE_FAN_OUT: u64 = 10;

fn item_id(number: u64) -> String {
    format!("S-{number:07}")
}

fn item_name(seed: u64, number: u64) -> String {
    name(seed, &[FAMILY_PLACE, number])
}

/// The items of section `section`, from 1.
fn section_items(section: u64) -> std::ops::RangeInclusive<u64> {
    (section - 1) * SECTION + 1..=section * SECTION
}

pub(super) fn respond(visit: &Visit<'_>) -> Page {
    let path = visit.request.path;
    let origin = format!("http://{}", visit.request.authority);
    if path == "/" {
        return Page::ok(document(
            "Items",
            "<main>\n<h1>Items</h1>\n<p>Every item is in <a href=\"/sitemap.xml\">the sitemap</a>.</p>\n</main>\n",
        ));
    }
    if path == "/sitemap.xml" {
        return sitemap_index(
            &origin,
            (1..=SITEMAPS).map(|number| format!("/sitemaps/{number}.xml")),
        );
    }
    if let Some(number) = path
        .strip_prefix("/sitemaps/")
        .and_then(|rest| rest.strip_suffix(".xml"))
        .and_then(canonical_number)
        .filter(|number| (1..=SITEMAPS).contains(number))
    {
        return sitemap(&origin, (number - 1) * SITEMAP + 1..=number * SITEMAP);
    }
    if let Some(number) = path
        .strip_prefix("/items/")
        .and_then(canonical_number)
        .filter(|number| (1..=ITEMS).contains(number))
    {
        return Page::ok(item(visit.seed, number));
    }
    if let Some(rest) = path.strip_prefix("/sites/") {
        return site_page(visit.seed, rest);
    }
    let Some(rest) = path.strip_prefix("/sections/") else {
        return Page::not_found();
    };
    let (section, file) = rest.split_once('/').unwrap_or((rest, ""));
    let Some(section) =
        canonical_number(section).filter(|section| (1..=SECTIONS).contains(section))
    else {
        return Page::not_found();
    };
    let items = section_items(section);
    let half = items.start() + SECTION / 2;
    match file {
        "" => Page::ok(section_page(visit.seed, section)),
        "sitemap.xml" => sitemap_index(
            &origin,
            [1, 2]
                .map(|part| format!("/sections/{section}/sitemap-{part}.xml"))
                .into_iter(),
        ),
        "sitemap-1.xml" => sitemap(&origin, *items.start()..=half - 1),
        "sitemap-2.xml" => sitemap(&origin, half..=*items.end()),
        _ => Page::not_found(),
    }
}

fn item(seed: u64, number: u64) -> String {
    let (id, name) = (item_id(number), item_name(seed, number));
    let section = (number - 1) / SECTION + 1;
    document(
        &name,
        &format!(
            "<main>\n<article class=\"item\" data-id=\"{id}\">\n<h1 class=\"name\">{name}</h1>\n</article>\n<a href=\"/sections/{section}\">Section {section}</a>\n</main>\n"
        ),
    )
}

fn section_page(seed: u64, section: u64) -> String {
    let mut body = format!("<main>\n<h1>Section {section}</h1>\n<ul class=\"items\">\n");
    for number in section_items(section) {
        writeln!(
            body,
            "<li><a href=\"/items/{number}\">{}</a></li>",
            item_name(seed, number)
        )
        .expect("invariant: writing to a String succeeds");
    }
    body.push_str("</ul>\n</main>\n");
    document(&format!("Section {section}"), &body)
}

/// The page `<page>` of the site of `<size>` pages, `rest` being
/// `<size>/<page>`: an item of the family, which links home (page 1) and
/// its children in the site's tree of [`SITE_FAN_OUT`] children a page,
/// so a crawl from page 1 reaches every page of the site and no other.
fn site_page(seed: u64, rest: &str) -> Page {
    let Some((size, page)) = rest.split_once('/') else {
        return Page::not_found();
    };
    let (Some(size), Some(page)) = (
        canonical_number(size).filter(|size| (1..=ITEMS).contains(size)),
        canonical_number(page),
    ) else {
        return Page::not_found();
    };
    if !(1..=size).contains(&page) {
        return Page::not_found();
    }
    let (id, name) = (item_id(page), item_name(seed, page));
    let mut body = format!(
        "<main>\n<article class=\"item\" data-id=\"{id}\">\n<h1 class=\"name\">{name}</h1>\n</article>\n<nav>\n<a href=\"/sites/{size}/1\">Home</a>\n<ul class=\"pages\">\n"
    );
    let first_child = (page - 1) * SITE_FAN_OUT + 2;
    for child in first_child..=(first_child + SITE_FAN_OUT - 1).min(size) {
        writeln!(
            body,
            "<li><a href=\"/sites/{size}/{child}\">{}</a></li>",
            item_name(seed, child)
        )
        .expect("invariant: writing to a String succeeds");
    }
    body.push_str("</ul>\n</nav>\n</main>\n");
    Page::ok(document(&name, &body))
}

fn xml(text: String) -> Page {
    Page::text("application/xml", text)
}

/// A sitemap index naming `sitemaps`, paths on `origin`.
fn sitemap_index(origin: &str, sitemaps: impl Iterator<Item = String>) -> Page {
    let mut text = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<sitemapindex xmlns=\"http://www.sitemaps.org/schemas/sitemap/0.9\">\n",
    );
    for path in sitemaps {
        writeln!(text, "<sitemap><loc>{origin}{path}</loc></sitemap>")
            .expect("invariant: writing to a String succeeds");
    }
    text.push_str("</sitemapindex>\n");
    xml(text)
}

/// A sitemap of the items `items`, on `origin`.
fn sitemap(origin: &str, items: std::ops::RangeInclusive<u64>) -> Page {
    let mut text = String::with_capacity(64 * usize::try_from(SITEMAP).unwrap_or(0));
    text.push_str(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<urlset xmlns=\"http://www.sitemaps.org/schemas/sitemap/0.9\">\n",
    );
    for number in items {
        writeln!(text, "<url><loc>{origin}/items/{number}</loc></url>")
            .expect("invariant: writing to a String succeeds");
    }
    text.push_str("</urlset>\n");
    xml(text)
}

pub(super) fn tasks(seed: u64, port: Option<u16>) -> Vec<Task> {
    let listed = 1 + generator::below(seed, &[FAMILY_PLACE, 0, LISTED_PLACE], SECTIONS);
    let sitemapped = 1 + generator::below(seed, &[FAMILY_PLACE, 0, SITEMAPPED_PLACE], SECTIONS);
    let records = |section| -> Vec<Record> {
        section_items(section)
            .map(|number| {
                Record::from([("id", item_id(number)), ("name", item_name(seed, number))])
            })
            .collect()
    };
    vec![
        crawl_task(
            (HOST, FAMILY),
            1,
            port,
            &format!("/sections/{listed}"),
            "Crawl the items this section page links and return one Record per item, with the \
                fields `id` and `name`. The section is one of 16,384 of the site's 4,194,304 \
                items; crawl this section only.",
            records(listed),
        ),
        crawl_task(
            (HOST, FAMILY),
            2,
            port,
            &format!("/sections/{sitemapped}/sitemap.xml"),
            "Crawl the items this sitemap index lists, through its sitemaps, and return one \
                Record per item, with the fields `id` and `name`. The section is one of 16,384 \
                of the site's 4,194,304 items; crawl this section only.",
            records(sitemapped),
        ),
    ]
}
