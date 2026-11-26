//! A code-free crawl from the Rust SDK (spec #611's #1514; ADR 0093), on
//! the linked engine: `start_crawl` takes a reviewed `CrawlDeclaration`,
//! whose seeds name their Labels by name, the engine handles every request
//! itself, and the program reads the declared Dataset by its name with the
//! raw call `dataset.page`, as the TypeScript and Python SDKs do.

use goodfetch_foxhound::{Crawl, CrawlDeclaration, Foxhound, RunId, Seed};
use goodfetch_protocol::result_document::OracleOutcome;
use goodfetch_protocol::run::Lifecycle;
use goodfetch_test_support::{ProvingGround, TestEnv, ToolStatus, real_time_runtime};
use serde_json::{Value, json};

use super::catalog::{SEED, TASK, judged};
use super::crawl::options;

/// The catalog crawled as data, as the engine's own test declares it:
/// each listing page follows its Next link as a listing and each
/// product's link as a product, and each product page extracts its ID,
/// name and price into the Dataset `products`, keyed by ID, with no delay
/// between requests.
fn catalog() -> CrawlDeclaration {
    let css = |selector: &str| json!({ "kind": "css", "selector": selector });
    serde_json::from_value(json!({
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
        "datasets": [{ "name": "products", "key": ["id"] }],
        "settings": { "politeness": { "domain": { "delay": 0 } } },
    }))
    .unwrap()
}

/// A program starts the catalog's declaration with no handler, its seed
/// naming the Label `listing`, waits for the Run to finish, and reads
/// `products` by its name; the oracle verifies the Records.
///
/// Covers: story:11-1069
#[test]
fn a_program_starts_a_declared_crawl_and_reads_its_dataset_by_name() {
    let env = TestEnv::new();
    let site = ProvingGround::start(&env, SEED).unwrap();
    let start = site.start_url(TASK).unwrap();
    let (lifecycle, records) = real_time_runtime().block_on(async {
        let foxhound = Foxhound::connect(options(&env, &site)).await.unwrap();
        let run = foxhound
            .start_crawl(catalog(), [Seed::named(&start, "listing")])
            .run_id(&RunId::parse("catalog").unwrap())
            .await
            .unwrap();
        let report = run.finish().await.unwrap().into_inner();
        let page: Value = foxhound
            .call("dataset.page")
            .params(json!({ "run_id": "catalog", "dataset": "products", "limit": 1000 }))
            .await
            .unwrap();
        foxhound.close().await.unwrap();
        let records: Vec<Value> = page["records"]
            .as_array()
            .unwrap()
            .iter()
            .map(|record| record["value"].clone())
            .collect();
        (report.report.lifecycle, records)
    });

    let answer = judged(&site, ToolStatus::Success, &records);
    assert_eq!(
        (lifecycle, answer.outcome),
        (Lifecycle::Finished, OracleOutcome::Verified),
        "{answer:?}"
    );
}

/// A code-free crawl's seed names its Label by name: one holding a
/// `Label`, of a crawl the program declares, is refused with
/// `invalid_params` before anything is sent.
///
/// Covers: story:11-1069
#[test]
fn a_code_free_seed_holding_a_label_is_refused() {
    let env = TestEnv::new();
    let listing = Crawl::new().label::<()>("listing").unwrap();
    let refused = real_time_runtime().block_on(async {
        let foxhound = Foxhound::connect(super::options(&env)).await.unwrap();
        let refused = foxhound
            .start_crawl(catalog(), [Seed::new("https://shop.test/", &listing, ())])
            .await
            .unwrap_err();
        foxhound.close().await.unwrap();
        refused
    });

    assert_eq!(refused.code().as_str(), "invalid_params", "{refused}");
}
