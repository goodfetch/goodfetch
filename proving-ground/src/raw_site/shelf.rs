//! A shelf: the three-page catalog the lab's incident, access-policy and
//! identity sites share, ported from its fixtures. Pages `/catalog/1` to
//! `/catalog/3` show two Records each, every one with its ID and a price
//! drawn from the seed, and link the next page with `rel="next"`.

use std::fmt::Write as _;

use crate::generator;
use crate::task::Record;

/// How many pages a shelf has.
pub(super) const PAGES: u64 = 3;
const PER_PAGE: u64 = 2;

/// One task's shelf.
pub(super) struct Shelf {
    /// The Records' ID prefix, one per family.
    pub(super) prefix: &'static str,
    /// The family's generator place.
    pub(super) place: u64,
    pub(super) ordinal: u64,
}

impl Shelf {
    /// The Record numbered `number` (from 1): its ID and price.
    fn record(&self, seed: u64, number: u64) -> Record {
        let cents = 100 + generator::below(seed, &[self.place, self.ordinal, number], 99_900);
        Record::from([
            ("id", format!("{}-{number:03}", self.prefix)),
            ("price", format!("{}.{:02}", cents / 100, cents % 100)),
        ])
    }

    /// Every Record of the shelf, in page order.
    pub(super) fn records(&self, seed: u64) -> Vec<Record> {
        (1..=PAGES)
            .flat_map(|page| self.records_of(seed, page))
            .collect()
    }

    /// The Records page `page` (from 1) shows.
    pub(super) fn records_of(&self, seed: u64, page: u64) -> Vec<Record> {
        let first = (page - 1) * PER_PAGE + 1;
        (first..first + PER_PAGE)
            .map(|number| self.record(seed, number))
            .collect()
    }

    /// Page `page`'s HTML, its link to the next page at `next`, with `head`
    /// in its `<head>`.
    pub(super) fn page(&self, seed: u64, page: u64, next: Option<&str>, head: &str) -> String {
        let mut html = format!(
            "<!doctype html><html><head><meta charset=\"utf-8\"><title>Shelf, page {page}</title>{head}</head><body><main id=\"catalog\">"
        );
        for record in self.records_of(seed, page) {
            let _ = write!(
                html,
                "<article data-id=\"{id}\"><h2 class=\"id\">{id}</h2><span class=\"price\">{price}</span></article>",
                id = record["id"],
                price = record["price"],
            );
        }
        if let Some(next) = next {
            let _ = write!(
                html,
                "<a class=\"next\" rel=\"next\" href=\"{next}\">Next</a>"
            );
        }
        html.push_str("</main></body></html>");
        html
    }

    /// Page `page`'s HTML as [`Self::page`] makes it, linking the next page
    /// under `/catalog/` when there is one.
    pub(super) fn plain_page(&self, seed: u64, page: u64, head: &str) -> String {
        let next = (page < PAGES).then(|| format!("/catalog/{}", page + 1));
        self.page(seed, page, next.as_deref(), head)
    }
}

/// The page number of a shelf path `<base>/catalog/<n>`, if it is one.
pub(super) fn page_number(path: &str, base: &str) -> Option<u64> {
    path.strip_prefix(base)?
        .strip_prefix("/catalog/")?
        .parse()
        .ok()
        .filter(|page| (1..=PAGES).contains(page))
}

/// The byte offset in `html` where its link to the next page starts, or
/// two fifths of it: a body cut there lacks the link.
pub(super) fn before_next(html: &str) -> usize {
    html.find("<a class=\"next\"").unwrap_or(html.len() * 2 / 5)
}
