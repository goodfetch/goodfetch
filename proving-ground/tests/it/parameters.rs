//! The `parameters` family (ADR 0005): every listing links each product in
//! three colours. On the first listing page the colour changes nothing, as
//! `ref` never does, but further on some products have a distinct page per
//! colour. A crawl that learned the colour has no effect merges those pages,
//! and the oracle names the pages it lost.
//! Covers: story:2-36

use serde_json::{Value, json};

use crate::crawler::{Crawler, Fetched, field, fixed_rules, links};
use crate::proving_ground::{ProvingGround, submission};

const SEED: u64 = 164;
const TASK: &str = "parameters/1";

/// Each distinct product page's Record, once.
fn records(pages: &[Fetched]) -> Vec<Value> {
    let mut records: Vec<Value> = pages
        .iter()
        .filter_map(|page| {
            let body = &page.answer.body;
            Some(json!({
                "id": field(body, r#"<article class="product" data-id="([^"]+)""#)?,
                "name": field(body, r#"<h1 class="name">([^<]+)</h1>"#)?,
                "price": field(body, r#"data-price="([^"]+)""#)?,
            }))
        })
        .collect();
    records.sort_by_key(|record| record["id"].to_string());
    records.dedup();
    records
}

/// The frontier key of a crawl that learned `colour` and `ref` change
/// nothing: both dropped.
fn colour_learned_as_no_effect(url: &str) -> String {
    let key = fixed_rules(url);
    let Some((path, query)) = key.split_once('?') else {
        return key;
    };
    let kept: Vec<&str> = query
        .split('&')
        .filter(|pair| !pair.starts_with("colour=") && !pair.starts_with("ref="))
        .collect();
    format!("{path}?{}", kept.join("&"))
}

fn start_url(proving_ground: &ProvingGround) -> String {
    proving_ground.task(TASK)["start_url"]
        .as_str()
        .expect("a start URL")
        .to_owned()
}

#[test]
fn a_crawl_that_keeps_every_colour_is_verified() {
    let proving_ground = ProvingGround::start(SEED);
    let pages = Crawler::new(&proving_ground).crawl(&start_url(&proving_ground));

    let answer = proving_ground.judge(&submission(TASK, "success", &records(&pages)));

    assert_eq!(answer["outcome"], "verified", "{answer}");
}

#[test]
fn a_crawl_missing_one_product_page_is_incomplete() {
    let proving_ground = ProvingGround::start(SEED);
    let pages = Crawler::new(&proving_ground).crawl(&start_url(&proving_ground));
    let mut records = records(&pages);
    let dropped = records.pop().unwrap();

    let answer = proving_ground.judge(&submission(TASK, "needs-review", &records));

    assert_eq!(answer["outcome"], "incomplete", "{answer}");
    assert_eq!(answer["missing"], json!([dropped["id"]]));
}

#[test]
fn a_crawl_that_merges_the_colours_is_a_false_success_naming_the_pages_it_lost() {
    let proving_ground = ProvingGround::start(SEED);
    let mut crawler = Crawler::new(&proving_ground);
    crawler.key = colour_learned_as_no_effect;
    let pages = crawler.crawl(&start_url(&proving_ground));

    let answer = proving_ground.judge(&submission(TASK, "success", &records(&pages)));

    assert_eq!(answer["outcome"], "false_success", "{answer}");
    let missing = answer["missing"].as_array().unwrap();
    assert!(!missing.is_empty(), "{answer}");
    assert!(
        missing
            .iter()
            .all(|id| !id.as_str().unwrap().ends_with("-1")),
        "{answer}"
    );
    assert_eq!(answer["extra"], json!([]));
}

#[test]
fn on_the_first_listing_page_the_colour_changes_no_page() {
    let proving_ground = ProvingGround::start(SEED);
    let start = start_url(&proving_ground);
    let listing = proving_ground.get(&start).body;
    let origin = crate::crawler::origin(&start);
    let products: Vec<String> = links(&listing, false)
        .into_iter()
        .filter(|link| link.starts_with("/product?") && link.contains("colour=1"))
        .collect();

    for product in &products {
        let pages: Vec<String> = (1..=3)
            .map(|colour| {
                let url = format!(
                    "{origin}{}",
                    product.replace("colour=1", &format!("colour={colour}"))
                );
                proving_ground.get(&url).body
            })
            .collect();
        assert!(pages.iter().all(|page| page == &pages[0]), "{product}");
    }
    assert_eq!(products.len(), 12, "{listing}");
}
