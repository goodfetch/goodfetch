//! The `sitemaps` family on `sitemaps.test`: a sitemap index of 50,048
//! sitemaps, more than the protocol's 50,000 (sitemaps.org), at
//! `/sitemap-index.xml` and gzip-compressed at `/sitemap-index.xml.gz`, both
//! generated on request (about 3 MB plain), so no committed file holds it
//! (ADR 0062's 256 KiB). Sitemap `/sitemaps/<n>.xml` lists item
//! `/items/<n>` when `n` is a multiple of 1,024 or past 50,000, and is empty
//! otherwise: a crawl that reads only the index's first 50,000 entries
//! loses the 48 items past them. Sitemaps name absolute URLs on the host and
//! port the request named.

use std::fmt::Write as _;
use std::io::Write as _;

use flate2::Compression;
use flate2::write::GzEncoder;
use hyper::StatusCode;

use super::{Page, Visit, canonical_number, crawl_task, document, name};
use crate::task::{Record, Task};

pub(super) const HOST: &str = "sitemaps.test";
pub(super) const FAMILY: &str = "sitemaps";

/// The generator place of the family's values.
const FAMILY_PLACE: u64 = 30;

const SITEMAPS: u64 = 50_048;
/// The protocol's limit of entries in one sitemap index.
const PROTOCOL_LIMIT: u64 = 50_000;
/// Every how many sitemaps one within the protocol's limit lists an item.
const LISTED_EVERY: u64 = 1_024;

/// Whether sitemap `number` lists its item.
fn lists_item(number: u64) -> bool {
    number.is_multiple_of(LISTED_EVERY) || number > PROTOCOL_LIMIT
}

fn item_id(number: u64) -> String {
    format!("M-{number:05}")
}

fn item_name(seed: u64, number: u64) -> String {
    name(seed, &[FAMILY_PLACE, number])
}

pub(super) fn respond(visit: &Visit<'_>) -> Page {
    let origin = format!("http://{}", visit.request.authority);
    let path = visit.request.path;
    match path {
        "/sitemap-index.xml" => return xml(index(&origin)),
        "/sitemap-index.xml.gz" => return gzip(index(&origin).as_bytes()),
        _ => {}
    }
    let numbered = |prefix: &str, suffix: &str| {
        path.strip_prefix(prefix)
            .and_then(|rest| rest.strip_suffix(suffix))
            .and_then(canonical_number)
            .filter(|number| (1..=SITEMAPS).contains(number))
    };
    if let Some(number) = numbered("/sitemaps/", ".xml") {
        return xml(sitemap(&origin, number));
    }
    numbered("/items/", "")
        .filter(|number| lists_item(*number))
        .map_or_else(Page::not_found, |number| Page::ok(item(visit.seed, number)))
}

fn xml(text: String) -> Page {
    Page::text("application/xml", text)
}

/// `text` gzip-compressed, as a `.gz` sitemap is served: the file itself,
/// not a content coding.
pub(super) fn gzip(text: &[u8]) -> Page {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::fast());
    encoder
        .write_all(text)
        .expect("invariant: writing to a Vec succeeds");
    let body = encoder
        .finish()
        .expect("invariant: finishing a gzip stream into a Vec succeeds");
    Page {
        status: StatusCode::OK,
        content_type: "application/gzip",
        headers: Vec::new(),
        body: body.into(),
        held: None,
    }
}

fn index(origin: &str) -> String {
    let mut text = String::with_capacity(64 * 50_048);
    text.push_str(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<sitemapindex xmlns=\"http://www.sitemaps.org/schemas/sitemap/0.9\">\n",
    );
    for number in 1..=SITEMAPS {
        writeln!(
            text,
            "<sitemap><loc>{origin}/sitemaps/{number}.xml</loc></sitemap>"
        )
        .expect("invariant: writing to a String succeeds");
    }
    text.push_str("</sitemapindex>\n");
    text
}

fn sitemap(origin: &str, number: u64) -> String {
    let mut text = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<urlset xmlns=\"http://www.sitemaps.org/schemas/sitemap/0.9\">\n",
    );
    if lists_item(number) {
        writeln!(text, "<url><loc>{origin}/items/{number}</loc></url>")
            .expect("invariant: writing to a String succeeds");
    }
    text.push_str("</urlset>\n");
    text
}

fn item(seed: u64, number: u64) -> String {
    let (id, name) = (item_id(number), item_name(seed, number));
    document(
        &name,
        &format!(
            "<main>\n<article class=\"entry\" data-id=\"{id}\">\n<h1 class=\"name\">{name}</h1>\n</article>\n</main>\n"
        ),
    )
}

pub(super) fn tasks(seed: u64, port: Option<u16>) -> Vec<Task> {
    let records = || -> Vec<Record> {
        (1..=SITEMAPS)
            .filter(|number| lists_item(*number))
            .map(|number| {
                Record::from([("id", item_id(number)), ("name", item_name(seed, number))])
            })
            .collect()
    };
    let description = "Crawl the items the sitemaps of this sitemap index list and return one \
        Record per item, with the fields `id` and `name`. The index lists more sitemaps than \
        the protocol's 50,000; most of them are empty.";
    vec![
        crawl_task(
            (HOST, FAMILY),
            1,
            port,
            "/sitemap-index.xml",
            description,
            records(),
        ),
        crawl_task(
            (HOST, FAMILY),
            2,
            port,
            "/sitemap-index.xml.gz",
            description,
            records(),
        ),
    ]
}
