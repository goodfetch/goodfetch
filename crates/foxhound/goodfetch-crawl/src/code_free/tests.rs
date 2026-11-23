//! The code-free crawl's declarations as functions of its extraction: the
//! Dataset `pages` declared from the formats request, a Record of a page
//! checked against it, and the sitemap seeds `follow: sitemap` adds.

use goodfetch_protocol::point_in_time;
use std::collections::BTreeMap;
use std::sync::Arc;

use goodfetch_dataset::Datasets;
use goodfetch_protocol::foxhound::crawl::{CrawlExtraction, PushedRecord, Seed};
use goodfetch_protocol::foxhound::dataset::RecordData;
use goodfetch_protocol::page::PageFormats;
use serde_json::{Map, Value, json};

use super::{Pages, dataset, declare_into, record_of, sitemap_seeds};

fn extraction(value: Value) -> CrawlExtraction {
    serde_json::from_value(value).unwrap()
}

fn formats(value: Value) -> PageFormats {
    serde_json::from_value(value).unwrap()
}

fn pushed(data: &Value) -> Vec<PushedRecord> {
    vec![PushedRecord {
        dataset: "pages".to_owned(),
        data: RecordData::of(data).unwrap(),
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
    let declared = dataset(&extraction, &goodfetch_counted_work::CountedWork::default()).unwrap();
    let datasets = Datasets::declare(std::slice::from_ref(&declared)).unwrap();
    let page = formats(json!({
        "url": "http://site.test/a",
        "base_url": "http://site.test/a",
        "main_content": "found",
        "markdown": "# A",
        "links": [{ "url": "http://site.test/b", "href": "/b", "text": "B", "rel": [] }],
    }));
    let at = point_in_time::from_rfc3339("2026-10-26T10:00:00Z").unwrap();
    let record = record_of(page, Some(200), at, Map::new());
    let mut without_links = record.clone();
    without_links.as_object_mut().unwrap().remove("links");
    let mut with_html = record.clone();
    with_html["html"] = json!("<p>A</p>");

    assert_eq!(
        (
            declared.name.as_str(),
            declared.key.clone(),
            record["status"].clone(),
            record["fetched_at"].clone(),
            datasets
                .check(
                    pushed(&record),
                    &goodfetch_counted_work::CountedWork::default()
                )
                .is_ok(),
            datasets
                .check(
                    pushed(&without_links),
                    &goodfetch_counted_work::CountedWork::default()
                )
                .is_ok(),
            datasets
                .check(
                    pushed(&with_html),
                    &goodfetch_counted_work::CountedWork::default()
                )
                .is_ok(),
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
    let (pages, work) = (
        Pages::default(),
        goodfetch_counted_work::CountedWork::default(),
    );
    let refused = goodfetch_test_support::runtime().block_on(async {
        declare_into(&extraction, &mut labels, &mut datasets, &pages, &work)
            .await
            .unwrap();
        declare_into(
            &extraction,
            &mut BTreeMap::new(),
            &mut datasets.clone(),
            &pages,
            &work,
        )
        .await
        .map(drop)
    });
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
    let declared = dataset(&extraction, &goodfetch_counted_work::CountedWork::default()).unwrap();
    let datasets = Datasets::declare(std::slice::from_ref(&declared)).unwrap();
    let page = formats(json!({
        "url": "http://site.test/a",
        "base_url": "http://site.test/a",
        "main_content": "found",
        "markdown": "# A",
    }));
    let at = point_in_time::from_rfc3339("2026-10-26T10:00:00Z").unwrap();
    let with = |name: Value, price: Value| {
        let mut fields = Map::new();
        fields.insert("name".to_owned(), name);
        fields.insert("price".to_owned(), price);
        record_of(page.clone(), Some(200), at, fields)
    };
    let mut without_price = with(json!("Delta kite"), json!(1099.98));
    without_price.as_object_mut().unwrap().remove("price");

    assert_eq!(
        (
            declared.schema.as_ref().unwrap()["properties"]["price"].clone(),
            datasets
                .check(
                    pushed(&with(json!("Delta kite"), json!(1099.98))),
                    &goodfetch_counted_work::CountedWork::default()
                )
                .is_ok(),
            datasets
                .check(
                    pushed(&with(Value::Null, Value::Null)),
                    &goodfetch_counted_work::CountedWork::default()
                )
                .is_ok(),
            datasets
                .check(
                    pushed(&without_price),
                    &goodfetch_counted_work::CountedWork::default()
                )
                .is_ok(),
            datasets
                .check(
                    pushed(&with(json!("Delta kite"), json!("1099.98"))),
                    &goodfetch_counted_work::CountedWork::default()
                )
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
/// property, `status` or `fetched_at` takes, and a field the engine could
/// never read.
///
/// Covers: story:6-304
#[rstest::rstest]
#[case::a_formats_property(json!({ "url": { "locator": { "kind": "css", "selector": "a" } } }))]
#[case::status(json!({ "status": { "locator": { "kind": "css", "selector": "p" } } }))]
#[case::fetched_at(json!({ "fetched_at": { "locator": { "kind": "css", "selector": "p" } } }))]
#[case::an_unreadable_field(json!({ "name": { "locator": { "kind": "css", "selector": "h1[" } } }))]
#[case::an_xpath_the_engine_refuses(json!({ "name": { "locator": { "kind": "xpath", "selector": "//h1[" } } }))]
fn the_start_refuses_a_field_it_cannot_hold_or_read(#[case] fields: Value) {
    let extraction = extraction(json!({ "formats": { "markdown": true }, "fields": fields }));

    let refused = goodfetch_test_support::runtime().block_on(declare_into(
        &extraction,
        &mut BTreeMap::new(),
        &mut Vec::new(),
        &Pages::default(),
        &goodfetch_counted_work::CountedWork::default(),
    ));

    assert_eq!(
        refused.map(drop).map_err(|error| error.code()),
        Err("invalid_params")
    );
}

/// Two starts in one engine reading the same extraction share the Dataset
/// `pages` built for the first, its schema generated and its validator
/// compiled once; a start reading another extraction builds its own
/// (issue 2204).
#[test]
fn two_starts_of_one_extraction_build_the_dataset_pages_once() {
    let (pages, work) = (
        Pages::default(),
        goodfetch_counted_work::CountedWork::default(),
    );
    let asked = extraction(json!({ "formats": { "markdown": true } }));
    let other = extraction(json!({ "formats": { "html": true } }));
    let (first, second, third) = goodfetch_test_support::runtime().block_on(async {
        (
            pages.of(&asked, &work).await.unwrap(),
            pages.of(&asked, &work).await.unwrap(),
            pages.of(&other, &work).await.unwrap(),
        )
    });
    assert_eq!(
        (Arc::ptr_eq(&first, &second), Arc::ptr_eq(&first, &third)),
        (true, false)
    );
}

/// A page's Record holds the page's Markdown, HTML and text where the page
/// read them, moved into it, never copied (ADR 0140's F4: a Record's data
/// is copied nowhere), beside its other formats, its status, when it was
/// read and its fields' values.
///
/// Covers: story:6-304
#[test]
fn a_pages_record_holds_the_pages_text_where_the_page_read_it() {
    let page = formats(json!({
        "url": "http://site.test/a",
        "base_url": "http://site.test/",
        "main_content": "found",
        "markdown": "# A",
        "html": "<h1>A</h1>",
        "text": "A",
        "links": [{ "url": "http://site.test/b", "href": "/b", "text": "B", "rel": [] }],
    }));
    let read_at =
        [&page.markdown, &page.html, &page.text].map(|text| text.as_ref().unwrap().as_ptr());
    let at = point_in_time::from_rfc3339("2026-10-26T10:00:00Z").unwrap();
    let mut fields = Map::new();
    fields.insert("name".to_owned(), json!("Delta kite"));

    let record = record_of(page, None, at, fields);

    assert_eq!(
        record,
        json!({
            "url": "http://site.test/a",
            "base_url": "http://site.test/",
            "main_content": "found",
            "markdown": "# A",
            "html": "<h1>A</h1>",
            "text": "A",
            "links": [{ "url": "http://site.test/b", "href": "/b", "text": "B", "rel": [] }],
            "status": null,
            "fetched_at": "2026-10-26T10:00:00Z",
            "name": "Delta kite",
        })
    );
    assert_eq!(
        ["markdown", "html", "text"].map(|format| record[format].as_str().unwrap().as_ptr()),
        read_at
    );
}
