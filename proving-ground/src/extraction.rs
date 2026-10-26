//! The `extraction` family on `extraction.test`: ADR 0025's Extraction
//! cases on seeded products (J-011). Product pages carry schema.org JSON-LD
//! and Open Graph metadata; a shop lists products only its hidden JSON API
//! holds; pages whose class names churn from page to page; the same kind of
//! entity under six small structure changes; a huge page generated on
//! request, far past ADR 0062's 256 KiB; and one product's regional
//! variants, each in its locale's language, currency and number format;
//! and pages whose main content's Markdown the lab wrote by hand (ADR 0007),
//! judged by their Markdown's structure; and pages read in every form
//! `page.formats` gives, each written by hand too; and pages whose links,
//! images, media and tables are read, one behind a redirect, also by hand;
//! and pages whose metadata and contacts are read, by hand too; and one
//! page whose values a code-free crawl's fields read, also by hand.
//! Every other expected Record comes from the same seed data as the pages.

mod content_pages;
mod markdown_pages;
mod reference_pages;
mod structured_pages;
mod value_pages;

use std::fmt::Write as _;

use serde_json::json;

use crate::generator;
use crate::task::{Entry, Expected, Kind, Record, Task, origin};

/// The host name the family is served on.
pub(crate) const HOST: &str = "extraction.test";

const FAMILY: &str = "extraction";
const FAMILY_PLACE: u64 = 21;

/// The generator places of a product's values.
const ADJECTIVE_PLACE: u64 = 1;
const NOUN_PLACE: u64 = 2;
const PRICE_PLACE: u64 = 3;
const BRAND_PLACE: u64 = 4;
const GTIN_PLACE: u64 = 5;
const RATING_PLACE: u64 = 6;
const DESCRIPTION_PLACE: u64 = 7;
const CLASS_PLACE: u64 = 8;
/// A regional price's place, after the region's number.
const REGION_PRICE_PLACE: u64 = 9;

/// The products of each case, by number: 12 per case, the huge page's
/// 2,000, and the one product sold in every region.
const METADATA: std::ops::RangeInclusive<u64> = 1..=12;
const HIDDEN_API: std::ops::RangeInclusive<u64> = 13..=24;
const CHURN: std::ops::RangeInclusive<u64> = 25..=36;
const VARIANTS: std::ops::RangeInclusive<u64> = 37..=48;
const HUGE: std::ops::RangeInclusive<u64> = 1001..=3000;
const REGIONAL: u64 = 100;
/// Products per page of the hidden API.
const API_PAGE: u64 = 6;

const ADJECTIVES: [&str; 16] = [
    "Arctic", "Bold", "Coral", "Daring", "Eager", "Fabled", "Gentle", "Humble", "Indigo", "Jolly",
    "Kindred", "Lively", "Misty", "Noble", "Orchid", "Proud",
];
const NOUNS: [&str; 16] = [
    "Atlas", "Beacon", "Compass", "Drum", "Engine", "Fountain", "Garden", "Harbor", "Island",
    "Jacket", "Kite", "Lantern", "Meadow", "Needle", "Orbit", "Prism",
];
const BRANDS: [&str; 8] = [
    "Northwind",
    "Bluefield",
    "Copperline",
    "Driftwood",
    "Evergreen",
    "Foxglove",
    "Graniteworks",
    "Hollowpeak",
];
const QUALITIES: [&str; 8] = [
    "sturdy",
    "light",
    "handmade",
    "weatherproof",
    "compact",
    "elegant",
    "durable",
    "quiet",
];
const USES: [&str; 8] = [
    "for daily use",
    "for long trips",
    "for small kitchens",
    "for the garden",
    "for the workshop",
    "for reading at night",
    "for rainy days",
    "for a busy desk",
];

/// One product, as its pages show it.
struct Product {
    id: String,
    name: String,
    brand: &'static str,
    gtin: String,
    /// In cents.
    cents: u64,
    /// Out of 5, with one decimal.
    rating: String,
    description: String,
}

impl Product {
    /// In dollars, with two decimals.
    fn price(&self) -> String {
        format!("{}.{:02}", self.cents / 100, self.cents % 100)
    }
}

fn draw(seed: u64, number: u64, value: u64, bound: u64) -> u64 {
    generator::below(seed, &[FAMILY_PLACE, number, value], bound)
}

fn word<const N: usize>(words: &[&'static str; N], value: u64) -> &'static str {
    words[usize::try_from(value).expect("invariant: a word index is below its list's length")]
}

/// The product numbered `number` for `seed`.
fn product(seed: u64, number: u64) -> Product {
    let pick = |value, words_len: usize| draw(seed, number, value, words_len as u64);
    let digits: String = (0..12)
        .map(|digit| {
            char::from(
                b'0' + u8::try_from(draw(seed, number, GTIN_PLACE * 100 + digit, 10))
                    .expect("invariant: a digit is below 10"),
            )
        })
        .collect();
    let rating = 10 + draw(seed, number, RATING_PLACE, 41);
    Product {
        id: format!("X-{number:04}"),
        name: format!(
            "{} {}",
            word(&ADJECTIVES, pick(ADJECTIVE_PLACE, ADJECTIVES.len())),
            word(&NOUNS, pick(NOUN_PLACE, NOUNS.len()))
        ),
        brand: word(&BRANDS, pick(BRAND_PLACE, BRANDS.len())),
        gtin: format!("{digits}{}", check_digit(&digits)),
        cents: 100 + draw(seed, number, PRICE_PLACE, 99_900),
        rating: format!("{}.{}", rating / 10, rating % 10),
        description: format!(
            "A {} {} {}.",
            word(&QUALITIES, pick(DESCRIPTION_PLACE, QUALITIES.len())),
            word(&NOUNS, pick(NOUN_PLACE, NOUNS.len())).to_lowercase(),
            word(&USES, pick(DESCRIPTION_PLACE + 100, USES.len()))
        ),
    }
}

/// The GTIN-13 check digit of twelve digits (GS1's modulo 10).
fn check_digit(digits: &str) -> u32 {
    let sum: u32 = digits
        .chars()
        .filter_map(|digit| digit.to_digit(10))
        .zip([1, 3].into_iter().cycle())
        .map(|(digit, weight)| digit * weight)
        .sum();
    (10 - sum % 10) % 10
}

/// The number of the product `X-NNNN` in `range`, if it is one.
fn number_in(id: &str, range: &std::ops::RangeInclusive<u64>) -> Option<u64> {
    let digits = id.strip_prefix("X-")?;
    let number = digits.parse::<u64>().ok()?;
    (digits.len() == 4
        && digits.bytes().all(|byte| byte.is_ascii_digit())
        && range.contains(&number))
    .then_some(number)
}

/// The page at `path`, as its `Content-Type` and body; `None` when the
/// family has no such page.
pub(crate) fn page(seed: u64, path: &str) -> Option<(&'static str, Vec<u8>)> {
    let html = |page: String| ("text/html; charset=utf-8", page.into_bytes());
    let page = match path {
        "/products" => index("Products", "/products", METADATA),
        "/shop" => shop(),
        "/churn" => churn_index(seed),
        "/variants" => index("Variants", "/variants", VARIANTS),
        "/huge" => huge(seed),
        _ if path.starts_with(markdown_pages::INDEX) => markdown_pages::page(path)?,
        _ if path.starts_with(content_pages::INDEX) => content_pages::page(path)?,
        _ if path.starts_with(reference_pages::INDEX) => reference_pages::page(path)?,
        _ if path.starts_with(structured_pages::INDEX) => structured_pages::page(path)?,
        value_pages::PATH => value_pages::page(path)?,
        _ => return routed(seed, path),
    };
    Some(html(page))
}

/// Where the page at `path` moves, a path on the same host the family
/// answers `302 Found` with; `None` for a page that does not move.
pub(crate) fn moved(path: &str) -> Option<String> {
    reference_pages::moved(path)
}

fn routed(seed: u64, path: &str) -> Option<(&'static str, Vec<u8>)> {
    let html = |page: String| ("text/html; charset=utf-8", page.into_bytes());
    if let Some(id) = path.strip_prefix("/products/") {
        return number_in(id, &METADATA).map(|number| html(metadata_page(seed, number)));
    }
    if let Some(written) = path.strip_prefix("/api/products/") {
        let page = written
            .parse::<u64>()
            .ok()
            .filter(|page| page.to_string() == written)?;
        return api_page(seed, page).map(|body| ("application/json", body.into_bytes()));
    }
    if let Some(id) = path.strip_prefix("/churn/") {
        return number_in(id, &CHURN).map(|number| html(churn_page(seed, number)));
    }
    if let Some(id) = path.strip_prefix("/variants/") {
        return number_in(id, &VARIANTS).map(|number| html(variant_page(seed, number)));
    }
    let (region, rest) = path.strip_prefix('/')?.split_once('/')?;
    let region = REGIONS.iter().find(|candidate| candidate.id == region)?;
    let id = rest.strip_prefix("products/")?;
    (number_in(id, &(REGIONAL..=REGIONAL)).is_some()).then(|| html(regional_page(seed, region)))
}

fn head(lang: &str, title: &str, extra: &str) -> String {
    format!(
        "<!doctype html>\n<html lang=\"{lang}\">\n<head>\n<meta charset=\"utf-8\">\n<title>{title}</title>\n{extra}</head>\n<body>\n<main>\n"
    )
}

const TAIL: &str = "</main>\n</body>\n</html>\n";

fn index(title: &str, prefix: &str, products: std::ops::RangeInclusive<u64>) -> String {
    let mut html = head("en", title, "");
    writeln!(html, "<ul class=\"products\">").expect("invariant: writing to a String succeeds");
    for number in products {
        writeln!(
            html,
            "<li><a href=\"{prefix}/X-{number:04}\">X-{number:04}</a></li>"
        )
        .expect("invariant: writing to a String succeeds");
    }
    html.push_str("</ul>\n");
    html.push_str(TAIL);
    html
}

/// A product page with its schema.org Product in JSON-LD and its Open
/// Graph metadata; the brand, GTIN, rating and currency are in the JSON-LD
/// alone, the description in the metadata alone.
fn metadata_page(seed: u64, number: u64) -> String {
    let product = product(seed, number);
    let price = product.price();
    let data = json!({
        "@context": "https://schema.org",
        "@type": "Product",
        "name": product.name,
        "sku": product.id,
        "gtin13": product.gtin,
        "brand": { "@type": "Brand", "name": product.brand },
        "aggregateRating": { "@type": "AggregateRating", "ratingValue": product.rating, "reviewCount": "12" },
        "offers": {
            "@type": "Offer",
            "price": price,
            "priceCurrency": "USD",
            "availability": "https://schema.org/InStock",
        },
    });
    let metadata = format!(
        "<meta name=\"description\" content=\"{description}\">\n<meta property=\"og:type\" content=\"product\">\n<meta property=\"og:title\" content=\"{name}\">\n<meta property=\"og:description\" content=\"{description}\">\n<script type=\"application/ld+json\">{data}</script>\n",
        description = product.description,
        name = product.name,
    );
    let mut html = head("en", &product.name, &metadata);
    write!(
        html,
        "<article class=\"product\">\n<h1>{name}</h1>\n<p>SKU: {id}</p>\n<p>Price: ${price}</p>\n</article>\n",
        name = product.name,
        id = product.id,
    )
    .expect("invariant: writing to a String succeeds");
    html.push_str(TAIL);
    html
}

/// The shop, whose script loads its products from the hidden API, page by
/// page; its HTML lists none.
fn shop() -> String {
    let mut html = head("en", "Shop", "");
    html.push_str(
        "<h1>Shop</h1>\n<ul id=\"products\"></ul>\n<script>\n(async () => {\n  const list = document.getElementById(\"products\");\n  let next = \"/api/products/1\";\n  while (next) {\n    const page = await (await fetch(next)).json();\n    for (const product of page.products) {\n      const item = document.createElement(\"li\");\n      item.className = \"product\";\n      item.dataset.id = product.id;\n      item.textContent = `${product.name}: $${product.price}`;\n      list.append(item);\n    }\n    next = page.next;\n  }\n})();\n</script>\n",
    );
    html.push_str(TAIL);
    html
}

/// The hidden API's page `page`, from 1.
fn api_page(seed: u64, page: u64) -> Option<String> {
    let pages = (HIDDEN_API.end() - HIDDEN_API.start() + 1) / API_PAGE;
    if !(1..=pages).contains(&page) {
        return None;
    }
    let first = HIDDEN_API.start() + (page - 1) * API_PAGE;
    let products: Vec<_> = (first..first + API_PAGE)
        .map(|number| {
            let product = product(seed, number);
            json!({ "id": product.id, "name": product.name, "price": product.price() })
        })
        .collect();
    let next = (page < pages).then(|| format!("/api/products/{}", page + 1));
    Some(json!({ "products": products, "next": next }).to_string())
}

/// A class name drawn for one element of one page, so no two pages share
/// one.
fn churned(seed: u64, number: u64, element: u64) -> String {
    format!(
        "c{:06x}",
        draw(seed, number, CLASS_PLACE * 100 + element, 1 << 24)
    )
}

fn churn_index(seed: u64) -> String {
    let mut html = head("en", "Churn", "");
    writeln!(html, "<ul class=\"{}\">", churned(seed, 0, 0))
        .expect("invariant: writing to a String succeeds");
    for number in CHURN {
        writeln!(
            html,
            "<li class=\"{class}\"><a class=\"{link}\" href=\"/churn/X-{number:04}\">X-{number:04}</a></li>",
            class = churned(seed, number, 10),
            link = churned(seed, number, 11),
        )
        .expect("invariant: writing to a String succeeds");
    }
    html.push_str("</ul>\n");
    html.push_str(TAIL);
    html
}

fn churn_page(seed: u64, number: u64) -> String {
    let product = product(seed, number);
    let class = |element| churned(seed, number, element);
    let mut html = head("en", &product.name, "");
    write!(
        html,
        "<div class=\"{a}\">\n<div class=\"{b}\">\n<h2 class=\"{c}\">{name}</h2>\n<div class=\"{d}\"><span class=\"{e}\">SKU</span> <span class=\"{f}\">{id}</span></div>\n<div class=\"{g}\"><span class=\"{h}\">Price</span> <span class=\"{i}\">${price}</span></div>\n</div>\n</div>\n",
        a = class(1),
        b = class(2),
        c = class(3),
        d = class(4),
        e = class(5),
        f = class(6),
        g = class(7),
        h = class(8),
        i = class(9),
        name = product.name,
        id = product.id,
        price = product.price(),
    )
    .expect("invariant: writing to a String succeeds");
    html.push_str(TAIL);
    html
}

/// A product in one of six layouts, by its number: the same fields, moved,
/// wrapped, tabulated, listed, reordered or split.
fn variant_page(seed: u64, number: u64) -> String {
    let product = product(seed, number);
    let Product { id, name, .. } = &product;
    let price = product.price();
    let body = match number % 6 {
        0 => format!(
            "<article>\n<h1>{name}</h1>\n<p>SKU: {id}</p>\n<p>Price: ${price}</p>\n</article>\n"
        ),
        1 => format!(
            "<div class=\"wrap\"><div class=\"inner\">\n<article>\n<h1>{name}</h1>\n<p>SKU: {id}</p>\n<p>Price: ${price}</p>\n</article>\n</div></div>\n"
        ),
        2 => format!(
            "<table>\n<tr><th>Name</th><td>{name}</td></tr>\n<tr><th>SKU</th><td>{id}</td></tr>\n<tr><th>Price</th><td>${price}</td></tr>\n</table>\n"
        ),
        3 => format!(
            "<dl>\n<dt>Name</dt><dd>{name}</dd>\n<dt>SKU</dt><dd>{id}</dd>\n<dt>Price</dt><dd>${price}</dd>\n</dl>\n"
        ),
        4 => format!(
            "<section>\n<p>Price: ${price}</p>\n<h2>{name}</h2>\n<p>SKU: {id}</p>\n</section>\n"
        ),
        _ => format!(
            "<article>\n<h1>{name}</h1>\n<p>SKU: {id}</p>\n<p>Price: $<span class=\"whole\">{}</span>.<span class=\"cents\">{:02}</span></p>\n</article>\n",
            product.cents / 100,
            product.cents % 100
        ),
    };
    let mut html = head("en", name, "");
    html.push_str(&body);
    html.push_str(TAIL);
    html
}

/// The huge page: 2,000 products in one table, each row with a long
/// description, generated on request and never stored (ADR 0062).
fn huge(seed: u64) -> String {
    let mut html = head("en", "Every product", "");
    html.push_str("<table class=\"products\">\n");
    for number in HUGE {
        let product = product(seed, number);
        let blurb = product.description.repeat(72);
        writeln!(
            html,
            "<tr class=\"product\" data-id=\"{id}\"><td class=\"name\">{name}</td><td class=\"price\">${price}</td><td class=\"blurb\">{blurb}</td></tr>",
            id = product.id,
            name = product.name,
            price = product.price(),
        )
        .expect("invariant: writing to a String succeeds");
    }
    html.push_str("</table>\n");
    html.push_str(TAIL);
    html
}

/// One region the regional product is sold in: its language, currency,
/// label and price format (CLDR's for its locale).
struct Region {
    id: &'static str,
    lang: &'static str,
    currency: &'static str,
    label: &'static str,
    /// The amount's minor units (2 for cents, 0 for yen).
    decimals: u32,
    group: &'static str,
    decimal: &'static str,
    /// Indian grouping: the last three digits, then pairs.
    indian: bool,
    /// The format, with `{}` for the number.
    format: &'static str,
}

static REGIONS: [Region; 5] = [
    Region {
        id: "us",
        lang: "en-US",
        currency: "USD",
        label: "Price",
        decimals: 2,
        group: ",",
        decimal: ".",
        indian: false,
        format: "${}",
    },
    Region {
        id: "de",
        lang: "de-DE",
        currency: "EUR",
        label: "Preis",
        decimals: 2,
        group: ".",
        decimal: ",",
        indian: false,
        format: "{}\u{A0}€",
    },
    Region {
        id: "fr",
        lang: "fr-FR",
        currency: "EUR",
        label: "Prix",
        decimals: 2,
        group: "\u{202F}",
        decimal: ",",
        indian: false,
        format: "{}\u{A0}€",
    },
    Region {
        id: "jp",
        lang: "ja-JP",
        currency: "JPY",
        label: "価格",
        decimals: 0,
        group: ",",
        decimal: "",
        indian: false,
        format: "\u{FFE5}{}",
    },
    Region {
        id: "in",
        lang: "en-IN",
        currency: "INR",
        label: "Price",
        decimals: 2,
        group: ",",
        decimal: ".",
        indian: true,
        format: "\u{20B9}{}",
    },
];

/// The regional product's amount in a region's minor units.
fn regional_amount(seed: u64, number: u64) -> u64 {
    100 + draw(seed, REGIONAL, REGION_PRICE_PLACE * 100 + number, 1_999_900)
}

/// An amount as a plain decimal, in its region's minor units.
fn plain(amount: u64, decimals: u32) -> String {
    if decimals == 0 {
        return amount.to_string();
    }
    let unit = 10_u64.pow(decimals);
    format!(
        "{}.{:0width$}",
        amount / unit,
        amount % unit,
        width = decimals as usize
    )
}

/// An amount as its region shows it.
fn shown(amount: u64, region: &Region) -> String {
    let unit = 10_u64.pow(region.decimals);
    let whole = (amount / unit).to_string();
    let mut groups = Vec::new();
    let mut rest = whole.as_str();
    let mut size = 3;
    while rest.len() > size {
        let (head, tail) = rest.split_at(rest.len() - size);
        groups.push(tail);
        rest = head;
        if region.indian {
            size = 2;
        }
    }
    groups.push(rest);
    groups.reverse();
    let mut number = groups.join(region.group);
    if region.decimals > 0 {
        write!(
            number,
            "{}{:0width$}",
            region.decimal,
            amount % unit,
            width = region.decimals as usize
        )
        .expect("invariant: writing to a String succeeds");
    }
    region.format.replace("{}", &number)
}

fn regional_page(seed: u64, region: &Region) -> String {
    let product = product(seed, REGIONAL);
    let number = REGIONS
        .iter()
        .zip(1..)
        .find_map(|(candidate, number)| (candidate.id == region.id).then_some(number))
        .expect("invariant: the region is one of the family's");
    let mut alternates = String::new();
    for other in &REGIONS {
        writeln!(
            alternates,
            "<link rel=\"alternate\" hreflang=\"{lang}\" href=\"/{id}/products/{product}\">",
            lang = other.lang,
            id = other.id,
            product = product.id,
        )
        .expect("invariant: writing to a String succeeds");
    }
    writeln!(
        alternates,
        "<link rel=\"alternate\" hreflang=\"x-default\" href=\"/us/products/{}\">",
        product.id
    )
    .expect("invariant: writing to a String succeeds");
    let mut html = head(region.lang, &product.name, &alternates);
    write!(
        html,
        "<article itemscope itemtype=\"https://schema.org/Product\">\n<h1 itemprop=\"name\">{name}</h1>\n<p itemprop=\"offers\" itemscope itemtype=\"https://schema.org/Offer\">{label}: <span class=\"price\">{price}</span><meta itemprop=\"priceCurrency\" content=\"{currency}\"></p>\n</article>\n",
        name = product.name,
        label = region.label,
        price = shown(regional_amount(seed, number), region),
        currency = region.currency,
    )
    .expect("invariant: writing to a String succeeds");
    html.push_str(TAIL);
    html
}

/// The host name of the family's task `ordinal`.
fn task_host(ordinal: u32) -> String {
    format!("{ordinal}.{HOST}")
}

/// A Record of a product's `id`, `name` and `price`.
fn listed(seed: u64, number: u64) -> Record {
    let product = product(seed, number);
    let price = product.price();
    Record::from([("id", product.id), ("name", product.name), ("price", price)])
}

/// The expected Records of the product pages with metadata.
fn metadata_records(seed: u64) -> Vec<Record> {
    METADATA
        .map(|number| {
            let product = product(seed, number);
            let price = product.price();
            Record::from([
                ("id", product.id),
                ("name", product.name),
                ("price", price),
                ("currency", "USD".to_owned()),
                ("brand", product.brand.to_owned()),
                ("gtin", product.gtin),
                ("rating", product.rating),
                ("description", product.description),
            ])
        })
        .collect()
}

/// The expected Record of the regional product in each region.
fn regional_records(seed: u64) -> Vec<Record> {
    REGIONS
        .iter()
        .zip(1..)
        .map(|(region, number)| {
            Record::from([
                ("id", region.id.to_owned()),
                ("language", region.lang.to_owned()),
                ("currency", region.currency.to_owned()),
                (
                    "price",
                    plain(regional_amount(seed, number), region.decimals),
                ),
            ])
        })
        .collect()
}

/// The family's tasks for `seed`, on their own host names and `port`.
/// Ordinals are append-only (ADR 0063).
pub(crate) fn tasks(seed: u64, port: Option<u16>) -> Vec<Task> {
    let task = |ordinal: u32, kind, path: &str, description, records| Task {
        host: task_host(ordinal),
        entry: Entry {
            id: format!("{FAMILY}/{ordinal}"),
            family: FAMILY,
            kind,
            start_url: format!("{}{path}", origin(&task_host(ordinal), port)),
            description,
            account: None,
            time_steps: None,
        },
        expected: Expected::records(records),
    };
    let listing =
        |range: std::ops::RangeInclusive<u64>| range.map(|number| listed(seed, number)).collect();
    vec![
        task(
            1,
            Kind::Crawl,
            "/products",
            "Crawl every product page the index links and return one Record per product with \
                the fields `id`, `name`, `price` (a decimal), `currency`, `brand`, `gtin`, \
                `rating` and `description`, read from the page's text, its schema.org JSON-LD \
                and its Open Graph metadata.",
            metadata_records(seed),
        ),
        task(
            2,
            Kind::Extract,
            "/shop",
            "Extract every product the shop lists once its script has loaded them, as one \
                Record each with the fields `id`, `name` and `price` (a decimal).",
            listing(HIDDEN_API),
        ),
        task(
            3,
            Kind::Crawl,
            "/churn",
            "Crawl every product page the index links, whose class names differ from page to \
                page, and return one Record per product with the fields `id`, `name` and \
                `price` (a decimal).",
            listing(CHURN),
        ),
        task(
            4,
            Kind::Crawl,
            "/variants",
            "Crawl every product page the index links, each laid out a little differently, \
                and return one Record per product with the fields `id`, `name` and `price` (a \
                decimal).",
            listing(VARIANTS),
        ),
        task(
            5,
            Kind::Extract,
            "/huge",
            "Extract every product of this one huge page, as one Record each with the fields \
                `id`, `name` and `price` (a decimal).",
            listing(HUGE),
        ),
        task(
            6,
            Kind::Crawl,
            &format!("/us/products/X-{REGIONAL:04}"),
            "Follow the page's alternate links to the product's page in each region and \
                return one Record per region with the fields `id` (the region, the path's first \
                segment), `language` (the page's language tag), `currency` (its ISO 4217 code) \
                and `price`, a plain decimal with a full stop, read from the price as the \
                region's locale writes it.",
            regional_records(seed),
        ),
        Task {
            expected: Expected {
                markdown_site: Some(HOST),
                ..Expected::records(markdown_pages::records())
            },
            ..task(
                7,
                Kind::Crawl,
                markdown_pages::INDEX,
                "Crawl every page the index links and return one Record per page with the \
                    fields `id` (the page's path) and `markdown`, the page's main content as \
                    Markdown: empty for a page with no main content. Markdown is judged by its \
                    structure (headings, paragraphs, list items, table cells, code blocks and \
                    links), whatever syntax writes it.",
                Vec::new(),
            )
        },
        content_modes_task(port),
        references_task(port),
        structured_task(port),
    ]
}

/// Task 10: the structured pages, each read for its metadata and contacts
/// (spec #580, ticket #787).
fn structured_task(port: Option<u16>) -> Task {
    const ORDINAL: u32 = 10;
    let origin = origin(&task_host(ORDINAL), port);
    Task {
        host: task_host(ORDINAL),
        entry: Entry {
            id: format!("{FAMILY}/{ORDINAL}"),
            family: FAMILY,
            kind: Kind::Crawl,
            start_url: format!("{origin}{}", structured_pages::INDEX),
            description: "Crawl every page the index links and return one Record per page with \
                the fields `id` (the path the index links) and, as JSON objects: `metadata`, \
                `{ \"title\", \"description\", \"canonical\", \"lang\", \"open_graph\", \
                \"twitter\", \"json_ld\", \"microdata\", \"meta\" }`, the first `<title>`'s \
                text, the `description` meta tag, the canonical link resolved, the root's \
                `lang` (each `null` when absent), the Open Graph and Twitter meta tags as \
                `{ \"property\", \"content\" }` in document order, each JSON-LD block as JSON, \
                each top-level microdata item as `{ \"type\", \"id\", \"properties\" }` by the \
                HTML Standard's JSON conversion, and every named meta tag neither Open Graph's nor Twitter's, by its name in lower case; and \
                `contacts`, `{ \"emails\", \"phones\", \"social\" }`, each value once with the \
                text it was read from (its link's text, else the value as written): e-mail \
                addresses from `mailto:` links and text, telephone numbers from `tel:` links \
                and text with `e164` only when the number carries its country code or the \
                page's language tag has a region (a national number in text with neither is \
                no number), and social profiles with their `network`. Each field is judged as \
                a JSON value.",
            account: None,
            time_steps: None,
        },
        expected: Expected::records(structured_pages::records(&origin)),
    }
}

/// Task 9: the reference pages, each read for its links, images, media
/// and tables (spec #580; pitfall U08).
fn references_task(port: Option<u16>) -> Task {
    const ORDINAL: u32 = 9;
    let origin = origin(&task_host(ORDINAL), port);
    Task {
        host: task_host(ORDINAL),
        entry: Entry {
            id: format!("{FAMILY}/{ORDINAL}"),
            family: FAMILY,
            kind: Kind::Crawl,
            start_url: format!("{origin}{}", reference_pages::INDEX),
            description: "Crawl every page the index links and return one Record per page with \
                the fields `id` (the path the index links, before any redirect) and, as JSON \
                arrays in document order: `links`, each `<a href>` and `<area href>` as \
                `{ \"url\", \"href\", \"text\", \"rel\" }`; `images`, each `<img src>` as \
                `{ \"url\", \"src\", \"alt\" }`; `media`, each `<video>` and `<audio>` source \
                as `{ \"url\", \"src\", \"kind\" (\"video\" or \"audio\"), \"type\" }`; and \
                `tables`, each table as `{ \"caption\", \"header_rows\", \"body_rows\" }`, a \
                row a list of its cells' text, a row's header its first cell. `href` and `src` \
                are as the page writes them, character references decoded; `url` is that \
                value resolved against the document's base URL (its `<base href>`, else its \
                URL after redirects) by the WHATWG URL Standard, `null` when it does not \
                parse; `alt`, `type` and `caption` are `null` when absent; text has its white \
                space collapsed. Each field is judged as a JSON value.",
            account: None,
            time_steps: None,
        },
        expected: Expected::records(reference_pages::records(&origin)),
    }
}

/// Task 8: the content-mode pages, each read in every form `page.formats`
/// gives (spec #580).
fn content_modes_task(port: Option<u16>) -> Task {
    const ORDINAL: u32 = 8;
    Task {
        host: task_host(ORDINAL),
        entry: Entry {
            id: format!("{FAMILY}/{ORDINAL}"),
            family: FAMILY,
            kind: Kind::Crawl,
            start_url: format!(
                "{}{}",
                origin(&task_host(ORDINAL), port),
                content_pages::INDEX
            ),
            description: "Crawl every page the index links and return one Record per page with \
                the fields `id` (the page's path); `markdown`, the page's main content as \
                Markdown; `markdown_full`, the whole body as Markdown; `markdown_fit`, the main \
                content as Markdown without its link-dense blocks (more than half their text in \
                links), its image-only blocks and the navigation lists the page repeats; \
                `html_raw`, the page's HTML as it stands; `html_cleaned`, its HTML with what ad \
                and annoyance filter lists hide taken out; `html_main`, the main content's HTML; \
                and `text`, the main content's text. Markdown is judged by its structure, HTML \
                by the text of its blocks and text white space aside, whatever syntax writes \
                each.",
            account: None,
            time_steps: None,
        },
        expected: Expected {
            markdown_site: Some(HOST),
            ..Expected::records(content_pages::records())
        },
    }
}
