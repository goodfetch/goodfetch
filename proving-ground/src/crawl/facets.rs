//! The `facets` family on `facets.test` (ADR 0120's faceted listing): a shop
//! of 24 products, all listed by `/shop`, whose listing links, for every
//! facet it has not set, each of that facet's values, appended to its own
//! query: `/shop?colour=red`, then `/shop?colour=red&size=m`, and so on, in
//! any order. Four facets (colour, size, material and sort order) make
//! hundreds of listings even once a frontier sorts their parameters, and
//! each product page links back into them. Nothing but a bound the crawl
//! sets stops it: a path budget, or an exclude pattern naming the facet
//! parameters (ADR 0120 gives facets no default rule).
//!
//! Each request of a task for a faceted listing (`/shop` with a query) past
//! the task's allowance records a `trap` finding.

use std::fmt::Write as _;

use super::{Counts, Page, Visit, canonical_number, crawl_task, document, name};
use crate::generator;
use crate::task::{Finding, Record, Task};

pub(super) const HOST: &str = "facets.test";
pub(super) const FAMILY: &str = "facets";

/// The generator place of the family's values.
const FAMILY_PLACE: u64 = 28;

const PRODUCTS: u64 = 24;
/// The faceted listings a task's crawl may request.
const FACET_ALLOWANCE: u64 = 24;

/// Each facet's parameter name and values.
const FACETS: [(&str, &[&str]); 4] = [
    ("colour", &["red", "blue", "green", "black"]),
    ("size", &["s", "m", "l", "xl"]),
    ("material", &["wool", "cotton", "linen"]),
    ("sort", &["price", "name", "new"]),
];

/// What the family keeps for one `serve`: each task's faceted listing
/// requests.
#[derive(Default)]
pub(super) struct State {
    listings: Counts,
}

fn product_id(number: u64) -> String {
    format!("F-{number:02}")
}

fn product_name(seed: u64, number: u64) -> String {
    name(seed, &[FAMILY_PLACE, number])
}

/// The value product `number` has of the filtering facet at `facet`.
fn value_of(seed: u64, number: u64, facet: usize) -> &'static str {
    let values = FACETS[facet].1;
    let at = generator::below(
        seed,
        &[FAMILY_PLACE, number, facet as u64 + 1],
        values.len() as u64,
    );
    values[usize::try_from(at).expect("invariant: a value index is below 4")]
}

pub(super) fn respond(visit: &Visit<'_>, state: &State) -> Page {
    let request = visit.request;
    match request.path {
        "/shop" => {
            let Some(set) = facets(request.query) else {
                return Page::not_found();
            };
            if !set.is_empty() && state.listings.add(request.host) > FACET_ALLOWANCE {
                visit.find(Finding::Trap {
                    target: request.target(),
                });
            }
            Page::ok(listing(visit.seed, request.query, &set))
        }
        path => path
            .strip_prefix("/products/")
            .and_then(canonical_number)
            .filter(|number| (1..=PRODUCTS).contains(number))
            .map_or_else(Page::not_found, |number| {
                Page::ok(product(visit.seed, number))
            }),
    }
}

/// The facets a listing's query sets, each facet's index and value index;
/// `None` for an unknown facet or value, or a facet set twice.
fn facets(query: &str) -> Option<Vec<(usize, usize)>> {
    let mut set: Vec<(usize, usize)> = Vec::new();
    for pair in query.split('&').filter(|pair| !pair.is_empty()) {
        let (name, value) = pair.split_once('=')?;
        let facet = FACETS.iter().position(|(facet, _)| *facet == name)?;
        let value = FACETS[facet].1.iter().position(|known| *known == value)?;
        if set.iter().any(|(earlier, _)| *earlier == facet) {
            return None;
        }
        set.push((facet, value));
    }
    Some(set)
}

fn listing(seed: u64, query: &str, set: &[(usize, usize)]) -> String {
    let mut body = String::from("<main>\n<h1>Shop</h1>\n<nav class=\"facets\">\n");
    let prefix = if query.is_empty() {
        "/shop?".to_owned()
    } else {
        format!("/shop?{query}&amp;")
    };
    for (facet, (parameter, values)) in FACETS.iter().enumerate() {
        if set.iter().any(|(earlier, _)| *earlier == facet) {
            continue;
        }
        for value in *values {
            writeln!(
                body,
                "<a href=\"{prefix}{parameter}={value}\">{parameter}: {value}</a>"
            )
            .expect("invariant: writing to a String succeeds");
        }
    }
    body.push_str("</nav>\n<ul class=\"products\">\n");
    // The sort order filters nothing; the other facets keep the products
    // that have their value.
    let filters = |number: u64| {
        set.iter().all(|(facet, value)| {
            FACETS[*facet].0 == "sort" || value_of(seed, number, *facet) == FACETS[*facet].1[*value]
        })
    };
    for number in (1..=PRODUCTS).filter(|number| filters(*number)) {
        writeln!(
            body,
            "<li><a href=\"/products/{number}\">{}</a></li>",
            product_name(seed, number)
        )
        .expect("invariant: writing to a String succeeds");
    }
    body.push_str("</ul>\n</main>\n");
    document("Shop", &body)
}

fn product(seed: u64, number: u64) -> String {
    let (id, name) = (product_id(number), product_name(seed, number));
    let (colour, size) = (value_of(seed, number, 0), value_of(seed, number, 1));
    document(
        &name,
        &format!(
            "<main>\n<article class=\"entry\" data-id=\"{id}\">\n<h1 class=\"name\">{name}</h1>\n</article>\n<a href=\"/shop?colour={colour}\">More in {colour}</a>\n<a href=\"/shop?size={size}\">More in size {size}</a>\n</main>\n"
        ),
    )
}

pub(super) fn tasks(seed: u64, port: Option<u16>) -> Vec<Task> {
    vec![crawl_task(
        (HOST, FAMILY),
        1,
        port,
        "/shop",
        "Crawl the products this shop lists and return one Record per product, with the \
            fields `id` and `name`. Its listing links every combination of its facets \
            (`colour`, `size`, `material` and `sort`), which list no product `/shop` does not; \
            request at most 24 faceted listings (`/shop` with a query), bounding them with a \
            path budget or an exclude pattern.",
        (1..=PRODUCTS)
            .map(|number| {
                Record::from([
                    ("id", product_id(number)),
                    ("name", product_name(seed, number)),
                ])
            })
            .collect(),
    )]
}
