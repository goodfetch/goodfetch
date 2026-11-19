//! Code-free crawl cases (spec #582; ADR 0103), on Foxhound's executable
//! over stdio against the Proving ground: a `crawl.start` with no handler
//! takes `extraction`, and the engine handles every request itself,
//! committing one Record per page into the Dataset `pages`, keyed by
//! `url`, whose schema it generates from the formats request; the program
//! is never sent a `crawl_request.handle`. `follow` chooses the pages
//! fetched beyond the seeds: every in-scope link (`scope`), the entries of
//! the sitemaps each seed's site names (`sitemap`) or none. Named `fields`
//! are read by locators and given by Value transforms (ADR 0123), each
//! Record's Provenance naming each transform, and each field the page did
//! not give is null with its reason.

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use goodfetch_protocol::home::RUN_TRACE_PROVENANCE;
use goodfetch_test_client::Attached;
use goodfetch_test_client::HomeLayout;
use goodfetch_test_support::ToolStatus;
use goodfetch_test_support::{
    BROWSERS_PATH_VARIABLE, ProvingGround, ReceivedRequests, TestEnv, browser_cache,
};
use rstest::rstest;
use rusqlite::Connection;
use serde_json::{Value, json};

use crate::attached;
use crate::attached_with;
use crate::fetch_tiers::{settings, tiers};
use crate::frontier::seed_labels;

/// The Proving ground's seed.
const SEED: u64 = 994;

/// The Markdown task, whose index links every Markdown page and whose
/// oracle judges each page's Markdown.
const MARKDOWN_TASK: &str = "extraction/7";

/// How many Markdown pages the task's index links.
const MARKDOWN_PAGES: usize = 31;

/// The site whose robots.txt names a sitemap on its partner host.
const SITEMAP_SITE: &str = "sitemap-seed.test";

/// The fields every Record of a Markdown crawl holds.
const FIELDS: [&str; 6] = [
    "base_url",
    "fetched_at",
    "main_content",
    "markdown",
    "status",
    "url",
];

/// The path of the URL `url`.
fn path_of(url: &str) -> String {
    let host = url.find("://").map_or(0, |at| at + "://".len());
    url[host..]
        .find('/')
        .map_or_else(String::new, |at| url[host + at..].to_owned())
}

/// Starts the code-free crawl `run_id` of `seeds` reading `extraction`
/// under `more` settings, waits for its Run to end, and answers the
/// start's reply and every Record of `pages`; the engine must never send
/// the program a handler request.
fn crawl(
    engine: &mut Attached,
    proving_ground: &ProvingGround,
    run: (&str, &[Value]),
    extraction: &Value,
    more: &Value,
) -> (Value, Vec<Value>) {
    let (started, read) = crawl_read(engine, proving_ground, run, extraction, more);
    let values = read.iter().map(|record| record["value"].clone()).collect();
    (started, values)
}

/// As [`crawl`], answering each Record of `pages` as `run.records` reads
/// it: its value, its Provenance and the rest.
fn crawl_read(
    engine: &mut Attached,
    proving_ground: &ProvingGround,
    (run_id, seeds): (&str, &[Value]),
    extraction: &Value,
    more: &Value,
) -> (Value, Vec<Value>) {
    let mut more = more.clone();
    more["politeness"]["domain"]["delay"] = json!(0);
    let started = engine.call(
        "start",
        "crawl.start",
        &json!({
            "run_id": run_id,
            "seeds": seeds,
            "labels": seed_labels(seeds),
            "datasets": [],
            "extraction": extraction,
            "settings": settings(proving_ground, &more),
        }),
        &mut |handle| panic!("a code-free crawl sent its program {handle}"),
    );
    if started.get("error").is_some() {
        return (started, Vec::new());
    }
    engine.send(&json!({
        "jsonrpc": "2.0", "id": "records", "method": "run.records",
        "params": { "run_id": run_id, "dataset": "pages" },
    }));
    engine.send(&json!({
        "jsonrpc": "2.0", "method": "$/credit", "params": { "id": "records", "credits": 1000 },
    }));
    let mut records = Vec::new();
    loop {
        let message = engine
            .next()
            .expect("the engine closed before the reader answered");
        assert_ne!(
            message["method"], "crawl_request.handle",
            "a code-free crawl sent its program {message}"
        );
        if message["method"] == "$/record" {
            records.push(message["params"]["record"].clone());
        } else if message["id"] == "records" {
            assert!(message.get("result").is_some(), "{message}");
            return (started, records);
        }
    }
}

/// The paths of `records`' URLs, sorted.
fn paths(records: &[Value]) -> Vec<String> {
    let mut paths: Vec<String> = records
        .iter()
        .map(|record| path_of(record["url"].as_str().unwrap()))
        .collect();
    paths.sort();
    paths
}

/// A crawl from the Markdown task's index with `formats: { markdown }` and
/// the built-in `follow: scope`, one link deep, fetches the index and
/// every in-scope page it links, with no code: `pages` holds one Record
/// per page, keyed by its URL, each
/// with its status, when it was fetched and its Markdown, and nothing
/// else; every Record validated against the schema the engine generated
/// from the request as it committed, and the oracle verifies each page's
/// Markdown.
///
/// Covers: story:6-304, operation:crawl.start, operation:run.records
#[test]
fn a_crawl_with_no_code_fills_pages_keyed_by_url_with_each_pages_markdown() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let mut engine = crate::site::engine_on(&env, &proving_ground);
    let index = proving_ground.start_url(MARKDOWN_TASK).unwrap();

    let (started, records) = crawl(
        &mut engine,
        &proving_ground,
        ("markdown", &[json!({ "url": index, "label": "page" })]),
        &json!({ "formats": { "markdown": true } }),
        &json!({ "fetch": { "tier": "http" }, "crawl": { "max_depth": 1 } }),
    );
    assert!(engine.finished());

    let index_path = path_of(&index);
    let judged: Vec<Value> = records
        .iter()
        .filter(|record| path_of(record["url"].as_str().unwrap()) != index_path)
        .map(|record| {
            json!({ "id": path_of(record["url"].as_str().unwrap()), "markdown": record["markdown"] })
        })
        .collect();
    let verdict = proving_ground
        .judge_records(MARKDOWN_TASK, ToolStatus::Success, &judged)
        .unwrap();
    let urls: BTreeSet<&str> = records
        .iter()
        .map(|record| record["url"].as_str().unwrap())
        .collect();
    let shapes: BTreeSet<(Vec<String>, Option<u64>, bool)> = records
        .iter()
        .map(|record| {
            (
                record.as_object().unwrap().keys().cloned().collect(),
                record["status"].as_u64(),
                record["fetched_at"]
                    .as_str()
                    .is_some_and(|at| at.ends_with('Z') && at.contains('T')),
            )
        })
        .collect();
    assert_eq!(
        (
            started["result"]["run_id"].clone(),
            records.len(),
            urls.len(),
            urls.contains(index.as_str()),
            shapes,
            &verdict["outcome"],
        ),
        (
            json!("markdown"),
            MARKDOWN_PAGES + 1,
            MARKDOWN_PAGES + 1,
            true,
            BTreeSet::from([(
                FIELDS.iter().map(|field| (*field).to_owned()).collect(),
                Some(200),
                true
            )]),
            &json!("verified"),
        ),
        "{verdict}"
    );
}

/// `follow: none` fetches only the seeds: the index's links are never
/// requested, and `pages` holds the seeds' Records alone.
///
/// Covers: story:6-304, operation:crawl.start
#[test]
fn follow_none_fetches_only_the_seeds() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let mut engine = crate::site::engine_on(&env, &proving_ground);
    let index = proving_ground.start_url(MARKDOWN_TASK).unwrap();
    let page = format!("{index}/headings/levels");

    let (_, records) = crawl(
        &mut engine,
        &proving_ground,
        (
            "seeds-only",
            &[
                json!({ "url": index, "label": "page" }),
                json!({ "url": page, "label": "page" }),
            ],
        ),
        &json!({ "formats": { "markdown": true }, "follow": "none" }),
        &json!({ "fetch": { "tier": "http" } }),
    );
    assert!(engine.finished());

    let observations = proving_ground.observations().unwrap();
    let mut received: Vec<String> = ReceivedRequests::of(&observations)
        .every()
        .filter(|request| request.target != "/robots.txt")
        .map(|request| request.target.as_str().to_owned())
        .collect();
    received.sort();
    let mut seeds = vec![path_of(&index), path_of(&page)];
    seeds.sort();
    assert_eq!((paths(&records), received), (seeds.clone(), seeds));
}

/// `follow: sitemap` reads the sitemaps each seed's site names in its
/// robots.txt, here one its partner host holds: the seed and the site's
/// own entry are fetched, each a Record, and the partner's entry, outside
/// the scope, is not.
///
/// Covers: story:6-304, operation:crawl.start
#[test]
fn follow_sitemap_reads_the_sitemap_the_sites_robots_txt_names() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let mut engine = crate::site::engine_on(&env, &proving_ground);
    let about = proving_ground.url(SITEMAP_SITE, "/about");

    let (_, records) = crawl(
        &mut engine,
        &proving_ground,
        ("sitemap", &[json!({ "url": about, "label": "page" })]),
        &json!({ "formats": { "markdown": true }, "follow": "sitemap" }),
        &json!({ "fetch": { "tier": "http" } }),
    );
    assert!(engine.finished());

    let hosts: BTreeSet<String> = records
        .iter()
        .map(|record| {
            let url = record["url"].as_str().unwrap();
            url.split("://")
                .nth(1)
                .unwrap()
                .split(':')
                .next()
                .unwrap()
                .to_owned()
        })
        .collect();
    assert_eq!(
        (paths(&records), hosts),
        (
            vec!["/about".to_owned(), "/books/4".to_owned()],
            BTreeSet::from([SITEMAP_SITE.to_owned()])
        )
    );
}

/// A code-free crawl declares its own Dataset: a start that also declares
/// one is refused with `invalid_params` before its Run begins.
///
/// Covers: story:6-304, operation:crawl.start
#[test]
fn a_code_free_start_that_declares_a_dataset_is_refused() {
    let env = TestEnv::new();
    let mut engine = attached(&env);
    let refused = engine.call(
        "start",
        "crawl.start",
        &json!({
            "run_id": "declared",
            "seeds": [{ "url": "http://example.test/", "label": "page" }],
            "datasets": [{ "name": "pages" }],
            "extraction": { "formats": { "markdown": true } },
        }),
        &mut |handle| panic!("a refused crawl sent its program {handle}"),
    );
    assert!(engine.finished());
    assert_eq!(
        refused["error"]["data"]["code"],
        json!("invalid_params"),
        "{refused}"
    );
}

/// The fields of the value page, in German: a name, a SKU picked from its
/// sentence by a `match`, a price, a release date and two date-times, one
/// stating no offset and one stating its own, a relative link, and a
/// discount the page does not show.
fn value_fields() -> Value {
    let css = |selector: &str| json!({ "kind": "css", "selector": selector });
    json!({
        "name": { "locator": css("h1") },
        "sku": { "locator": css(".sku"), "match": { "pattern": "Artikelnummer: (\\S+)" } },
        "price": { "locator": css(".price"), "transform": { "kind": "number", "locale": "de-DE" } },
        "released": { "locator": css("time.released"), "transform": { "kind": "date", "locale": "de-DE" } },
        "available": {
            "locator": css("time.available"),
            "transform": { "kind": "date_time", "locale": "de-DE", "time_zone": "Europe/Berlin" },
        },
        "updated": {
            "locator": css("time.updated"),
            "transform": { "kind": "date_time", "locale": "de-DE", "time_zone": "Europe/Berlin" },
        },
        "manual": { "locator": css("a.manual"), "attribute": "href" },
        "discount": { "locator": css(".discount"), "transform": { "kind": "number", "locale": "de-DE" } },
    })
}

/// What the value page's Record holds of its fields, and what its
/// Provenance names: each transform with the text it read, its locale and
/// its zone, and the discount null for want of an element.
fn value_record(origin: &str) -> (Value, Value, Value) {
    (
        json!({
            "name": "Delta-Drachen",
            "sku": "DK-0042",
            "price": 1099.98,
            "released": "2026-02-01",
            "available": "2026-02-01T10:00:00+01:00",
            "updated": "2026-02-01T10:00:00+05:30",
            "manual": format!("{origin}/manuals/delta.pdf"),
            "discount": null,
        }),
        json!([
            { "field": "/available", "text": "2026-02-01T10:00", "transform": "date_time", "locale": "de-DE", "time_zone": "Europe/Berlin" },
            { "field": "/manual", "text": "manuals/delta.pdf", "transform": "text" },
            { "field": "/name", "text": "\n  Delta-Drachen\n", "transform": "text" },
            { "field": "/price", "text": "1099.98", "transform": "number", "locale": "de-DE" },
            { "field": "/released", "text": "2026-02-01", "transform": "date", "locale": "de-DE" },
            { "field": "/sku", "text": "Artikelnummer: DK-0042 (neu)", "transform": "match" },
            { "field": "/sku", "text": "DK-0042", "transform": "text" },
            { "field": "/updated", "text": "2026-02-01T10:00:00+05:30", "transform": "date_time", "locale": "de-DE" },
        ]),
        json!([{ "field": "/discount", "reason": "no_element" }]),
    )
}

/// The value page's one Record: its fields' values, its Provenance's
/// Value transforms and its unread fields.
fn fields_of(records: &[Value]) -> (Value, Value, Value) {
    let [record] = records else {
        panic!("one Record of the value page, not {records:?}");
    };
    let names = value_fields();
    let values: serde_json::Map<String, Value> = names
        .as_object()
        .unwrap()
        .keys()
        .map(|name| (name.clone(), record["value"][name].clone()))
        .collect();
    (
        Value::Object(values),
        record["provenance"]["value_transforms"].clone(),
        record["provenance"]["unread_fields"].clone(),
    )
}

/// A code-free crawl with `fields` fills each on the `http` tier, with no
/// code: a text and a link read as they are, a SKU picked by a `match`, a
/// price, a date and two date-times given by their Value transforms from
/// the page's machine-readable values, typed as the Dataset's generated
/// schema holds them (the Record validated against it as it committed);
/// the Record's Provenance names each transform with the text it read, its
/// locale and its zone; and a locator that finds nothing leaves its field
/// null with its reason, not an error.
///
/// Covers: story:6-304, operation:crawl.start, operation:run.records
#[test]
fn a_crawl_with_fields_fills_each_by_its_value_transform_and_provenance_names_it() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let mut engine = crate::site::engine_on(&env, &proving_ground);
    let page = proving_ground.url("extraction.test", "/values");
    let origin = proving_ground.url("extraction.test", "");

    let (started, records) = crawl_read(
        &mut engine,
        &proving_ground,
        ("fields", &[json!({ "url": page, "label": "page" })]),
        &json!({ "formats": { "markdown": true }, "follow": "none", "fields": value_fields() }),
        &json!({ "fetch": { "tier": "http" } }),
    );
    assert!(engine.finished());

    assert!(started.get("result").is_some(), "{started}");
    assert_eq!(fields_of(&records), value_record(&origin));
    assert_eq!(records[0]["provenance"]["tier"]["tier"], json!("http"));
}

/// An `XPath` field reads alike on every tier: the `http` tier reads it
/// from the served document; under `auto` the page's sample reads it from
/// the served document with no escalation, then on the browser, whose run
/// commits; the browser tier's page reads it itself. Each Record holds the
/// same values and Provenance as the CSS fields' crawl, and names no call
/// that escalated it.
///
/// Covers: story:6-304, operation:crawl.start, operation:run.records
#[rstest]
#[case::http("http", json!("http"))]
#[case::auto_sampled_with_no_escalation("auto", json!("browser"))]
#[case::browser("browser", json!("browser"))]
fn an_xpath_field_reads_alike_on_every_tier(#[case] tier: &str, #[case] read_on: Value) {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let cache = browser_cache(&env).expect("Playwright's Chrome for Testing is installed");
    let mut engine = attached_with(&env, Some(Duration::from_secs(60)), |command| {
        command.env(BROWSERS_PATH_VARIABLE, &cache);
    });
    let page = proving_ground.url("extraction.test", "/values");
    let origin = proving_ground.url("extraction.test", "");
    let mut fields = value_fields();
    fields["name"]["locator"] = json!({ "kind": "xpath", "selector": "//article/h1" });
    fields["manual"]["locator"] =
        json!({ "kind": "xpath", "selector": "//a[contains(@class, 'manual')]" });

    let (started, records) = crawl_read(
        &mut engine,
        &proving_ground,
        ("xpath", &[json!({ "url": page, "label": "page" })]),
        &json!({ "formats": { "markdown": true }, "follow": "none", "fields": fields }),
        &json!({ "fetch": { "tier": tier } }),
    );
    assert!(engine.finished());

    assert!(started.get("result").is_some(), "{started}");
    assert_eq!(
        (
            fields_of(&records),
            records[0]["provenance"]["tier"]["tier"].clone(),
            records[0]["provenance"]["tier"]["call"].clone(),
        ),
        (value_record(&origin), read_on, Value::Null)
    );
}

/// A code-free crawl's `fields` take a list and rows as `page.read` does
/// (ADR 0147), alike on the `http` tier and in the browser: the catalog
/// listing's product names as a list, and its products as rows, each row
/// keeping its name and price beside the blurb it lacks, which the Record's
/// Provenance names unread.
///
/// Covers: story:6-304, operation:crawl.start, operation:run.records
#[rstest]
#[case::http("http")]
#[case::browser("browser")]
fn a_crawls_fields_take_a_list_and_rows_alike_on_every_tier(#[case] tier: &str) {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let cache = browser_cache(&env).expect("Playwright's Chrome for Testing is installed");
    let mut engine = attached_with(&env, Some(Duration::from_secs(60)), |command| {
        command.env(BROWSERS_PATH_VARIABLE, &cache);
    });
    let listing = proving_ground.start_url("catalog/1").unwrap();
    let css = |selector: &str| json!({ "locator": { "kind": "css", "selector": selector } });

    let (started, records) = crawl_read(
        &mut engine,
        &proving_ground,
        (
            "lists-and-rows",
            &[json!({ "url": listing, "label": "page" })],
        ),
        &json!({
            "formats": {},
            "follow": "none",
            "fields": {
                "names": { "list": css("li.product a") },
                "products": {
                    "rows": { "kind": "css", "selector": "li.product" },
                    "fields": {
                        "name": css("a"),
                        "price": { "locator": { "kind": "xpath", "selector": "./span[@class='price']" } },
                        "blurb": css(".blurb"),
                    },
                },
            },
        }),
        &json!({ "fetch": { "tier": tier } }),
    );
    assert!(engine.finished());

    assert!(started.get("result").is_some(), "{started}");
    let [record] = records.as_slice() else {
        panic!("one Record of the listing page, not {records:?}");
    };
    let rows = record["value"]["products"].as_array().unwrap();
    assert!(
        !rows.is_empty()
            && rows.iter().all(|row| {
                row["name"].is_string() && row["price"].is_string() && row["blurb"].is_null()
            }),
        "{record}"
    );
    let names: Vec<Value> = rows.iter().map(|row| row["name"].clone()).collect();
    let unread: Vec<Value> = (0..rows.len())
        .map(|row| json!({ "field": format!("/products/{row}/blurb"), "reason": "no_element" }))
        .collect();
    assert_eq!(
        (
            &record["value"]["names"],
            &record["provenance"]["unread_fields"]
        ),
        (&Value::Array(names), &Value::Array(unread))
    );
}

/// A field's locator ladder reads the first rung that finds an element
/// (issue 1972), alike on the `http` tier and in the browser: the value
/// page's name and manual, each declared with a `locator` and an earlier
/// `fallbacks` rung that find nothing before the rung that finds it, read
/// the values, Provenance and nulls of the crawl whose fields have one
/// locator each.
///
/// Covers: story:6-304, operation:crawl.start, operation:run.records
#[rstest]
#[case::http("http")]
#[case::browser("browser")]
fn a_fields_later_rung_supplies_its_value_when_the_earlier_ones_find_nothing(#[case] tier: &str) {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let cache = browser_cache(&env).expect("Playwright's Chrome for Testing is installed");
    let mut engine = attached_with(&env, Some(Duration::from_secs(60)), |command| {
        command.env(BROWSERS_PATH_VARIABLE, &cache);
    });
    let page = proving_ground.url("extraction.test", "/values");
    let origin = proving_ground.url("extraction.test", "");
    let mut fields = value_fields();
    fields["name"]["locator"] = json!({ "kind": "css", "selector": ".product-title" });
    fields["name"]["fallbacks"] = json!([
        { "kind": "xpath", "selector": "//header/h2" },
        { "kind": "css", "selector": "h1" },
    ]);
    fields["manual"]["locator"] = json!({ "kind": "xpath", "selector": "//a[@rel='manual']" });
    fields["manual"]["fallbacks"] = json!([{ "kind": "css", "selector": "a.manual" }]);

    let (started, records) = crawl_read(
        &mut engine,
        &proving_ground,
        ("ladder", &[json!({ "url": page, "label": "page" })]),
        &json!({ "formats": {}, "follow": "none", "fields": fields }),
        &json!({ "fetch": { "tier": tier } }),
    );
    assert!(engine.finished());

    assert!(started.get("result").is_some(), "{started}");
    assert_eq!(
        (
            fields_of(&records),
            records[0]["provenance"]["tier"]["tier"].clone()
        ),
        (value_record(&origin), json!(tier))
    );
}

/// A list's and a row field's locator ladders read their first rung that
/// finds an element (issue 1972), alike on the `http` tier and in the
/// browser: the catalog listing's names, and each row's price, read past
/// a first rung that finds nothing, and a row field none of whose rungs
/// finds an element is null with the reason it has today.
///
/// Covers: story:6-304, operation:crawl.start, operation:run.records
#[rstest]
#[case::http("http")]
#[case::browser("browser")]
fn a_lists_and_a_row_fields_later_rung_supplies_their_values(#[case] tier: &str) {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let cache = browser_cache(&env).expect("Playwright's Chrome for Testing is installed");
    let mut engine = attached_with(&env, Some(Duration::from_secs(60)), |command| {
        command.env(BROWSERS_PATH_VARIABLE, &cache);
    });
    let listing = proving_ground.start_url("catalog/1").unwrap();
    let css = |selector: &str| json!({ "kind": "css", "selector": selector });

    let (started, records) = crawl_read(
        &mut engine,
        &proving_ground,
        ("ladders", &[json!({ "url": listing, "label": "page" })]),
        &json!({
            "formats": {},
            "follow": "none",
            "fields": {
                "names": { "list": { "locator": css("li.product h3"), "fallbacks": [css("li.product a")] } },
                "products": {
                    "rows": css("li.product"),
                    "fields": {
                        "name": { "locator": css("a") },
                        "price": {
                            "locator": css(".cost"),
                            "fallbacks": [{ "kind": "xpath", "selector": "./span[@class='price']" }],
                        },
                        "blurb": { "locator": css(".blurb"), "fallbacks": [css(".summary")] },
                    },
                },
            },
        }),
        &json!({ "fetch": { "tier": tier } }),
    );
    assert!(engine.finished());

    assert!(started.get("result").is_some(), "{started}");
    let [record] = records.as_slice() else {
        panic!("one Record of the listing page, not {records:?}");
    };
    let rows = record["value"]["products"].as_array().unwrap();
    assert!(
        !rows.is_empty()
            && rows.iter().all(|row| {
                row["name"].is_string() && row["price"].is_string() && row["blurb"].is_null()
            }),
        "{record}"
    );
    let names: Vec<Value> = rows.iter().map(|row| row["name"].clone()).collect();
    let unread: Vec<Value> = (0..rows.len())
        .map(|row| json!({ "field": format!("/products/{row}/blurb"), "reason": "no_element" }))
        .collect();
    assert_eq!(
        (
            &record["value"]["names"],
            &record["provenance"]["unread_fields"]
        ),
        (&Value::Array(names), &Value::Array(unread))
    );
}

/// Provenance names the tier that sent a document, whoever chose it
/// (issue 2409): under `auto`, in an engine with browser pages, a Label's
/// samples send their cheaper run from inside the Session's browser, and
/// so does each request once the Label is learned onto the document tier;
/// every such run is recorded on `browser_fetch`, none on `http`, and how
/// it was chosen stays `sample` or `compared`.
///
/// Covers: story:5-116, story:6-304, operation:crawl.start
#[test]
fn a_document_sent_from_the_sessions_browser_under_auto_is_recorded_on_browser_fetch() {
    const HOST: &str = "tiers.test";
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let cache = browser_cache(&env).expect("Playwright's Chrome for Testing is installed");
    let mut engine = attached_with(&env, Some(Duration::from_secs(60)), |command| {
        command.env(BROWSERS_PATH_VARIABLE, &cache);
    });
    let seeds: Vec<Value> = (1..=5)
        .map(|page| {
            let url = proving_ground.url(HOST, &format!("/products/{page}"));
            json!({ "url": url, "label": "page" })
        })
        .collect();

    let (started, records) = crawl(
        &mut engine,
        &proving_ground,
        ("sent-from-the-browser", &seeds),
        &json!({
            "formats": {},
            "follow": "none",
            "fields": { "title": { "locator": { "kind": "css", "selector": "h1" } } },
        }),
        &json!({ "fetch": { "tier": "auto" }, "politeness": { "domain": { "concurrency": 1 } } }),
    );
    assert!(engine.finished());

    assert!(started.get("result").is_some(), "{started}");
    let sample = [
        json!(["page", "browser_fetch", "sample", null, null]),
        json!(["page", "browser", "sample", null, null]),
    ];
    let learned = json!(["page", "browser_fetch", "compared", null, null]);
    assert_eq!(
        (records.len(), tiers(&env, "sent-from-the-browser")),
        (
            5,
            [
                &sample[..],
                &sample[..],
                &sample[..],
                &[learned.clone(), learned]
            ]
            .concat()
        )
    );
}

/// The host of the pages a Label's samples fetch from each client: its
/// storage page answers a `mark` cookie, its quiet page none.
const MARKS_HOST: &str = "scope.test";

/// The user configuration that trusts the Proving ground's CA for
/// [`MARKS_HOST`].
fn trusting_marks_host(proving_ground: &ProvingGround) -> String {
    format!(
        "tls.trusted_cas.\"{MARKS_HOST}\" = ['{}']\n",
        proving_ground.description().ca.display()
    )
}

/// How a client shows itself: its TLS `ClientHello`'s JA4, without the
/// extensions a resumed handshake adds, and the headers it sends but
/// `cookie`, in order, with their values.
type Client = (Option<String>, Vec<(String, String)>);

/// A request [`MARKS_HOST`] received for a page `marked`: its mark, the
/// client it came from and whether it carried its own mark's cookie.
type Marked = (String, Client, bool);

/// Fetches, under `fetch`, one seed of label `page` per mark
/// `{prefix}-1` to `{prefix}-5` at `{origin}{page}?mark=…`, one request
/// at a time, and answers each request [`MARKS_HOST`] received for one of
/// them, in order. Frontier learning is off: it would skip the marks
/// after the first few, whose pages read alike.
fn marked_run(
    (engine, proving_ground): (&mut Attached, &ProvingGround),
    (run_id, origin, page, prefix): (&str, &str, &str, &str),
    fetch: &Value,
) -> Vec<Marked> {
    let seeds: Vec<Value> = (1..=5)
        .map(|mark| json!({ "url": format!("{origin}{page}?mark={prefix}-{mark}"), "label": "page" }))
        .collect();
    let (started, records) = crawl(
        engine,
        proving_ground,
        (run_id, &seeds),
        &json!({
            "formats": {},
            "follow": "none",
            "fields": { "title": { "locator": { "kind": "css", "selector": "h1" } } },
        }),
        &json!({
            "fetch": fetch,
            "politeness": { "domain": { "concurrency": 1 } },
            "frontier": { "learning": false },
        }),
    );
    assert!(started.get("result").is_some(), "{started}");
    assert_eq!(records.len(), 5, "{run_id}");

    let observations = proving_ground.observations().unwrap();
    let query = format!("{page}?mark={prefix}-");
    ReceivedRequests::of(&observations)
        .every()
        .filter(|request| request.host == MARKS_HOST && request.target.starts_with(&query))
        .map(|request| {
            let mark = request.target[page.len() + "?mark=".len()..].to_owned();
            let ja4 = request
                .tls
                .as_ref()
                .and_then(|hello| hello["ja4_base"].as_str().map(str::to_owned));
            let headers = request
                .headers
                .iter()
                .filter(|(name, _)| !name.eq_ignore_ascii_case("cookie"))
                .cloned()
                .collect();
            let own_cookie = request
                .cookies
                .iter()
                .any(|(name, value)| name == "mark" && *value == mark);
            (mark, (ja4, headers), own_cookie)
        })
        .collect()
}

/// The marks among `requests` sent with their own mark's cookie.
fn with_their_own_cookie(requests: &[Marked]) -> Vec<String> {
    requests
        .iter()
        .filter(|(_, _, own)| *own)
        .map(|(mark, _, _)| mark.clone())
        .collect()
}

/// The keys of `run_id`'s `weakened` list, as `run.finish` reports it.
fn weakened(engine: &mut Attached, run_id: &str) -> BTreeSet<String> {
    let report = engine.call(
        "finish",
        "run.finish",
        &json!({ "run_id": run_id }),
        &mut |_| None,
    );
    strings(&report["result"]["weakened"], "key")
}

/// The provenance of a Label's three samples, their cheaper run on
/// `cheaper`, then of its two requests learned onto `cheaper`.
fn sampled_then_learned(cheaper: &str) -> Vec<Value> {
    let sample = [
        json!(["page", cheaper, "sample", null, null]),
        json!(["page", "browser", "sample", null, null]),
    ];
    let learned = json!(["page", cheaper, "compared", null, null]);
    [
        &sample[..],
        &sample[..],
        &sample[..],
        &[learned.clone(), learned],
    ]
    .concat()
}

/// Under `auto`, `fetch.http_from: transport` sends each sample's `http`
/// run from a transport Session and its browser run from another Session,
/// nothing carried between them: no browser run carries the cookie its
/// sample's `http` run was answered, and every run of the cheaper tier is
/// recorded on `http`. The Run lists the setting in `weakened` and the
/// Audit log records it. Under the default, `browser`, each sample's
/// cheaper run is sent from inside the Session's browser as before, so the
/// browser run carries the cookie it set, every cheaper run is recorded on
/// `browser_fetch`, and nothing is weakened.
///
/// Covers: setting:fetch.http_from, story:5-116, operation:crawl.start
#[test]
#[expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]
fn under_transport_a_samples_browser_run_carries_nothing_of_its_http_run() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let cache = browser_cache(&env).expect("Playwright's Chrome for Testing is installed");
    let mut engine = attached_with(&env, Some(Duration::from_secs(60)), |command| {
        command.env(BROWSERS_PATH_VARIABLE, &cache);
    });
    let origin = proving_ground.url(MARKS_HOST, "");

    let from_the_transport = marked_run(
        (&mut engine, &proving_ground),
        ("from-the-transport", &origin, "/storage", "t"),
        &json!({ "tier": "auto", "http_from": "transport" }),
    );
    let from_the_browser = marked_run(
        (&mut engine, &proving_ground),
        ("from-the-browser", &origin, "/storage", "b"),
        &json!({ "tier": "auto" }),
    );
    let weakened = ["from-the-transport", "from-the-browser"]
        .map(|run_id| weakened(&mut engine, run_id).contains("fetch.http_from"));
    assert!(engine.finished());

    assert_eq!(
        (
            tiers(&env, "from-the-transport"),
            with_their_own_cookie(&from_the_transport),
            tiers(&env, "from-the-browser"),
            with_their_own_cookie(&from_the_browser),
            weakened,
            switched_off_in_audit_log(&env).contains("fetch.http_from"),
        ),
        (
            sampled_then_learned("http"),
            Vec::<String>::new(),
            sampled_then_learned("browser_fetch"),
            ["b-1", "b-2", "b-3"].map(str::to_owned).to_vec(),
            [true, false],
            true,
        ),
        "{from_the_transport:?}\n{from_the_browser:?}"
    );
}

/// Under `auto`, `fetch.http_from: transport` sends each request of a
/// Label learned onto `http` from the transport, and Provenance records it
/// on `http`: its TLS `ClientHello` and its headers are those of a crawl
/// forced onto `http`. The transport passes for the engine's Chrome on the
/// wire, so only the cookies a Session keeps tell the two clients apart,
/// which the case above reads.
///
/// Covers: setting:fetch.http_from, setting:fetch.tier
#[test]
#[expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]
fn under_transport_a_label_learned_onto_http_sends_with_the_transports_tls_and_headers() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    env.user_configuration(&trusting_marks_host(&proving_ground));
    let cache = browser_cache(&env).expect("Playwright's Chrome for Testing is installed");
    let mut engine = attached_with(&env, Some(Duration::from_secs(60)), |command| {
        command.env(BROWSERS_PATH_VARIABLE, &cache);
    });
    let origin = format!("https://{MARKS_HOST}:{}", proving_ground.address().port());

    let forced = marked_run(
        (&mut engine, &proving_ground),
        ("forced-http", &origin, "/quiet", "r"),
        &json!({ "tier": "http" }),
    );
    let from_the_transport = marked_run(
        (&mut engine, &proving_ground),
        ("from-the-transport", &origin, "/quiet", "t"),
        &json!({ "tier": "auto", "http_from": "transport" }),
    );
    assert!(engine.finished());

    let clients = |requests: &[Marked], marks: &[&str]| -> BTreeSet<Client> {
        requests
            .iter()
            .filter(|(mark, _, _)| marks.contains(&mark.as_str()))
            .map(|(_, client, _)| client.clone())
            .collect()
    };
    let transport = clients(&forced, &["r-1", "r-2", "r-3", "r-4", "r-5"]);
    assert_eq!(
        (
            transport.len(),
            tiers(&env, "from-the-transport"),
            clients(&from_the_transport, &["t-4", "t-5"]),
        ),
        (1, sampled_then_learned("http"), transport.clone()),
        "{forced:?}\n{from_the_transport:?}"
    );
}

/// The Proving ground's rung site: a static page, a page that answers
/// only the pinned browser's TLS fingerprint, and a script-rendered page,
/// each kind apart by its paths' first segment.
const RUNGS_HOST: &str = "rungs.localhost";

/// The rung site's kinds, each crawled as a Label of its own name.
const RUNGS: [&str; 3] = ["identity", "static", "scripted"];

/// The pages of each kind of the rung site.
const RUNG_PAGES: u32 = 6;

/// The samples each Label of the rung site takes, `fetch.sample.count`'s
/// default: its first pages.
const RUNG_SAMPLES: u32 = 3;

/// What a code-free Run of the rung site gave.
struct RungRun {
    /// Each Record's path, with its `id` and `name`, sorted by path.
    read: Vec<(String, Value)>,
    /// The tier each Record's Provenance names, by its path.
    served_on: BTreeMap<String, String>,
    /// What `run.finish` reported.
    report: Value,
}

/// Crawls, code-free under `auto` with `http_from`, every page of the rung
/// site, one request at a time, each Label its kind.
fn rungs_run(
    (engine, proving_ground): (&mut Attached, &ProvingGround),
    run_id: &str,
    http_from: &str,
) -> RungRun {
    let kinds = RUNGS.map(|kind| (kind, RUNG_PAGES));
    rung_pages_run((engine, proving_ground), run_id, http_from, &kinds)
}

/// As [`rungs_run`], crawling pages 1 to `pages` of each of `kinds`.
fn rung_pages_run(
    (engine, proving_ground): (&mut Attached, &ProvingGround),
    run_id: &str,
    http_from: &str,
    kinds: &[(&str, u32)],
) -> RungRun {
    let origin = format!("https://{RUNGS_HOST}:{}", proving_ground.address().port());
    let seeds: Vec<Value> = kinds
        .iter()
        .flat_map(|&(kind, pages)| {
            let origin = &origin;
            (1..=pages)
                .map(move |page| json!({ "url": format!("{origin}/{kind}/{page}"), "label": kind }))
        })
        .collect();
    let (started, records) = crawl_read(
        engine,
        proving_ground,
        (run_id, &seeds),
        &json!({
            "formats": {},
            "follow": "none",
            "fields": {
                "id": { "locator": { "kind": "css", "selector": "p.id" } },
                "name": { "locator": { "kind": "css", "selector": "h1.name" } },
            },
        }),
        &json!({
            "fetch": { "tier": "auto", "http_from": http_from },
            "politeness": { "domain": { "concurrency": 1 } },
            "frontier": { "learning": false },
        }),
    );
    assert!(started.get("result").is_some(), "{started}");
    let report = engine.call(
        "finish",
        "run.finish",
        &json!({ "run_id": run_id }),
        &mut |_| None,
    );
    let path = |record: &Value| path_of(record["value"]["url"].as_str().unwrap_or_default());
    let mut read: Vec<(String, Value)> = records
        .iter()
        .map(|record| {
            let value = &record["value"];
            (
                path(record),
                json!({ "id": value["id"], "name": value["name"] }),
            )
        })
        .collect();
    read.sort_by(|left, right| left.0.cmp(&right.0));
    let served_on = records
        .iter()
        .map(|record| {
            let tier = &record["provenance"]["tier"]["tier"];
            (path(record), tier.as_str().unwrap_or_default().to_owned())
        })
        .collect();
    RungRun {
        read,
        served_on,
        report: report["result"].clone(),
    }
}

/// The tiers `run_id`'s requests past their Label's samples were served
/// on, by Label.
fn learned_onto(env: &TestEnv, run_id: &str) -> BTreeMap<String, BTreeSet<String>> {
    let mut learned: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for entry in tiers(env, run_id) {
        if entry[2] != "sample" {
            learned
                .entry(entry[0].as_str().unwrap_or_default().to_owned())
                .or_default()
                .insert(entry[1].as_str().unwrap_or_default().to_owned());
        }
    }
    learned
}

/// Each kind of the rung site and the tier its Label ends on.
fn rungs_learned(identity: &str) -> BTreeMap<String, BTreeSet<String>> {
    [
        ("identity", identity),
        ("static", "http"),
        ("scripted", "browser"),
    ]
    .into_iter()
    .map(|(label, tier)| (label.to_owned(), BTreeSet::from([tier.to_owned()])))
    .collect()
}

/// An engine that trusts the Proving ground's authority for the rung site
/// and starts the pinned browser from `env`'s Browser cache.
fn rungs_engine(env: &TestEnv, proving_ground: &ProvingGround) -> Attached {
    env.user_configuration(&format!(
        "tls.trusted_cas.\"{RUNGS_HOST}\" = ['{}']\n",
        proving_ground.description().ca.display()
    ));
    let cache = browser_cache(env).expect("Playwright's Chrome for Testing is installed");
    attached_with(env, Some(Duration::from_secs(60)), |command| {
        command.env(BROWSERS_PATH_VARIABLE, &cache);
    })
}

/// The rung site's manifest: each page of `kinds` by path, with the
/// Record a right reading of it gives, null where it gives none.
fn rung_records(proving_ground: &ProvingGround, kinds: &[&str]) -> Vec<(String, Value)> {
    let manifest: Value = serde_json::from_str(
        &crate::frontier::fetch(
            proving_ground,
            &format!("http://{RUNGS_HOST}/manifest.json"),
        )
        .body,
    )
    .unwrap();
    let mut expected: Vec<(String, Value)> = manifest["pages"]
        .as_array()
        .unwrap_or_else(|| panic!("{manifest}"))
        .iter()
        .map(|page| {
            (
                page["path"].as_str().unwrap_or_default().to_owned(),
                page["records"][0].clone(),
            )
        })
        .filter(|(path, _)| {
            kinds
                .iter()
                .any(|kind| path.starts_with(&format!("/{kind}/")))
        })
        .collect();
    expected.sort_by(|left, right| left.0.cmp(&right.0));
    expected
}

/// The kind of each page of the rung site's manifest, by its path.
fn rung_kinds(proving_ground: &ProvingGround) -> BTreeMap<String, String> {
    let manifest: Value = serde_json::from_str(
        &crate::frontier::fetch(
            proving_ground,
            &format!("http://{RUNGS_HOST}/manifest.json"),
        )
        .body,
    )
    .unwrap();
    manifest["pages"]
        .as_array()
        .unwrap_or_else(|| panic!("{manifest}"))
        .iter()
        .map(|page| {
            (
                page["path"].as_str().unwrap_or_default().to_owned(),
                page["kind"].as_str().unwrap_or_default().to_owned(),
            )
        })
        .filter(|(path, _)| {
            RUNGS
                .iter()
                .any(|kind| path.starts_with(&format!("/{kind}/")))
        })
        .collect()
}

/// The proxy tier each domain's pool of `run_id` stands at, as its run
/// file keeps it.
fn proxy_tiers(env: &TestEnv, run_id: &str) -> BTreeMap<String, String> {
    let connection = Connection::open(HomeLayout::of(env.home()).run_file(run_id)).unwrap();
    let mut statement = connection
        .prepare("SELECT domain_key, proxy_tier FROM pool_tiers")
        .unwrap();
    statement
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

/// Under `auto` and `fetch.http_from: learned` (#2419), a refusal of a
/// Label's transport answers on the fetch tier first: the rung site's
/// page that refuses the transport from page 4, saying its address is
/// blocked, after 3 pages the transport read alike, retires the Session
/// whose transport it refused and drops the Label's choice, so the next
/// requests are samples, whose comparison moves the Label to
/// `browser_fetch`; every page commits its Record, the refused one once
/// retried, and the proxy tier stays where it was. Where the browser's
/// run of such a sample is refused too, the proxy tier moves up and the
/// request ends `blocked`.
///
/// Covers: setting:fetch.http_from, story:6-251
#[test]
#[expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]
fn under_learned_a_refusal_of_the_transport_resamples_its_label_before_moving_the_proxy_tier() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let mut engine = rungs_engine(&env, &proving_ground);

    let refused = rung_pages_run(
        (&mut engine, &proving_ground),
        "transport-refused",
        "learned",
        &[("refused", 9)],
    );
    let blocked = rung_pages_run(
        (&mut engine, &proving_ground),
        "every-client-refused",
        "learned",
        &[("blocked", 4)],
    );
    let outcomes = crate::fetch_tiers::outcomes(&mut engine, "every-client-refused");
    assert!(engine.finished());

    let chosen = |run_id: &str| -> Vec<String> {
        tiers(&env, run_id)
            .iter()
            .map(|entry| {
                format!(
                    "{}:{}",
                    entry[1].as_str().unwrap_or_default(),
                    entry[2].as_str().unwrap_or_default()
                )
            })
            .collect()
    };
    let raised = |run_id: &str| -> BTreeSet<String> {
        proxy_tiers(&env, run_id)
            .into_values()
            .filter(|tier| tier != "none")
            .collect()
    };
    assert_eq!(
        (
            &refused.read,
            learned_onto(&env, "transport-refused"),
            raised("transport-refused"),
            raised("every-client-refused"),
            blocked.read.len(),
            outcomes
                .iter()
                .map(|(_, kind, _)| kind.as_str().unwrap_or_default())
                .collect::<Vec<_>>(),
        ),
        (
            &rung_records(&proving_ground, &["refused"]),
            BTreeMap::from([(
                "refused".to_owned(),
                BTreeSet::from(["browser_fetch".to_owned(), "http".to_owned()])
            )]),
            BTreeSet::new(),
            BTreeSet::from(["datacenter".to_owned()]),
            3,
            vec!["succeeded", "succeeded", "succeeded", "blocked"],
        ),
        "{:?}\n{:?}\n{:?}\n{outcomes:?}",
        chosen("transport-refused"),
        chosen("every-client-refused"),
        blocked.read,
    );
}

/// A request the rung site received: its path, its connection's TLS
/// fingerprint (its JA4 without what a resumed handshake adds) and the
/// `visitor` cookie it sent, which the site gives each new cookie jar.
type RungRequest = (String, Option<String>, Option<String>);

/// Every request the rung site received, in order.
fn rung_requests(proving_ground: &ProvingGround) -> Vec<RungRequest> {
    let observations = proving_ground.observations().unwrap();
    ReceivedRequests::of(&observations)
        .every()
        .filter(|request| request.host.split(':').next() == Some(RUNGS_HOST))
        .map(|request| {
            let ja4 = request
                .tls
                .as_ref()
                .and_then(|hello| hello["ja4_base"].as_str().map(str::to_owned));
            let visitor = request
                .cookies
                .iter()
                .find(|(name, _)| name == "visitor")
                .map(|(_, value)| value.clone());
            (request.target.clone(), ja4, visitor)
        })
        .collect()
}

/// The pool Sessions `run_id`'s run file holds, by where each sends from.
fn sessions_by_client(env: &TestEnv, run_id: &str) -> BTreeMap<String, i64> {
    let connection = Connection::open(HomeLayout::of(env.home()).run_file(run_id)).unwrap();
    let mut statement = connection
        .prepare("SELECT sends_from, COUNT(*) FROM pool_sessions GROUP BY sends_from")
        .unwrap();
    statement
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

/// The tier the manifest's kind of page `kind` needs: the cheapest that
/// reads its Record.
fn tier_of_kind(kind: &str) -> &'static str {
    match kind {
        "static" => "http",
        "browser_identity" => "browser_fetch",
        "script_rendered" => "browser",
        other => panic!("the rung site has no ladder kind {other}"),
    }
}

/// The fingerprints the rung site saw: the transport's, which every
/// robots.txt request carries (#2422), and the browser's, every other one.
struct Fingerprints {
    transport: BTreeSet<Option<String>>,
    browser: BTreeSet<Option<String>>,
}

impl Fingerprints {
    fn of(every: &[RungRequest]) -> Self {
        let transport: BTreeSet<_> = every
            .iter()
            .filter(|(path, _, _)| path == "/robots.txt")
            .map(|(_, ja4, _)| ja4.clone())
            .collect();
        let browser = every
            .iter()
            .map(|(_, ja4, _)| ja4.clone())
            .filter(|ja4| !transport.contains(ja4))
            .collect();
        Self { transport, browser }
    }

    /// The fingerprints of the client a page served on `tier` is sent by.
    fn of_tier(&self, tier: &str) -> BTreeSet<Option<String>> {
        if tier == "http" {
            self.transport.clone()
        } else {
            self.browser.clone()
        }
    }
}

/// The entries of `by_path` whose page is past its Label's samples.
fn past_samples<T: Clone>(by_path: &BTreeMap<String, T>) -> BTreeMap<String, T> {
    by_path
        .iter()
        .filter(|(path, _)| {
            path.rsplit('/')
                .next()
                .and_then(|page| page.parse::<u32>().ok())
                .is_some_and(|page| page > RUNG_SAMPLES)
        })
        .map(|(path, value)| (path.clone(), value.clone()))
        .collect()
}

/// The fingerprint of each request of `seen` for `path`.
fn seen_with(seen: &[RungRequest], path: &str) -> Vec<Option<String>> {
    seen.iter()
        .filter(|(target, _, _)| target == path)
        .map(|(_, ja4, _)| ja4.clone())
        .collect()
}

/// The fingerprints each `visitor` cookie came back with, by its value.
fn jars(every: &[RungRequest]) -> BTreeMap<String, BTreeSet<Option<String>>> {
    let mut jars: BTreeMap<String, BTreeSet<Option<String>>> = BTreeMap::new();
    for (_, ja4, visitor) in every {
        if let Some(visitor) = visitor {
            jars.entry(visitor.clone()).or_default().insert(ja4.clone());
        }
    }
    jars
}

/// The rung site's code-free Runs under `fetch.http_from` `learned`,
/// `transport` and `browser`, in that order, each with the requests the
/// site received during it.
fn rung_runs_by_http_from(
    (engine, proving_ground): (&mut Attached, &ProvingGround),
) -> Vec<(RungRun, Vec<RungRequest>)> {
    let mut runs = Vec::new();
    let mut received = 0;
    for (run_id, http_from) in [
        ("rungs-learned", "learned"),
        ("rungs-transport", "transport"),
        ("rungs-browser", "browser"),
    ] {
        let run = rungs_run((&mut *engine, proving_ground), run_id, http_from);
        let requests = rung_requests(proving_ground);
        runs.push((run, requests[received..].to_vec()));
        received = requests.len();
    }
    runs
}

/// The ladder's acceptance (#2423), read from outside on the rung site.
/// Under `auto` and `fetch.http_from: learned`, each page past its Label's
/// samples is served on the tier its kind in the site's manifest needs:
/// a static page on `http`, a page answered only to the pinned browser's
/// fingerprint on `browser_fetch` (#2417), a script-rendered page on
/// `browser`, and every Record equals the manifest's. The site saw each of
/// those pages sent by the client its tier implies, the transport's
/// fingerprint for `http` and the browser's otherwise, so a static page
/// past its samples sends no browser-fetch hop (`C7.hop_*` until #2294's
/// counters run), and each later identity page arrives once. Under
/// `transport` the identity pages end on `browser` and nothing runs on
/// `browser_fetch`; under `browser` no page arrives with the transport's
/// fingerprint. Across the three Runs each cookie the site gave a
/// transport's jar, one Session's, came back with that fingerprint only
/// and never from a browser, and the Run file holds Sessions of both
/// clients under `learned`. The Run report lists
/// `fetch.http_from` in `weakened` under `learned` and `transport` only,
/// and the further runs of a sample add nothing to Usage.
///
/// Covers: setting:fetch.http_from, setting:fetch.tier, story:5-116
#[test]
#[expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]
fn under_auto_each_kind_of_rung_page_is_served_on_the_cheapest_tier_that_reads_it() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let mut engine = rungs_engine(&env, &proving_ground);

    let runs = rung_runs_by_http_from((&mut engine, &proving_ground));
    assert!(engine.finished());
    let [
        (learned, learned_seen),
        (transported, _),
        (browser, browser_seen),
    ] = &runs[..]
    else {
        unreachable!("three Runs");
    };

    let every = rung_requests(&proving_ground);
    let fingerprints = Fingerprints::of(&every);
    let manifest = rung_records(&proving_ground, &RUNGS);
    let kinds = rung_kinds(&proving_ground);
    // Each page past its samples: the tier its kind needs, and the
    // fingerprints the site saw its requests with.
    let needed: BTreeMap<String, String> = past_samples(&kinds)
        .into_iter()
        .map(|(path, kind)| (path, tier_of_kind(&kind).to_owned()))
        .collect();
    let clients_seen: BTreeMap<String, BTreeSet<Option<String>>> = needed
        .keys()
        .map(|path| {
            (
                path.clone(),
                seen_with(learned_seen, path).into_iter().collect(),
            )
        })
        .collect();
    let clients_implied: BTreeMap<String, BTreeSet<Option<String>>> = needed
        .iter()
        .map(|(path, tier)| (path.clone(), fingerprints.of_tier(tier)))
        .collect();
    let identity_requests: Vec<usize> = needed
        .keys()
        .filter(|path| path.starts_with("/identity/"))
        .map(|path| seen_with(learned_seen, path).len())
        .collect();
    let jars = jars(&every);
    let cookies_in_a_browser = every
        .iter()
        .filter(|(_, ja4, visitor)| visitor.is_some() && !fingerprints.transport.contains(ja4))
        .count();
    let documents_from_the_transport = browser_seen
        .iter()
        .filter(|(path, ja4, _)| kinds.contains_key(path) && fingerprints.transport.contains(ja4))
        .count();
    let weakened: Vec<bool> = runs
        .iter()
        .map(|(run, _)| strings(&run.report["weakened"], "key").contains("fetch.http_from"))
        .collect();
    assert_eq!(
        (
            (fingerprints.transport.len(), fingerprints.browser.len()),
            (
                past_samples(&learned.served_on),
                &learned.read,
                &clients_seen,
                identity_requests,
            ),
            (
                jars.values().all(|ja4s| ja4s.len() == 1),
                cookies_in_a_browser,
                sessions_by_client(&env, "rungs-learned").len(),
            ),
            learned.report["usage"]["requests"] == transported.report["usage"]["requests"],
            (
                learned_onto(&env, "rungs-transport"),
                tiers(&env, "rungs-transport")
                    .iter()
                    .any(|entry| entry[1] == "browser_fetch"),
                &transported.read,
            ),
            (documents_from_the_transport, &browser.read),
            weakened,
        ),
        (
            (1, 1),
            (needed, &manifest, &clients_implied, vec![1; 3]),
            (true, 0, 2),
            true,
            (rungs_learned("browser"), false, &manifest),
            (0, &manifest),
            vec![true, true, false],
        ),
        "{jars:?}\n{:?}\n{:?}\n{:?}\n{:?}\n{:?}\n{:?}",
        learned.served_on,
        browser.served_on,
        sessions_by_client(&env, "rungs-learned"),
        sessions_by_client(&env, "rungs-transport"),
        sessions_by_client(&env, "rungs-browser"),
        tiers(&env, "rungs-learned"),
    );
}

/// `config explain` lists `fetch.http_from`, `browser` by default, which
/// weakens nothing; a Run that sends from the transport, `learned` or
/// `transport`, weakens it.
///
/// Covers: setting:fetch.http_from, command:config.explain
#[expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]
#[rstest]
#[case::by_default(&[], json!(["browser", "default", false]))]
#[case::learned(&["--set", "fetch.http_from=learned"], json!(["learned", "run", true]))]
#[case::transport(&["--set", "fetch.http_from=transport"], json!(["transport", "run", true]))]
fn config_explain_lists_where_a_labels_http_leaves_from(
    #[case] set: &[&str],
    #[case] explained: Value,
) {
    let env = TestEnv::new();
    let output = goodfetch_test_support::cli(&env, env!("CARGO_BIN_EXE_goodfetch-foxhound"))
        .args(["config", "explain", "fetch.http_from", "--json"])
        .args(set.iter().copied())
        .run()
        .unwrap();

    assert_eq!(output.exit, Some(0), "{}", output.stderr);
    let document: Value = serde_json::from_str(&output.stdout).unwrap();
    let listed: Vec<Value> = document["settings"]
        .as_array()
        .unwrap_or_else(|| panic!("{document}"))
        .iter()
        .filter(|setting| setting["key"] == "fetch.http_from")
        .map(|setting| json!([setting["value"], setting["layer"], setting["weakens"]]))
        .collect();
    assert_eq!(listed, vec![explained], "{document}");
}

/// `config explain` says why a Run whose Sessions hold a login sends its
/// HTTP from the browser: its `fetch.http_from` away from `browser` yields
/// to the Run's `session.source`, named with its value, its layer and its
/// run-layer origin, here the CLI's flag (ADR 0083); a Run with no
/// `session.source`, or that sends from the browser, yields to nothing
/// (#2418).
///
/// Covers: setting:fetch.http_from, setting:session.source, command:config.explain
#[expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]
#[rstest]
#[case::learned_with_a_login(
    &["fetch.http_from=learned", SAVED_IDENTITY],
    json!({"key": "session.source", "value": {"kind": "saved_identity", "name": "shop"}, "layer": "run", "origin": "flag"}),
)]
#[case::browser_with_a_login(&["fetch.http_from=browser", SAVED_IDENTITY], Value::Null)]
#[case::learned_without_a_login(&["fetch.http_from=learned"], Value::Null)]
fn config_explain_says_a_login_sends_from_the_browser(
    #[case] set: &[&str],
    #[case] yields_to: Value,
) {
    let env = TestEnv::new();
    let output = goodfetch_test_support::cli(&env, env!("CARGO_BIN_EXE_goodfetch-foxhound"))
        .args(["config", "explain", "fetch.http_from", "--json"])
        .args(set.iter().flat_map(|value| ["--set", value]))
        .run()
        .unwrap();

    assert_eq!(output.exit, Some(0), "{}", output.stderr);
    let document: Value = serde_json::from_str(&output.stdout).unwrap();
    let explained: Vec<&Value> = document["settings"]
        .as_array()
        .unwrap_or_else(|| panic!("{document}"))
        .iter()
        .filter(|setting| setting["key"] == "fetch.http_from")
        .map(|setting| &setting["yields_to"])
        .collect();
    assert_eq!(explained, vec![&yields_to], "{document}");
}

/// A `--set` of the Run's `session.source` to a Saved identity.
const SAVED_IDENTITY: &str = r#"session.source={"kind":"saved_identity","name":"shop"}"#;

/// The headers of each request `host` received for `target`, their names
/// in lowercase, in order, without `host`, which each client places
/// itself.
fn received(
    proving_ground: &ProvingGround,
    host: &str,
    target: &str,
) -> Vec<Vec<(String, String)>> {
    proving_ground
        .received_requests()
        .unwrap()
        .every()
        .filter(|request| request.host.split(':').next() == Some(host) && request.target == target)
        .map(|request| {
            request
                .headers
                .iter()
                .map(|(name, value)| (name.to_ascii_lowercase(), value.clone()))
                .filter(|(name, _)| name != "host")
                .collect()
        })
        .collect()
}

/// A seed of `fetch-edge.test`'s page, which declares no AI-use signal.
fn edge_page(proving_ground: &ProvingGround) -> Value {
    json!({ "url": proving_ground.url("fetch-edge.test", "/target"), "label": "page" })
}

/// A sitemap seed of `sitemap-seed.test`'s index.
fn sitemap_index(proving_ground: &ProvingGround) -> Value {
    json!({
        "sitemap": proving_ground.url(SITEMAP_SITE, "/sitemap-index.xml"),
        "rules": [{ "pattern": "**", "label": "page" }],
    })
}

/// Every site file a code-free crawl reads, a robots.txt, an AI-use
/// signal file or a sitemap seed's sitemap, leaves from a Session's own
/// HTTP transport, at the default, `auto` sending its HTTP from the
/// browser, and on a forced `browser` alike: the site receives it once,
/// with the header set of a document a crawl forced onto `http` fetches,
/// never with a browser page's.
///
/// Covers: setting:fetch.http_from, setting:fetch.tier, setting:site_preferences.respect_robots_txt, setting:site_preferences.respect_use_signals
#[expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]
#[rstest]
#[case::robots_txt_at_the_default(json!({}), edge_page, ("fetch-edge.test", "/robots.txt"))]
#[case::robots_txt_on_a_forced_browser(
    json!({ "fetch": { "tier": "browser" } }),
    edge_page,
    ("fetch-edge.test", "/robots.txt")
)]
#[case::an_ai_use_signal_file_at_the_default(
    json!({ "site_preferences": { "respect_use_signals": true, "purpose": "ai-train" } }),
    edge_page,
    ("fetch-edge.test", "/.well-known/tdmrep.json")
)]
#[case::a_sitemap_seeds_sitemap_at_the_default(
    json!({}),
    sitemap_index,
    (SITEMAP_SITE, "/sitemap-index.xml")
)]
fn every_site_file_leaves_from_a_sessions_own_transport(
    #[case] more: Value,
    #[case] seed: fn(&ProvingGround) -> Value,
    #[case] (host, file): (&str, &str),
) {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let cache = browser_cache(&env).expect("Playwright's Chrome for Testing is installed");
    let mut engine = attached_with(&env, Some(Duration::from_secs(60)), |command| {
        command.env(BROWSERS_PATH_VARIABLE, &cache);
    });
    let extraction = json!({ "formats": {}, "follow": "none" });

    let (started, _) = crawl(
        &mut engine,
        &proving_ground,
        ("site-files", &[seed(&proving_ground)]),
        &extraction,
        &more,
    );
    assert!(started.get("result").is_some(), "{started}");
    let document = proving_ground.url("access.test", "/identity?tier=http");
    let (started, _) = crawl(
        &mut engine,
        &proving_ground,
        (
            "forced-http",
            &[json!({ "url": document, "label": "page" })],
        ),
        &extraction,
        &json!({ "fetch": { "tier": "http" } }),
    );
    assert!(started.get("result").is_some(), "{started}");
    assert!(engine.finished());

    let transport = received(&proving_ground, "access.test", "/identity?tier=http");
    assert_eq!(transport.len(), 1, "{transport:?}");
    assert_eq!(received(&proving_ground, host, file), transport);
}

/// A site file a code-free crawl reads after a document, the RSL license
/// the document's `Link` header names, leaves from a Session of its own,
/// at the default as on a forced `browser`: the site receives it once,
/// without the cookie the document set in the browser's Session.
///
/// Covers: setting:fetch.http_from, setting:fetch.tier, setting:site_preferences.respect_use_signals
#[expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]
#[rstest]
#[case::at_the_default(json!({}))]
#[case::on_a_forced_browser(json!({ "tier": "browser" }))]
fn a_site_file_read_after_a_document_carries_no_cookie_of_the_browsers(#[case] fetch: Value) {
    const MARKUP_HOST: &str = "markup.access-policy.test";
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let cache = browser_cache(&env).expect("Playwright's Chrome for Testing is installed");
    let mut engine = attached_with(&env, Some(Duration::from_secs(60)), |command| {
        command.env(BROWSERS_PATH_VARIABLE, &cache);
    });
    let document = proving_ground.url(MARKUP_HOST, "/catalog/5");

    let (started, _) = crawl(
        &mut engine,
        &proving_ground,
        ("license", &[json!({ "url": document, "label": "page" })]),
        &json!({ "formats": {}, "follow": "none" }),
        &json!({
            "fetch": fetch,
            "site_preferences": { "respect_use_signals": true, "purpose": "ai-train" },
        }),
    );
    assert!(started.get("result").is_some(), "{started}");
    assert!(engine.finished());

    let cookies: Vec<Vec<(String, String)>> = proving_ground
        .received_requests()
        .unwrap()
        .every()
        .filter(|request| {
            request.host.split(':').next() == Some(MARKUP_HOST) && request.target == "/license.xml"
        })
        .map(|request| request.cookies.clone())
        .collect();
    assert_eq!(cookies, vec![Vec::new()]);
}

/// Each attempt's browser page carries that attempt's own lease, however
/// many attempts run at once: of 16 seeds on the `browser` tier at a
/// concurrency of 16, one page answers 3 s late and 15 at once, and each
/// attempt keeps its page's commands going after the others ended and
/// gave their leases back, so every page commits a Record from its first
/// and only request (#2083: each page took the lease of the attempt that
/// handed its Run's pages over last, and its commands were withheld once
/// that attempt ended).
///
/// Covers: story:6-304, operation:crawl.start, operation:run.records
#[test]
fn concurrent_browser_attempts_each_keep_their_own_lease() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let cache = browser_cache(&env).expect("Playwright's Chrome for Testing is installed");
    let mut engine = attached_with(&env, Some(Duration::from_secs(60)), |command| {
        command.env(BROWSERS_PATH_VARIABLE, &cache);
    });
    let mut pages = vec!["/products/1?hold=3000".to_owned()];
    pages.extend((2..=16).map(|number| format!("/products/{number}")));
    let seeds: Vec<Value> = pages
        .iter()
        .map(|path| json!({ "url": proving_ground.url("tiers.test", path), "label": "page" }))
        .collect();

    let (started, records) = crawl(
        &mut engine,
        &proving_ground,
        ("concurrent", &seeds),
        &json!({ "formats": { "markdown": true }, "follow": "none" }),
        &json!({
            "fetch": { "tier": "browser" },
            "politeness": { "sharing": "run", "domain": { "concurrency": 16 } },
            "crawl": { "handler": { "concurrency": 16 } },
        }),
    );
    assert!(engine.finished());

    let observations = proving_ground.observations().unwrap();
    let mut received: Vec<String> = ReceivedRequests::of(&observations)
        .every()
        .map(|request| request.target.as_str())
        .filter(|target| target.starts_with("/products/"))
        .map(str::to_owned)
        .collect();
    received.sort();
    pages.sort();
    assert!(started.get("result").is_some(), "{started}");
    assert_eq!((paths(&records), received), (pages.clone(), pages));
}

/// An `XPath` field the engine does not read to nodes fails the start
/// with `invalid_params` before its Run begins.
///
/// Covers: story:6-304, operation:crawl.start
#[test]
fn a_code_free_start_with_an_xpath_the_engine_refuses_is_refused() {
    let env = TestEnv::new();
    let mut engine = attached(&env);
    let refused = engine.call(
        "start",
        "crawl.start",
        &json!({
            "run_id": "refused-xpath",
            "seeds": [{ "url": "http://example.test/", "label": "page" }],
            "datasets": [],
            "extraction": {
                "formats": { "markdown": true },
                "fields": { "count": { "locator": { "kind": "xpath", "selector": "count(//h1)" } } },
            },
        }),
        &mut |handle| panic!("a refused crawl sent its program {handle}"),
    );
    assert!(engine.finished());
    assert_eq!(
        refused["error"]["data"]["code"],
        json!("invalid_params"),
        "{refused}"
    );
}

/// The maximum-throughput combination (ADR 0138, `docs/throughput.md`):
/// a code-free crawl that asks for no format, with learning, the
/// Provenance log, the Records and the Block classification switched off,
/// still fetches the index and every page it links, once each, but keeps
/// no Record, each outcome saying why, and writes no Provenance log; its
/// report names each switch in its warnings and `weakened`, beside the
/// Proving ground's private ranges and no delay, and the Audit log records
/// each new switch once.
///
/// Covers: story:6-304, operation:crawl.start, setting:provenance.log, setting:dataset.records, setting:fetch.block_classification, setting:frontier.learning
#[test]
#[expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]
fn with_every_per_page_feature_off_the_crawl_fetches_and_follows_and_keeps_nothing_else() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let mut engine = crate::site::engine_on(&env, &proving_ground);
    let index = proving_ground.start_url(MARKDOWN_TASK).unwrap();
    let switched_off = json!({
        "fetch": { "tier": "http", "block_classification": false },
        "crawl": { "max_depth": 1 },
        "frontier": { "learning": false },
        "provenance": { "log": false },
        "dataset": { "records": false },
    });

    let (started, records) = crawl(
        &mut engine,
        &proving_ground,
        ("throughput", &[json!({ "url": index, "label": "page" })]),
        &json!({ "formats": {} }),
        &switched_off,
    );
    let report = engine.call(
        "finish",
        "run.finish",
        &json!({ "run_id": "throughput" }),
        &mut |_| None,
    );
    let outcomes = engine.call(
        "outcomes",
        "run.outcomes",
        &json!({ "run_id": "throughput" }),
        &mut |_| None,
    );
    assert!(engine.finished());

    let observations = proving_ground.observations().unwrap();
    let received: BTreeSet<String> = ReceivedRequests::of(&observations)
        .every()
        .filter(|request| request.target != "/robots.txt")
        .map(|request| request.target.as_str().to_owned())
        .collect();
    let marks: BTreeSet<String> = outcomes["result"]["outcomes"]
        .as_array()
        .unwrap_or_else(|| panic!("no outcomes: {outcomes}"))
        .iter()
        .map(|outcome| outcome["nothing_committed"].to_string())
        .collect();
    let codes = |field: &str, key: &str| strings(&report["result"][field], key);
    let log = HomeLayout::of(env.home()).run_trace("throughput", RUN_TRACE_PROVENANCE);
    assert_eq!(
        (
            started["result"]["run_id"].clone(),
            records.len(),
            received.len(),
            marks,
            codes("warnings", "code")
                .into_iter()
                .filter(|code| !crate::on_the_calendar(code))
                .collect::<BTreeSet<_>>(),
            codes("weakened", "key"),
            switched_off_in_audit_log(&env),
            log.exists(),
        ),
        (
            json!("throughput"),
            0,
            MARKDOWN_PAGES + 1,
            BTreeSet::from(["\"records_off\"".to_owned()]),
            [
                "block_classification_off",
                "learning_off",
                "provenance_off",
                "records_off",
            ]
            .map(str::to_owned)
            .into(),
            [
                "dataset.records",
                "egress.private_ranges",
                "fetch.block_classification",
                "frontier.learning",
                "politeness.domain.delay",
                "provenance.log",
            ]
            .map(str::to_owned)
            .into(),
            BTreeSet::from([
                "dataset.records".to_owned(),
                "fetch.block_classification".to_owned(),
                "provenance.log".to_owned(),
            ]),
            false,
        ),
        "{report}"
    );
}

/// The `key` of each entry of the array `entries`.
fn strings(entries: &Value, key: &str) -> BTreeSet<String> {
    entries
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|entry| entry[key].as_str().map(str::to_owned))
        .collect()
}

/// The setting of each `feature_switched_off` entry of `env`'s Audit log.
fn switched_off_in_audit_log(env: &TestEnv) -> BTreeSet<String> {
    let exported = goodfetch_test_support::cli(env, env!("CARGO_BIN_EXE_goodfetch-foxhound"))
        .args(["audit", "export"])
        .run()
        .unwrap();
    exported
        .stdout
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .filter(|entry| entry["kind"] == "feature_switched_off")
        .filter_map(|entry| entry["details"]["setting"].as_str().map(str::to_owned))
        .collect()
}
