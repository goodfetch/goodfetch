//! The `catalog` family on `catalog.test`, ported from the lab's HTTP catalog:
//! ten listing pages of ten products each, every product with its ID, name
//! and price drawn from the seed, an order form for each product, and the
//! family's tasks with their expected results, derived from the same seed
//! data and never from a tool's output.

use std::fmt::Write as _;

use crate::generator;
use crate::task::{self, Effect, Entry, Expected, Kind, Record, Task};

/// The host name the family is served on, which tests map to loopback with
/// the `resolve` setting (ADR 0059).
pub(crate) const HOST: &str = "catalog.test";

const FAMILY: &str = "catalog";

const LISTING_PAGES: u64 = 10;
const PRODUCTS_PER_PAGE: u64 = 10;
const PRODUCTS: u64 = LISTING_PAGES * PRODUCTS_PER_PAGE;

/// The generator places of the family's values: the family's own first, then
/// the product's number (0 for the family's own values) and the value's.
const FAMILY_PLACE: u64 = 1;
const ADJECTIVE_PLACE: u64 = 1;
const NOUN_PLACE: u64 = 2;
const PRICE_PLACE: u64 = 3;
/// Which product the extract task's page shows.
const EXTRACT_PLACE: u64 = 4;
/// Which product the order task orders.
const ORDER_PLACE: u64 = 5;

const ADJECTIVES: [&str; 16] = [
    "Amber", "Brisk", "Cedar", "Dusky", "Ember", "Frosted", "Gilded", "Hazel", "Ivory", "Jade",
    "Keen", "Lunar", "Mellow", "Nimble", "Olive", "Plush",
];
const NOUNS: [&str; 16] = [
    "Anchor", "Basket", "Candle", "Desk", "Easel", "Flask", "Globe", "Hammock", "Inkwell", "Jug",
    "Kettle", "Lamp", "Mirror", "Notebook", "Ottoman", "Pitcher",
];

/// One product, as its pages show it.
struct Product {
    id: String,
    name: String,
    /// In dollars, with two decimals.
    price: String,
}

/// The product numbered `number` (from 1) for `seed`.
fn product(seed: u64, number: u64) -> Product {
    let draw = |value, bound| generator::below(seed, &[FAMILY_PLACE, number, value], bound);
    let adjective = ADJECTIVES[index(draw(ADJECTIVE_PLACE, ADJECTIVES.len() as u64))];
    let noun = NOUNS[index(draw(NOUN_PLACE, NOUNS.len() as u64))];
    let cents = 100 + draw(PRICE_PLACE, 99_900);
    Product {
        id: product_id(number),
        name: format!("{adjective} {noun}"),
        price: format!("{}.{:02}", cents / 100, cents % 100),
    }
}

fn index(value: u64) -> usize {
    usize::try_from(value).expect("invariant: a word index is below 16")
}

fn product_id(number: u64) -> String {
    format!("P-{number:04}")
}

/// The page at `path` for `seed`, generated on request; `None` when the
/// family has no such page.
pub(crate) fn page(seed: u64, path: &str) -> Option<String> {
    if let Some(page) = path.strip_prefix("/catalog/") {
        let page = canonical_number(page)?;
        (1..=LISTING_PAGES)
            .contains(&page)
            .then(|| listing_page(seed, page))
    } else if let Some(rest) = path.strip_prefix("/products/") {
        match rest.strip_suffix("/order") {
            Some(id) => product_number(id).map(|number| order_page(seed, number)),
            None => product_number(rest).map(|number| product_page(seed, number)),
        }
    } else {
        None
    }
}

/// The number of the product `P-NNNN`, if the family has it.
fn product_number(id: &str) -> Option<u64> {
    let digits = id.strip_prefix("P-")?;
    let number = digits.parse::<u64>().ok()?;
    (digits.len() == 4
        && digits.bytes().all(|byte| byte.is_ascii_digit())
        && (1..=PRODUCTS).contains(&number))
    .then_some(number)
}

/// Places an order from the order form's body (`product=P-NNNN`, form
/// encoded): the Effect and its confirmation page; `None` when the body
/// names no product the family has.
pub(crate) fn order(body: &[u8]) -> Option<(Effect, String)> {
    let body = std::str::from_utf8(body).ok()?;
    let product = body
        .split('&')
        .filter_map(|pair| pair.split_once('='))
        .find(|(name, _)| *name == "product")
        .map(|(_, value)| value)?;
    product_number(product)?;
    let page = format!(
        "<!doctype html>\n<html lang=\"en\">\n<head><meta charset=\"utf-8\"><title>Order placed</title></head>\n<body>\n<main>\n<h1>Order placed</h1>\n<p>Your order of <span class=\"id\">{product}</span> is placed.</p>\n</main>\n</body>\n</html>\n"
    );
    Some((
        Effect::Order {
            product: product.to_owned(),
        },
        page,
    ))
}

/// A number written without a sign or leading zeros, so each page has one URL.
fn canonical_number(text: &str) -> Option<u64> {
    let number = text.parse::<u64>().ok()?;
    (number.to_string() == text).then_some(number)
}

fn listing_page(seed: u64, page: u64) -> String {
    let mut html = format!(
        "<!doctype html>\n<html lang=\"en\">\n<head><meta charset=\"utf-8\"><title>Catalog, page {page} of {LISTING_PAGES}</title></head>\n<body>\n<main id=\"catalog\">\n<h1>Catalog</h1>\n<ul class=\"products\">\n"
    );
    for number in (page - 1) * PRODUCTS_PER_PAGE + 1..=page * PRODUCTS_PER_PAGE {
        let Product { id, name, price } = product(seed, number);
        writeln!(
            html,
            "<li class=\"product\" data-id=\"{id}\"><a href=\"/products/{id}\">{name}</a> <span class=\"price\">${price}</span></li>"
        )
        .expect("invariant: writing to a String succeeds");
    }
    html.push_str("</ul>\n<nav class=\"pagination\">");
    if page > 1 {
        write!(
            html,
            "<a rel=\"prev\" href=\"/catalog/{}\">Previous</a>",
            page - 1
        )
        .expect("invariant: writing to a String succeeds");
    }
    if page < LISTING_PAGES {
        write!(
            html,
            "<a rel=\"next\" href=\"/catalog/{}\">Next</a>",
            page + 1
        )
        .expect("invariant: writing to a String succeeds");
    }
    html.push_str("</nav>\n</main>\n</body>\n</html>\n");
    html
}

fn product_page(seed: u64, number: u64) -> String {
    let Product { id, name, price } = product(seed, number);
    let page = (number - 1) / PRODUCTS_PER_PAGE + 1;
    format!(
        "<!doctype html>\n<html lang=\"en\">\n<head><meta charset=\"utf-8\"><title>{name}</title></head>\n<body>\n<main>\n<article class=\"product\" data-id=\"{id}\">\n<h1 class=\"name\">{name}</h1>\n<p>Product ID: <span class=\"id\">{id}</span></p>\n<p>Price: <span class=\"price\" data-price=\"{price}\">${price}</span></p>\n</article>\n<a href=\"/catalog/{page}\">Back to page {page} of the catalog</a>\n</main>\n</body>\n</html>\n"
    )
}

/// A product's order form, which orders it once when submitted.
fn order_page(seed: u64, number: u64) -> String {
    let Product { id, name, price } = product(seed, number);
    format!(
        "<!doctype html>\n<html lang=\"en\">\n<head><meta charset=\"utf-8\"><title>Order {name}</title></head>\n<body>\n<main>\n<h1>Order {name}</h1>\n<p>Price: <span class=\"price\">${price}</span></p>\n<form method=\"post\" action=\"/orders\">\n<input type=\"hidden\" name=\"product\" value=\"{id}\">\n<button type=\"submit\">Place the order</button>\n</form>\n</main>\n</body>\n</html>\n"
    )
}

/// The host name of the family's task `ordinal`, on which the task's pages
/// are served: `1.catalog.test` for `catalog/1`.
fn task_host(ordinal: u32) -> String {
    format!("{ordinal}.{HOST}")
}

/// The family's tasks for `seed`, their start URLs on their own host names
/// and `port`, or on no port when `tasks` lists them without serving.
/// Ordinals are append-only: a new task takes the next one, and
/// a removed one is never reused (ADR 0063).
pub(crate) fn tasks(seed: u64, port: Option<u16>) -> Vec<Task> {
    let origin = |ordinal| task::origin(&task_host(ordinal), port);
    let extracted = 1 + generator::below(seed, &[FAMILY_PLACE, 0, EXTRACT_PLACE], PRODUCTS);
    let ordered = 1 + generator::below(seed, &[FAMILY_PLACE, 0, ORDER_PLACE], PRODUCTS);
    vec![
        Task {
            host: task_host(1),
            entry: Entry {
                id: format!("{FAMILY}/1"),
                family: FAMILY,
                kind: Kind::Crawl,
                start_url: format!("{}/catalog/1", origin(1)),
                description: "Crawl the catalog from its first listing page, following each page's \
                    Next link, and return one Record per product, read from its product page, \
                    with the fields `id`, `name` and `price`.",
                account: None,
                time_steps: None,
            },
            expected: Expected::records(
                (1..=PRODUCTS).map(|number| record(seed, number)).collect(),
            ),
        },
        Task {
            host: task_host(2),
            entry: Entry {
                id: format!("{FAMILY}/2"),
                family: FAMILY,
                kind: Kind::Extract,
                start_url: format!("{}/products/{}", origin(2), product_id(extracted)),
                description: "Extract the product this page shows as one Record with the fields \
                    `id`, `name` and `price`.",
                account: None,
                time_steps: None,
            },
            expected: Expected::records(vec![record(seed, extracted)]),
        },
        Task {
            host: task_host(3),
            entry: Entry {
                id: format!("{FAMILY}/3"),
                family: FAMILY,
                kind: Kind::Submit,
                start_url: format!("{}/products/{}/order", origin(3), product_id(ordered)),
                description: "Order the product this page offers, once, through its order form.",
                account: None,
                time_steps: None,
            },
            expected: Expected::effects(vec![Effect::Order {
                product: product_id(ordered),
            }]),
        },
    ]
}

/// The expected Record of a product: its fields as its product page shows
/// them, the price without its currency sign.
fn record(seed: u64, number: u64) -> Record {
    let Product { id, name, price } = product(seed, number);
    Record::from([("id", id), ("name", name), ("price", price)])
}
