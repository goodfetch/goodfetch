//! The code-free crawl's declarations as functions of its extraction: the
//! Dataset `pages` declared from the formats request, a Record of a page
//! checked against it, and the sitemap seeds `follow: sitemap` adds.

use std::collections::BTreeMap;

use goodfetch_dataset::Datasets;
use goodfetch_protocol::foxhound::crawl::{CrawlExtraction, PushedRecord, Seed};
use goodfetch_protocol::page::PageFormats;
use goodfetch_protocol::run::PointInTime;
use serde_json::{Map, Value, json};

use super::{dataset, declare_into, record_of, sitemap_seeds};

fn extraction(value: Value) -> CrawlExtraction {
    serde_json::from_value(value).unwrap()
}

fn formats(value: Value) -> PageFormats {
    serde_json::from_value(value).unwrap()
}

fn pushed(data: Value) -> Vec<PushedRecord> {
    vec![PushedRecord {
        dataset: "pages".to_owned(),
        data,
    }]
}

/// The Dataset `pages` is keyed by `url`, its schema generated from the
/// formats request: a page's Record holding the formats asked for, its
/// status and when it was read checks against it, while one missing a
/// format asked for, or holding one not asked for, is refused.
///
/// Covers: story:6-304
#[test]
fn the_pages_schema_is_generated_from_the_formats_request_and_a_pages_record_holds_to_it() {
    let extraction = extraction(json!({ "formats": { "markdown": true, "links": true } }));
    let declared = dataset(&extraction);
    let datasets = Datasets::declare(std::slice::from_ref(&declared)).unwrap();
    let page = formats(json!({
        "url": "http://site.test/a",
        "base_url": "http://site.test/a",
        "main_content": "found",
        "markdown": "# A",
        "links": [{ "url": "http://site.test/b", "href": "/b", "text": "B", "rel": [] }],
    }));
    let at = PointInTime::from_rfc3339("2026-10-26T10:00:00Z".to_owned());
    let record = record_of(&page, Some(200), &at, Map::new());
    let mut without_links = record.clone();
    without_links.as_object_mut().unwrap().remove("links");
    let mut with_html = record.clone();
    with_html["html"] = json!("<p>A</p>");

    assert_eq!(
        (
            declared.name.as_str(),
            declared.key.clone(),
            record["status"].clone(),
            record["fetchedAt"].clone(),
            datasets.check(pushed(record)).is_ok(),
            datasets.check(pushed(without_links)).is_ok(),
            datasets.check(pushed(with_html)).is_ok(),
        ),
        (
            "pages",
            vec!["url".to_owned()],
            json!(200),
            json!("2026-10-26T10:00:00Z"),
            true,
            false,
            false,
        )
    );
}

/// The start declares the Label `page` and the Dataset `pages`, and
/// refuses a Dataset of the program's own with `invalid_params`.
///
/// Covers: story:6-304
#[test]
fn the_start_declares_page_and_pages_and_refuses_a_dataset_of_its_own() {
    let extraction = extraction(json!({ "formats": { "markdown": true } }));
    let (mut labels, mut datasets) = (BTreeMap::new(), Vec::new());
    declare_into(&extraction, &mut labels, &mut datasets).unwrap();
    let refused = declare_into(&extraction, &mut BTreeMap::new(), &mut datasets.clone());
    assert_eq!(
        (
            labels.keys().cloned().collect::<Vec<_>>(),
            datasets
                .iter()
                .map(|declared| declared.name.clone())
                .collect::<Vec<_>>(),
            refused.map_err(|error| error.code()),
        ),
        (
            vec!["page".to_owned()],
            vec!["pages".to_owned()],
            Err("invalid_params"),
        )
    );
}

/// `follow: sitemap` adds one robots.txt sitemap seed per site of the
/// request seeds, every entry given the Label `page`; the other modes
/// add none.
///
/// Covers: story:6-304
#[test]
fn follow_sitemap_seeds_each_seed_sites_robots_txt_once() {
    let seeds: Vec<Seed> = serde_json::from_value(json!([
        { "url": "http://site.test:8080/a?x=1#f", "label": "page" },
        { "url": "http://site.test:8080/b", "label": "page" },
        { "url": "https://other.test/", "label": "page" },
    ]))
    .unwrap();
    let added = |follow: &str| {
        serde_json::to_value(sitemap_seeds(
            &extraction(json!({ "formats": { "markdown": true }, "follow": follow })),
            &seeds,
        ))
        .unwrap()
    };
    assert_eq!(
        (added("sitemap"), added("scope"), added("none")),
        (
            json!([
                { "sitemap": "http://site.test:8080/robots.txt", "rules": [{ "pattern": "**", "label": "page" }] },
                { "sitemap": "https://other.test/robots.txt", "rules": [{ "pattern": "**", "label": "page" }] },
            ]),
            json!([]),
            json!([]),
        )
    );
}

/// A field takes its transform's type, or null, in the schema of `pages`,
/// which requires it: a Record holding each field's value, or null, holds
/// to it, while one missing a field, or holding a text where a number is
/// typed, is refused.
///
/// Covers: story:6-304
#[test]
fn the_pages_schema_holds_each_fields_type() {
    let extraction = extraction(json!({
        "formats": { "markdown": true },
        "fields": {
            "name": { "locator": { "kind": "css", "selector": "h1" } },
            "price": {
                "locator": { "kind": "css", "selector": ".price" },
                "transform": { "kind": "number", "locale": "de-DE" },
            },
        },
    }));
    let declared = dataset(&extraction);
    let datasets = Datasets::declare(std::slice::from_ref(&declared)).unwrap();
    let page = formats(json!({
        "url": "http://site.test/a",
        "base_url": "http://site.test/a",
        "main_content": "found",
        "markdown": "# A",
    }));
    let at = PointInTime::from_rfc3339("2026-10-26T10:00:00Z".to_owned());
    let with = |name: Value, price: Value| {
        let mut fields = Map::new();
        fields.insert("name".to_owned(), name);
        fields.insert("price".to_owned(), price);
        record_of(&page, Some(200), &at, fields)
    };
    let mut without_price = with(json!("Delta kite"), json!(1099.98));
    without_price.as_object_mut().unwrap().remove("price");

    assert_eq!(
        (
            declared.schema.as_ref().unwrap()["properties"]["price"].clone(),
            datasets
                .check(pushed(with(json!("Delta kite"), json!(1099.98))))
                .is_ok(),
            datasets
                .check(pushed(with(Value::Null, Value::Null)))
                .is_ok(),
            datasets.check(pushed(without_price)).is_ok(),
            datasets
                .check(pushed(with(json!("Delta kite"), json!("1099.98"))))
                .is_ok(),
        ),
        (
            json!({ "type": ["number", "null"] }),
            true,
            true,
            false,
            false
        )
    );
}

/// The start refuses with `invalid_params` a field whose name a format's
/// property, `status` or `fetchedAt` takes, and a field the engine could
/// never read.
///
/// Covers: story:6-304
#[rstest::rstest]
#[case::a_formats_property(json!({ "url": { "locator": { "kind": "css", "selector": "a" } } }))]
#[case::status(json!({ "status": { "locator": { "kind": "css", "selector": "p" } } }))]
#[case::fetched_at(json!({ "fetchedAt": { "locator": { "kind": "css", "selector": "p" } } }))]
#[case::an_unreadable_field(json!({ "name": { "locator": { "kind": "css", "selector": "h1[" } } }))]
fn the_start_refuses_a_field_it_cannot_hold_or_read(#[case] fields: Value) {
    let extraction = extraction(json!({ "formats": { "markdown": true }, "fields": fields }));

    let refused = declare_into(&extraction, &mut BTreeMap::new(), &mut Vec::new());

    assert_eq!(refused.map_err(|error| error.code()), Err("invalid_params"));
}
