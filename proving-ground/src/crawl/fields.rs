//! The `fields` family on `fields.test` (pitfall D07): 24 listings posted
//! over time, eight a page at `/listings?page=<1..3>`, oldest first, each at
//! `/listings/<n>`. The site gained fields as it went: the first eight
//! listings show a `name`, the next eight also a `price`, and the last
//! eight a `rating` as well, so a later Record carries fields earlier ones
//! lack, and an export whose columns are fixed by its first Record drops
//! them.

use std::fmt::Write as _;

use super::{Page, Visit, canonical_number, crawl_task, document, name};
use crate::generator;
use crate::task::{Record, Task};

pub(super) const HOST: &str = "fields.test";
pub(super) const FAMILY: &str = "fields";

/// The generator places of the family's values.
const FAMILY_PLACE: u64 = 29;
const PRICE_PLACE: u64 = 1;
const RATING_PLACE: u64 = 2;

const PAGES: u64 = 3;
const PER_PAGE: u64 = 8;
const LISTINGS: u64 = PAGES * PER_PAGE;

fn listing_id(number: u64) -> String {
    format!("D-{number:02}")
}

/// The fields listing `number` shows, `id` aside, in the order its page
/// shows them: those of the batch it was posted in.
fn fields(seed: u64, number: u64) -> Vec<(&'static str, String)> {
    let mut fields = vec![("name", name(seed, &[FAMILY_PLACE, number]))];
    if number > PER_PAGE {
        let cents = 100 + generator::below(seed, &[FAMILY_PLACE, number, PRICE_PLACE], 49_900);
        fields.push(("price", format!("{}.{:02}", cents / 100, cents % 100)));
    }
    if number > 2 * PER_PAGE {
        let tenths = 10 + generator::below(seed, &[FAMILY_PLACE, number, RATING_PLACE], 41);
        fields.push(("rating", format!("{}.{}", tenths / 10, tenths % 10)));
    }
    fields
}

pub(super) fn respond(visit: &Visit<'_>) -> Page {
    let request = visit.request;
    if request.path == "/listings" {
        return request
            .parameter("page")
            .and_then(canonical_number)
            .filter(|page| (1..=PAGES).contains(page))
            .map_or_else(Page::not_found, |page| Page::ok(listings(visit.seed, page)));
    }
    request
        .path
        .strip_prefix("/listings/")
        .and_then(canonical_number)
        .filter(|number| (1..=LISTINGS).contains(number))
        .map_or_else(Page::not_found, |number| {
            Page::ok(listing(visit.seed, number))
        })
}

fn listings(seed: u64, page: u64) -> String {
    let mut body =
        format!("<main>\n<h1>Listings, page {page} of {PAGES}</h1>\n<ul class=\"listings\">\n");
    for number in (page - 1) * PER_PAGE + 1..=page * PER_PAGE {
        writeln!(
            body,
            "<li><a href=\"/listings/{number}\">{}</a></li>",
            name(seed, &[FAMILY_PLACE, number])
        )
        .expect("invariant: writing to a String succeeds");
    }
    body.push_str("</ul>\n");
    if page < PAGES {
        writeln!(
            body,
            "<a rel=\"next\" href=\"/listings?page={}\">Newer listings</a>",
            page + 1
        )
        .expect("invariant: writing to a String succeeds");
    }
    body.push_str("</main>\n");
    document(&format!("Listings, page {page}"), &body)
}

fn listing(seed: u64, number: u64) -> String {
    let id = listing_id(number);
    let mut body = format!(
        "<main>\n<article class=\"entry\" data-id=\"{id}\">\n<h1>Listing {id}</h1>\n<dl>\n"
    );
    for (field, value) in fields(seed, number) {
        writeln!(body, "<dt>{field}</dt><dd class=\"{field}\">{value}</dd>")
            .expect("invariant: writing to a String succeeds");
    }
    body.push_str("</dl>\n</article>\n</main>\n");
    document(&format!("Listing {id}"), &body)
}

pub(super) fn tasks(seed: u64, port: Option<u16>) -> Vec<Task> {
    vec![crawl_task(
        (HOST, FAMILY),
        1,
        port,
        "/listings?page=1",
        "Crawl the listings from the first page, following each page's link to newer \
            listings, and return one Record per listing, with the field `id` and every field \
            its page shows. Newer listings show fields older ones lack.",
        (1..=LISTINGS)
            .map(|number| {
                let mut record = Record::from([("id", listing_id(number))]);
                record.extend(fields(seed, number));
                record
            })
            .collect(),
    )]
}
