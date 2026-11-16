//! The page suite's `page.read` cases (ADR 0147, issue 2402), on the
//! Proving ground's catalog: twenty fields of the extract task's product
//! page answer in one call, each the value the page serves; an absent
//! field is null with its reason in `unread` while the others are read;
//! and the rows of a listing page keep each row that lacks a field, with
//! its other fields read. What each field should read is taken from the
//! page as the Proving ground serves it.

use goodfetch_protocol::page::read::{PAGE_READ, PageRead, UnreadField, UnreadReason};
use goodfetch_protocol::page::{PageId, PageOpened};
use serde_json::{Map, Value, json};

use super::{Checks, Site, finish_run, new_page, start_run};
use crate::protocol::connection::{Connection, Gone};

/// The task whose start page is a product page.
const PRODUCT_TASK: &str = "catalog/2";

/// The task whose start page is the catalog's first listing page.
const LISTING_TASK: &str = "catalog/1";

/// A case of these lines, by its ID in the page suite's case list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Case {
    TwentyFields,
    AbsentField,
    Rows,
}

impl Case {
    /// The case `id` names, if it is one of these lines.
    pub(super) fn from_id(id: &str) -> Option<Self> {
        Some(match id {
            "twenty-fields-of-a-product-page-are-read-in-one-call" => Self::TwentyFields,
            "an-absent-field-is-null-with-its-reason-and-the-others-are-read" => Self::AbsentField,
            "a-listing-pages-rows-keep-a-row-that-lacks-a-field" => Self::Rows,
            _ => return None,
        })
    }

    /// Runs the case on the open Connection against `site`.
    pub(super) fn run(
        self,
        connection: &mut Connection,
        site: &Site,
        checks: &mut Checks,
    ) -> Result<(), Gone> {
        match self {
            Self::TwentyFields => twenty_fields(connection, site, checks),
            Self::AbsentField => absent_field(connection, site, checks),
            Self::Rows => rows(connection, site, checks),
        }
    }
}

/// What the product page shows, read from its HTML as served.
struct Product {
    origin: String,
    id: String,
    name: String,
    price: String,
    /// The listing page it links back to.
    page: String,
}

impl Product {
    /// What `html`, the product page served at `url`, shows.
    fn of(url: &str, html: &str) -> Option<Self> {
        Some(Self {
            origin: origin_of(url).to_owned(),
            id: between(html, "<article class=\"product\" data-id=\"", "\"")?.to_owned(),
            name: between(html, "<h1 class=\"name\">", "</h1>")?.to_owned(),
            price: between(html, "data-price=\"", "\"")?.to_owned(),
            page: between(html, "<a href=\"/catalog/", "\"")?.to_owned(),
        })
    }
}

/// The text of `html` between the first `start` and the next `end`.
fn between<'a>(html: &'a str, start: &str, end: &str) -> Option<&'a str> {
    let from = html.find(start)? + start.len();
    let length = html[from..].find(end)?;
    Some(&html[from..from + length])
}

/// The origin of an `http` URL: its scheme, host and port.
fn origin_of(url: &str) -> &str {
    let host = url.find("://").map_or(0, |at| at + "://".len());
    url[host..].find('/').map_or(url, |at| &url[..host + at])
}

/// A value field read with `locator`, its `kind` `css` or `xpath`, and
/// `extra` keys beside it: `attribute` or `match`.
fn field(kind: &str, selector: &str, extra: &Value) -> Value {
    let mut field = json!({ "locator": { "kind": kind, "selector": selector } });
    if let (Some(field), Some(extra)) = (field.as_object_mut(), extra.as_object()) {
        field.extend(extra.clone());
    }
    field
}

/// The start page of `task`, served as its HTML, opened on a new page of
/// the Run `run_id`: its URL, its HTML and the page's ID.
fn on_the_start_page(
    connection: &mut Connection,
    site: &Site,
    task: &str,
    run_id: &str,
    checks: &mut Checks,
) -> Result<Option<(String, String, PageId)>, Gone> {
    let url = site.start_url(task).map(ToOwned::to_owned);
    checks.check(
        format!("the Proving ground serves task `{task}`"),
        url.is_some(),
    );
    let Some(url) = url else {
        return Ok(None);
    };
    let Some(html) = checks.read(
        "the task's start page is served",
        site.proving_ground.page(&url),
    ) else {
        return Ok(None);
    };
    start_run(connection, site, run_id, &json!({}), checks)?;
    let Some(page) = new_page(connection, run_id, checks)? else {
        return Ok(None);
    };
    let went = connection.goto(&page, &url)?;
    checks.check(
        format!("the start page of `{task}` loads"),
        went["result"]["response"]["status"] == json!(200),
    );
    let PageOpened { page_id, .. } = page;
    Ok(Some((url, html, page_id)))
}

/// `page.read` of `fields` on `page`, as its result type holds it.
fn read(
    connection: &mut Connection,
    page: &PageId,
    fields: &Value,
    checks: &mut Checks,
) -> Result<Option<PageRead>, Gone> {
    let answer = connection.call(
        PAGE_READ.name,
        &json!({ "page_id": page, "fields": fields }),
    )?;
    let read = answer
        .get("result")
        .and_then(|result| serde_json::from_value::<PageRead>(result.clone()).ok());
    checks.check(
        format!(
            "`{}` answers with what its result type holds",
            PAGE_READ.name
        ),
        read.is_some(),
    );
    Ok(read)
}

/// Twenty fields of the product page, CSS and `XPath` locators, text and
/// attributes and `match` groups, read in one `page.read`: each is what the
/// page serves, and none is unread.
fn twenty_fields(
    connection: &mut Connection,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let run_id = "conformance-read-twenty-fields";
    let Some((url, html, page)) =
        on_the_start_page(connection, site, PRODUCT_TASK, run_id, checks)?
    else {
        return Ok(());
    };
    let product = Product::of(&url, &html);
    checks.check("the product page shows a product", product.is_some());
    if let Some(Product {
        origin,
        id,
        name,
        price,
        page: listing,
    }) = product
    {
        let none = json!({});
        let attribute = |name: &str| json!({ "attribute": name });
        let group = |pattern: &str| json!({ "match": { "pattern": pattern } });
        let fields = json!({
            "title": field("css", "title", &none),
            "name": field("css", "h1.name", &none),
            "name_by_xpath": field("xpath", "//h1[@class='name']", &none),
            "id": field("css", "span.id", &none),
            "id_by_xpath": field("xpath", "//span[@class='id']", &none),
            "data_id": field("css", "article.product", &attribute("data-id")),
            "data_id_by_xpath": field("xpath", "//article", &attribute("data-id")),
            "price_text": field("css", "span.price", &none),
            "price": field("css", "span.price", &attribute("data-price")),
            "price_by_xpath": field("xpath", "//span[@class='price']", &attribute("data-price")),
            "price_digits": field("css", "span.price", &group(r"\$([0-9.]+)")),
            "id_number": field("css", "span.id", &group(r"P-([0-9]+)")),
            "product_label": field("xpath", "//article/p[1]", &none),
            "price_label": field("xpath", "//article/p[2]", &none),
            "back": field("css", "main > a", &none),
            "back_href": field("css", "main > a", &attribute("href")),
            "lang": field("css", "html", &attribute("lang")),
            "charset": field("css", "meta", &attribute("charset")),
            "article_class": field("css", "article", &attribute("class")),
            "heading_class": field("xpath", "//h1", &attribute("class")),
        });
        checks.check(
            "the read declares twenty fields",
            fields.as_object().map(Map::len) == Some(20),
        );
        let number = id.strip_prefix("P-").unwrap_or_default().to_owned();
        let expected = json!({
            "title": name,
            "name": name,
            "name_by_xpath": name,
            "id": id,
            "id_by_xpath": id,
            "data_id": id,
            "data_id_by_xpath": id,
            "price_text": format!("${price}"),
            "price": price,
            "price_by_xpath": price,
            "price_digits": price,
            "id_number": number,
            "product_label": format!("Product ID: {id}"),
            "price_label": format!("Price: ${price}"),
            "back": format!("Back to page {listing} of the catalog"),
            "back_href": format!("{origin}/catalog/{listing}"),
            "lang": "en",
            "charset": "utf-8",
            "article_class": "product",
            "heading_class": "name",
        });
        if let Some(read) = read(connection, &page, &fields, checks)? {
            checks.check(
                "one `page.read` answers the twenty values the page serves",
                Value::Object(read.values) == expected,
            );
            checks.check("no field of the twenty is unread", read.unread.is_empty());
        }
    }
    finish_run(connection, run_id)
}

/// The product page's name and price read beside a field with no element,
/// one whose element lacks its attribute and one whose `match` does not
/// match: those three are null, each named in `unread` with its reason,
/// and the name and price are read.
fn absent_field(connection: &mut Connection, site: &Site, checks: &mut Checks) -> Result<(), Gone> {
    let run_id = "conformance-read-absent-field";
    let Some((url, html, page)) =
        on_the_start_page(connection, site, PRODUCT_TASK, run_id, checks)?
    else {
        return Ok(());
    };
    let product = Product::of(&url, &html);
    checks.check("the product page shows a product", product.is_some());
    if let Some(product) = product {
        let none = json!({});
        let fields = json!({
            "name": field("css", "h1.name", &none),
            "price": field("css", "span.price", &json!({ "attribute": "data-price" })),
            "rating": field("css", ".rating", &none),
            "stock": field("css", "h1.name", &json!({ "attribute": "data-stock" })),
            "sku": field("css", "span.id", &json!({ "match": { "pattern": "SKU-([0-9]+)" } })),
        });
        if let Some(read) = read(connection, &page, &fields, checks)? {
            checks.check(
                "the fields the page gives are read beside the absent ones",
                Value::Object(read.values)
                    == json!({
                        "name": product.name,
                        "price": product.price,
                        "rating": null,
                        "stock": null,
                        "sku": null,
                    }),
            );
            let unread = |field: &str, reason: UnreadReason, text: Option<&str>| UnreadField {
                field: field.to_owned(),
                reason,
                text: text.map(ToOwned::to_owned),
            };
            let mut expected = vec![
                unread("/rating", UnreadReason::NoElement, None),
                unread("/sku", UnreadReason::NoMatch, Some(&product.id)),
                unread("/stock", UnreadReason::NoAttribute, None),
            ];
            let mut named = read.unread;
            named.sort_by(|one, other| one.field.cmp(&other.field));
            expected.sort_by(|one, other| one.field.cmp(&other.field));
            checks.check(
                "`unread` names each null field with its reason, and only those",
                named == expected,
            );
        }
    }
    finish_run(connection, run_id)
}

/// One row the listing page shows: its product's name, price and link.
fn listed(html: &str) -> Vec<(String, String, String)> {
    html.split("<li class=\"product\"")
        .skip(1)
        .filter_map(|row| {
            let link = between(row, "<a href=\"", "\"")?;
            Some((
                between(row, &format!("{link}\">"), "</a>")?.to_owned(),
                between(row, "<span class=\"price\">", "</span>")?.to_owned(),
                link.to_owned(),
            ))
        })
        .collect()
}

/// The listing page's products as rows of a name, a price, a link and a
/// blurb no row has, with a list of their names: every row is kept, its
/// blurb null and named in `unread`, its other fields read.
fn rows(connection: &mut Connection, site: &Site, checks: &mut Checks) -> Result<(), Gone> {
    let run_id = "conformance-read-rows";
    let Some((url, html, page)) =
        on_the_start_page(connection, site, LISTING_TASK, run_id, checks)?
    else {
        return Ok(());
    };
    let listed = listed(&html);
    checks.check("the listing page lists products", !listed.is_empty());
    let origin = origin_of(&url);
    let none = json!({});
    let fields = json!({
        "products": {
            "rows": { "kind": "css", "selector": "li.product" },
            "fields": {
                "name": field("css", "a", &none),
                "price": field("xpath", "./span[@class='price']", &none),
                "link": field("css", "a", &json!({ "attribute": "href" })),
                "blurb": field("css", ".blurb", &none),
            },
        },
        "names": { "list": field("css", "li.product a", &none) },
    });
    if let Some(read) = read(connection, &page, &fields, checks)? {
        let rows: Vec<Value> = listed
            .iter()
            .map(|(name, price, link)| {
                json!({
                    "name": name,
                    "price": price,
                    "link": format!("{origin}{link}"),
                    "blurb": null,
                })
            })
            .collect();
        let names: Vec<&str> = listed.iter().map(|(name, ..)| name.as_str()).collect();
        checks.check(
            "each row is kept with the fields it has, its blurb null",
            Value::Object(read.values) == json!({ "products": rows, "names": names }),
        );
        let blurbs: Vec<UnreadField> = (0..listed.len())
            .map(|row| UnreadField {
                field: format!("/products/{row}/blurb"),
                reason: UnreadReason::NoElement,
                text: None,
            })
            .collect();
        checks.check(
            "`unread` names each row's missing blurb, and only those",
            read.unread == blurbs,
        );
    }
    finish_run(connection, run_id)
}
