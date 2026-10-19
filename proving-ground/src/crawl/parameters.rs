//! The `parameters` family on `parameters.test` (ADR 0005): four listing
//! pages of twelve products, each linked in three colours,
//! `/product?id=P-NNNN&colour=<1..3>&ref=listing`. `ref` never changes the
//! page. Most products come in one colour, and a product page shows its
//! first colour for a colour it lacks, so on the first listing page, where
//! every product has one colour, `colour` looks like `ref`. Further on, some
//! products have a distinct page per colour, with its own ID and price: a
//! crawl that learned `colour` has no effect merges them.

use std::fmt::Write as _;

use super::{Page, Visit, canonical_number, crawl_task, document, name, numbered};
use crate::generator;
use crate::task::{Record, Task};

pub(super) const HOST: &str = "parameters.test";
pub(super) const FAMILY: &str = "parameters";

/// The generator places of the family's values.
const FAMILY_PLACE: u64 = 24;
const COLOURS_PLACE: u64 = 1;
const PRICE_PLACE: u64 = 2;
const FORCED_PLACE: u64 = 3;

const LISTING_PAGES: u64 = 4;
const PER_PAGE: u64 = 12;
const PRODUCTS: u64 = LISTING_PAGES * PER_PAGE;
const COLOURS: [&str; 3] = ["Ivory", "Slate", "Coral"];

fn product_id(number: u64) -> String {
    format!("P-{number:04}")
}

/// How many colours product `number` comes in: one on the first listing
/// page, and one, two or three further on, one product in three for sure.
fn colours(seed: u64, number: u64) -> u64 {
    if number <= PER_PAGE {
        return 1;
    }
    let forced = PER_PAGE
        + 1
        + generator::below(seed, &[FAMILY_PLACE, 0, FORCED_PLACE], PRODUCTS - PER_PAGE);
    if number == forced {
        return 3;
    }
    match generator::below(seed, &[FAMILY_PLACE, number, COLOURS_PLACE], 4) {
        0 => 2,
        1 => 3,
        _ => 1,
    }
}

/// One product page: a product in one of its colours.
struct Sku {
    id: String,
    name: String,
    price: String,
}

fn sku(seed: u64, number: u64, colour: u64) -> Sku {
    let colour_name = COLOURS[usize::try_from(colour - 1).expect("invariant: a colour below 4")];
    let cents = 100 + generator::below(seed, &[FAMILY_PLACE, number, PRICE_PLACE, colour], 99_900);
    Sku {
        id: format!("{}-{colour}", product_id(number)),
        name: format!("{}, {colour_name}", name(seed, &[FAMILY_PLACE, number])),
        price: format!("{}.{:02}", cents / 100, cents % 100),
    }
}

pub(super) fn respond(visit: &Visit<'_>) -> Page {
    let request = visit.request;
    match request.path {
        "/" => Page::ok(document(
            "Products",
            "<main>\n<a href=\"/products?page=1\">Products</a>\n</main>\n",
        )),
        "/products" => match request.parameter("page").and_then(canonical_number) {
            Some(page) if (1..=LISTING_PAGES).contains(&page) => {
                Page::ok(listing(visit.seed, page))
            }
            _ => Page::not_found(),
        },
        "/product" => {
            let number = request
                .parameter("id")
                .and_then(|id| numbered(id, "P-", 4, PRODUCTS));
            let colour = request
                .parameter("colour")
                .map_or(Some(1), canonical_number);
            match (number, colour) {
                (Some(number), Some(colour)) if (1..=3).contains(&colour) => {
                    let shown = if colour <= colours(visit.seed, number) {
                        colour
                    } else {
                        1
                    };
                    Page::ok(product_page(visit.seed, number, shown))
                }
                _ => Page::not_found(),
            }
        }
        _ => Page::not_found(),
    }
}

fn listing(seed: u64, page: u64) -> String {
    let mut body = format!(
        "<main>\n<h1>Products, page {page} of {LISTING_PAGES}</h1>\n<ul class=\"products\">\n"
    );
    for number in (page - 1) * PER_PAGE + 1..=page * PER_PAGE {
        let id = product_id(number);
        write!(
            body,
            "<li class=\"product\">{}",
            name(seed, &[FAMILY_PLACE, number])
        )
        .expect("invariant: writing to a String succeeds");
        for (colour, colour_name) in (1..).zip(COLOURS) {
            write!(
                body,
                " <a href=\"/product?id={id}&amp;colour={colour}&amp;ref=listing\">{colour_name}</a>"
            )
            .expect("invariant: writing to a String succeeds");
        }
        body.push_str("</li>\n");
    }
    body.push_str("</ul>\n");
    if page < LISTING_PAGES {
        writeln!(
            body,
            "<a rel=\"next\" href=\"/products?page={}\">Next</a>",
            page + 1
        )
        .expect("invariant: writing to a String succeeds");
    }
    body.push_str("</main>\n");
    document(&format!("Products, page {page}"), &body)
}

fn product_page(seed: u64, number: u64, colour: u64) -> String {
    let Sku { id, name, price } = sku(seed, number, colour);
    document(
        &name,
        &format!(
            "<main>\n<article class=\"product\" data-id=\"{id}\">\n<h1 class=\"name\">{name}</h1>\n<p>Price: <span class=\"price\" data-price=\"{price}\">${price}</span></p>\n</article>\n</main>\n"
        ),
    )
}

pub(super) fn tasks(seed: u64, port: u16) -> Vec<Task> {
    vec![crawl_task(
        (HOST, FAMILY),
        1,
        port,
        "/products?page=1",
        "Crawl the products from the first listing page, following each page's Next link, and \
            return one Record per distinct product page, with the fields `id`, `name` and \
            `price`. Each listing links every product in three colours; a product page shows \
            the product's first colour for a colour it lacks.",
        (1..=PRODUCTS)
            .flat_map(|number| (1..=colours(seed, number)).map(move |colour| (number, colour)))
            .map(|(number, colour)| {
                let Sku { id, name, price } = sku(seed, number, colour);
                Record::from([("id", id), ("name", name), ("price", price)])
            })
            .collect(),
    )]
}
