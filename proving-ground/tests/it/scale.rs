//! The `scale` family (story 28): 4,194,304 item pages generated on request
//! from the seed, in sections of 256, listed by section pages and by
//! sitemaps. Its tasks crawl one section from its listing and another from
//! its sitemap index.
//! Covers: story:2-28, story:2-33

use proptest::prelude::*;
use serde_json::{Value, json};

use crate::crawler::{Crawler, Fetched, field, links};
use crate::proving_ground::ProvingGround;

const SEED: u64 = 164;
const ITEMS: u64 = 4_194_304;

fn records(pages: &[Fetched]) -> Vec<Value> {
    pages
        .iter()
        .filter_map(|page| {
            let body = &page.answer.body;
            Some(json!({
                "id": field(body, r#"<article class="item" data-id="([^"]+)""#)?,
                "name": field(body, r#"<h1 class="name">([^<]+)</h1>"#)?,
            }))
        })
        .collect()
}

fn start_url(proving_ground: &ProvingGround, task: &str) -> String {
    proving_ground.task(task)["start_url"]
        .as_str()
        .expect("a start URL")
        .to_owned()
}

fn judge(proving_ground: &ProvingGround, task: &str, status: &str, records: &[Value]) -> Value {
    proving_ground
        .judge(&json!({ "task": task, "status": status, "records": records }))
        .json()
}

/// A crawl that stays in the start page's section: its listing or its
/// sitemaps, and the items they list.
fn section_crawl(proving_ground: &ProvingGround, task: &str) -> Vec<Fetched> {
    let start = start_url(proving_ground, task);
    let mut crawler = Crawler::new(proving_ground);
    // The item pages link back to their section; the section is the scope.
    crawler.closes = vec![("/sections/", 3)];
    crawler.crawl(&start)
}

#[test]
fn a_crawl_of_a_section_from_its_listing_is_verified() {
    let proving_ground = ProvingGround::start(SEED);
    let pages = section_crawl(&proving_ground, "scale/1");

    let answer = judge(&proving_ground, "scale/1", "success", &records(&pages));

    assert_eq!(answer["outcome"], "verified", "{answer}");
    assert_eq!(records(&pages).len(), 256);
}

#[test]
fn a_crawl_of_a_section_from_its_sitemap_index_is_verified() {
    let proving_ground = ProvingGround::start(SEED);
    let pages = section_crawl(&proving_ground, "scale/2");

    let answer = judge(&proving_ground, "scale/2", "success", &records(&pages));

    assert_eq!(answer["outcome"], "verified", "{answer}");
}

#[test]
fn a_crawl_missing_one_item_is_incomplete() {
    let proving_ground = ProvingGround::start(SEED);
    let pages = section_crawl(&proving_ground, "scale/1");
    let mut records = records(&pages);
    let dropped = records.pop().unwrap();

    let answer = judge(&proving_ground, "scale/1", "failure", &records);

    assert_eq!(answer["outcome"], "incomplete", "{answer}");
    assert_eq!(answer["missing"], json!([dropped["id"]]));
}

#[test]
fn a_crawl_stopped_early_that_claims_success_is_a_false_success() {
    let proving_ground = ProvingGround::start(SEED);
    let mut crawler = Crawler::new(&proving_ground);
    crawler.limit = 100;
    let pages = crawler.crawl(&start_url(&proving_ground, "scale/2"));

    let answer = judge(&proving_ground, "scale/2", "success", &records(&pages));

    assert_eq!(answer["outcome"], "false_success", "{answer}");
    assert!(
        answer["missing"].as_array().unwrap().len() > 150,
        "{answer}"
    );
}

#[test]
fn the_family_serves_millions_of_items_listed_by_its_sitemaps() {
    let proving_ground = ProvingGround::start(SEED);
    let start = start_url(&proving_ground, "scale/1");
    let origin = crate::crawler::origin(&start);
    let get = |path: &str| proving_ground.get(&format!("{origin}{path}"));

    let index = get("/sitemap.xml");
    let sitemaps = links(&index.body, false);
    let last_sitemap = get(&sitemaps.last().unwrap()[origin.len()..]);
    let last_items = links(&last_sitemap.body, false);

    assert_eq!(index.status, 200);
    assert_eq!(index.header("content-type"), Some("application/xml"));
    assert_eq!(sitemaps.len(), 128);
    assert_eq!(last_items.len(), 32_768);
    assert_eq!(
        last_items.last().unwrap(),
        &format!("{origin}/items/{ITEMS}")
    );
    assert_eq!(get(&format!("/items/{ITEMS}")).status, 200);
    assert_eq!(get(&format!("/items/{}", ITEMS + 1)).status, 404);
}

#[test]
fn another_seed_serves_other_items() {
    let first = ProvingGround::start(SEED);
    let second = ProvingGround::start(SEED + 1);
    let section = |proving_ground: &ProvingGround| {
        proving_ground
            .get(&format!(
                "http://scale.test:{}/sections/1",
                proving_ground.port()
            ))
            .body
    };

    assert_ne!(section(&first), section(&second));
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(16))]

    /// Covers: story:2-30
    #[test]
    fn the_same_seed_serves_any_item_byte_for_byte(seed: u64, item in 1..=ITEMS) {
        let first = ProvingGround::start(seed);
        let second = ProvingGround::start(seed);
        let page = |proving_ground: &ProvingGround| {
            proving_ground.get(&format!("http://scale.test:{}/items/{item}", proving_ground.port()))
        };

        let (one, other) = (page(&first), page(&second));

        prop_assert_eq!(one.status, 200);
        prop_assert_eq!(one.body, other.body);
    }
}
