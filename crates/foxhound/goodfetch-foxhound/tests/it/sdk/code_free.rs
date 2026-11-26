//! A code-free crawl from the Rust SDK (spec #611's #1514; ADR 0093), on
//! the linked engine: `start_crawl` takes a reviewed `CrawlDeclaration`,
//! whose seeds name their Labels by name, the engine handles every request
//! itself, and the program reads the declared Dataset by its name with the
//! raw call `dataset.page`, as the TypeScript and Python SDKs do.

use goodfetch_foxhound::{Crawl, CrawlDeclaration, CrawlExtraction, Foxhound, RunId, Seed};
use goodfetch_protocol::foxhound::crawl::{CODE_FREE_DATASET, CODE_FREE_LABEL};
use goodfetch_protocol::result_document::OracleOutcome;
use goodfetch_protocol::run::Lifecycle;
use goodfetch_test_support::{ProvingGround, TestEnv, ToolStatus, real_time_runtime};
use serde_json::{Value, json};

use super::catalog::{SEED, TASK, judged};
use super::crawl::options;
use super::seeds::{SITEMAP_HOST, SITEMAPS};

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

/// A program starts a `CrawlExtraction` from its seed alone, naming the
/// engine's Label `page`, and reads one Record of the page from the
/// Dataset `pages`, the page's Markdown in it.
///
/// Covers: story:11-1069
#[test]
fn a_program_starts_an_extraction_and_reads_its_pages() {
    let env = TestEnv::new();
    let site = ProvingGround::start(&env, SEED).unwrap();
    let start = site.start_url(TASK).unwrap();
    let extraction: CrawlExtraction =
        serde_json::from_value(json!({ "formats": { "markdown": true }, "follow": "none" }))
            .unwrap();
    let (lifecycle, records) = real_time_runtime().block_on(async {
        let foxhound = Foxhound::connect(options(&env, &site)).await.unwrap();
        let run = foxhound
            .start_crawl(extraction, [Seed::named(&start, CODE_FREE_LABEL)])
            .run_id(&RunId::parse("extraction").unwrap())
            .await
            .unwrap();
        let report = run.finish().await.unwrap().into_inner();
        let page: Value = foxhound
            .call("dataset.page")
            .params(json!({ "run_id": "extraction", "dataset": CODE_FREE_DATASET }))
            .await
            .unwrap();
        foxhound.close().await.unwrap();
        (
            report.report.lifecycle,
            page["records"].as_array().unwrap().clone(),
        )
    });

    let pages: Vec<(&str, bool)> = records
        .iter()
        .map(|record| {
            let markdown = record["value"]["markdown"].as_str().unwrap_or_default();
            (
                record["value"]["url"].as_str().unwrap(),
                !markdown.is_empty(),
            )
        })
        .collect();
    assert_eq!(
        (lifecycle, pages),
        (Lifecycle::Finished, vec![(start.as_str(), true)])
    );
}

/// A code-free crawl's sitemap seed names its rules' Labels by name: an
/// extraction seeded from a sitemap index, whose rule gives each book the
/// engine's Label `page`, reads one Record of each book and none of the
/// entries no rule matches.
///
/// Covers: story:11-1069
#[test]
fn a_program_seeds_an_extraction_from_a_sitemap_naming_its_label() {
    let env = TestEnv::new();
    let site = ProvingGround::start(&env, SITEMAPS).unwrap();
    let url = |path: &str| site.url(SITEMAP_HOST, path);
    let extraction: CrawlExtraction =
        serde_json::from_value(json!({ "formats": { "markdown": true }, "follow": "none" }))
            .unwrap();
    let (lifecycle, records) = real_time_runtime().block_on(async {
        let foxhound = Foxhound::connect(options(&env, &site)).await.unwrap();
        let run = foxhound
            .start_crawl(
                extraction,
                [Seed::named_sitemap(
                    url("/sitemap-index.xml"),
                    [("**/books/*", CODE_FREE_LABEL)],
                )],
            )
            .run_id(&RunId::parse("sitemap-extraction").unwrap())
            .await
            .unwrap();
        let report = run.finish().await.unwrap().into_inner();
        let page: Value = foxhound
            .call("dataset.page")
            .params(json!({ "run_id": "sitemap-extraction", "dataset": CODE_FREE_DATASET }))
            .await
            .unwrap();
        foxhound.close().await.unwrap();
        (
            report.report.lifecycle,
            page["records"].as_array().unwrap().clone(),
        )
    });

    let mut pages: Vec<&str> = records
        .iter()
        .map(|record| record["value"]["url"].as_str().unwrap())
        .collect();
    pages.sort_unstable();
    assert_eq!(
        (lifecycle, pages),
        (
            Lifecycle::Finished,
            vec![
                url("/books/1").as_str(),
                url("/books/2").as_str(),
                url("/books/3").as_str(),
            ]
        )
    );
}

/// The linked engine ends with its Connection, so a detached code-free
/// crawl is refused with `detach_unsupported`.
///
/// Covers: story:11-1069
#[test]
fn a_detached_code_free_crawl_is_refused_on_an_engine_that_is_no_daemon() {
    let env = TestEnv::new();
    let refused = real_time_runtime().block_on(async {
        let foxhound = Foxhound::connect(super::options(&env)).await.unwrap();
        let refused = foxhound
            .start_crawl(catalog(), [Seed::named("https://shop.test/", "listing")])
            .detached()
            .await
            .unwrap_err();
        foxhound.close().await.unwrap();
        refused
    });

    assert_eq!(refused.code().as_str(), "detach_unsupported", "{refused}");
}

/// A code-free crawl's seed names its Label by name: a request seed or a
/// sitemap seed holding a `Label`, of a crawl the program declares, is
/// refused with `invalid_params` before anything is sent.
///
/// Covers: story:11-1069
#[test]
fn a_code_free_seed_holding_a_label_is_refused() {
    let env = TestEnv::new();
    let listing = Crawl::new().label::<()>("listing").unwrap();
    let refused = real_time_runtime().block_on(async {
        let foxhound = Foxhound::connect(super::options(&env)).await.unwrap();
        let mut refused = Vec::new();
        for seed in [
            Seed::new("https://shop.test/", &listing, ()),
            Seed::sitemap("https://shop.test/sitemap.xml", [("**", &listing)]),
        ] {
            let error = foxhound.start_crawl(catalog(), [seed]).await.unwrap_err();
            refused.push(error.code().as_str().to_owned());
        }
        foxhound.close().await.unwrap();
        refused
    });

    assert_eq!(refused, ["invalid_params", "invalid_params"]);
}
