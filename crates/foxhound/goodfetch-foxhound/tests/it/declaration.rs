//! Crawls declared as data (spec #611's #1514; ADR 0103), on Foxhound's
//! executable over stdio against the Proving ground: a `crawl.start` with
//! no handler takes a `declaration`, its Labels, what each of their pages
//! follows and extracts, its Datasets and settings, and the engine handles
//! every request itself, the program never sent a `crawl_request.handle`.
//! A start naming both `extraction` and `declaration` is `setting_invalid`,
//! and a declaration holding a handler or a Hook does not parse.

use goodfetch_test_client::Attached;
use goodfetch_test_support::{ProvingGround, TestEnv, ToolStatus};
use rstest::rstest;
use serde_json::{Value, json};

use crate::attached;
use crate::fetch_tiers::settings;

/// The Proving ground's seed the catalog crawls read.
const SEED: u64 = 1467;

/// The task whose oracle judges the catalog's products.
const TASK: &str = "catalog/1";

/// How many products the catalog lists.
const PRODUCTS: usize = 100;

/// A CSS locator in its JSON form.
fn css(selector: &str) -> Value {
    json!({ "kind": "css", "selector": selector })
}

/// The catalog crawled as data: each listing page follows its Next link
/// as a listing and each product's link as a product, and each product
/// page extracts its ID, name and price into the Dataset `products`, keyed
/// by ID; the declaration's own settings send every request on the HTTP
/// tier, with no delay between them.
fn catalog() -> Value {
    json!({
        "labels": [
            {
                "name": "listing",
                "follow": [
                    { "links": "a[rel=next]", "label": "listing" },
                    { "links": "li.product a", "label": "product" },
                ],
            },
            {
                "name": "product",
                "extract": [{
                    "dataset": "products",
                    "fields": {
                        "id": { "locator": css(".id") },
                        "name": { "locator": css("h1.name") },
                        "price": { "locator": css(".price"), "attribute": "data-price" },
                    },
                }],
            },
        ],
        "datasets": [{
            "name": "products",
            "schema": {
                "type": "object",
                "properties": {
                    "id": { "type": "string" },
                    "name": { "type": "string" },
                    "price": { "type": "string" },
                },
                "required": ["id", "name", "price"],
            },
            "key": ["id"],
        }],
        "settings": {
            "fetch": { "tier": "http" },
            "politeness": { "domain": { "delay": 0 } },
        },
    })
}

/// Starts the crawl `run_id` of `seeds` from `declaration` under the
/// Proving ground's settings, waits for its Run to end, and answers the
/// start's reply and the value of every Record of `dataset`; the engine
/// must never send the program a handler request.
fn crawl(
    engine: &mut Attached,
    proving_ground: &ProvingGround,
    (run_id, seeds): (&str, &[Value]),
    declaration: &Value,
    dataset: &str,
) -> (Value, Vec<Value>) {
    let started = engine.call(
        "start",
        "crawl.start",
        &json!({
            "run_id": run_id,
            "seeds": seeds,
            "datasets": [],
            "declaration": declaration,
            "settings": settings(proving_ground, &json!({})),
        }),
        &mut |handle| panic!("a declared crawl sent its program {handle}"),
    );
    if started.get("error").is_some() {
        return (started, Vec::new());
    }
    engine.send(&json!({
        "jsonrpc": "2.0", "id": "records", "method": "run.records",
        "params": { "run_id": run_id, "dataset": dataset },
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
            "a declared crawl sent its program {message}"
        );
        if message["method"] == "$/record" {
            records.push(message["params"]["record"]["value"].clone());
        } else if message["id"] == "records" {
            assert!(message.get("result").is_some(), "{message}");
            return (started, records);
        }
    }
}

/// A crawl started from a declaration with `follow` and `extract` fills
/// the Dataset it declares on the `catalog` family with no code: its
/// listing pages follow their Next links and their products' links, each
/// product page pushes one Record of its fields into `products`, and the
/// oracle verifies the hundred products.
///
/// Covers: story:11-1069, operation:crawl.start, operation:run.records
#[test]
fn a_crawl_declared_as_data_fills_its_declared_dataset_on_the_catalog() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let mut engine = crate::site::engine_on(&env, &proving_ground);
    let start = proving_ground.start_url(TASK).unwrap();

    let (started, records) = crawl(
        &mut engine,
        &proving_ground,
        ("catalog", &[json!({ "url": start, "label": "listing" })]),
        &catalog(),
        "products",
    );
    assert!(engine.finished());

    let verdict = proving_ground
        .judge_records(TASK, ToolStatus::Success, &records)
        .unwrap();
    assert_eq!(
        (
            started["result"]["run_id"].clone(),
            records.len(),
            &verdict["outcome"]
        ),
        (json!("catalog"), PRODUCTS, &json!("verified")),
        "{verdict}"
    );
}

/// A start that cannot run as declared is refused before its Run begins:
/// one naming both `extraction` and `declaration` with `setting_invalid`, a
/// declaration holding a handler or a Hook, which none of its types
/// expresses, with `invalid_params`, a follow rule naming a Label the
/// declaration does not declare with `label_unknown`, and an extract
/// naming a Dataset it does not declare, or a start declaring a Dataset or
/// a Label beside the declaration, with `invalid_params`.
///
/// Covers: story:11-1069, operation:crawl.start
#[rstest]
#[case::extraction_and_declaration(
    json!({ "extraction": { "formats": { "markdown": true } }, "declaration": catalog() }),
    "setting_invalid"
)]
#[case::a_handler_in_the_declaration(
    json!({ "declaration": { "labels": [{ "name": "listing", "handler": "listing" }] } }),
    "invalid_params"
)]
#[case::a_hook_in_the_declaration(
    json!({ "declaration": { "labels": [{ "name": "listing" }], "hooks": [] } }),
    "invalid_params"
)]
#[case::a_follow_of_an_undeclared_label(
    json!({ "declaration": { "labels": [{
        "name": "listing",
        "follow": [{ "links": "a", "label": "product" }],
    }] } }),
    "label_unknown"
)]
#[case::an_extract_into_an_undeclared_dataset(
    json!({ "declaration": { "labels": [{
        "name": "listing",
        "extract": [{ "dataset": "products", "fields": { "id": { "locator": css(".id") } } }],
    }] } }),
    "invalid_params"
)]
#[case::a_dataset_beside_the_declaration(
    json!({ "declaration": catalog(), "datasets": [{ "name": "pages" }] }),
    "invalid_params"
)]
#[case::a_label_beside_the_declaration(
    json!({ "declaration": catalog(), "labels": { "page": {} } }),
    "invalid_params"
)]
fn a_start_that_cannot_run_as_declared_is_refused(#[case] start: Value, #[case] code: &str) {
    let env = TestEnv::new();
    let mut engine = attached(&env);
    let mut params = json!({
        "run_id": "declared",
        "seeds": [{ "url": "http://example.test/", "label": "listing" }],
        "datasets": [],
    });
    if let (Some(params), Some(start)) = (params.as_object_mut(), start.as_object()) {
        params.extend(start.clone());
    }
    let refused = engine.call("start", "crawl.start", &params, &mut |handle| {
        panic!("a refused crawl sent its program {handle}")
    });
    assert!(engine.finished());
    assert_eq!(refused["error"]["data"]["code"], json!(code), "{refused}");
}
