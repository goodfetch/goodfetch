//! The `facets` family (ADR 0120's faceted listing): a shop whose listing
//! links every value of every facet it has not set, so its URLs grow
//! combinatorially. A crawl bounds it with a path budget or an exclude
//! pattern; the Proving ground records a `trap` finding for each faceted
//! listing requested past the task's allowance.
//! Covers: story:2-33

use serde_json::{Value, json};

use crate::crawler::{Crawler, Fetched, field, finding_kinds, links};
use crate::proving_ground::{ProvingGround, submission};

const SEED: u64 = 1538;
const TASK: &str = "facets/1";

fn records(pages: &[Fetched]) -> Vec<Value> {
    pages
        .iter()
        .filter_map(|page| {
            let body = &page.answer.body;
            Some(json!({
                "id": field(body, r#"<article class="entry" data-id="([^"]+)""#)?,
                "name": field(body, r#"<h1 class="name">([^<]+)</h1>"#)?,
            }))
        })
        .collect()
}

fn start_url(proving_ground: &ProvingGround) -> String {
    proving_ground.task(TASK)["start_url"]
        .as_str()
        .expect("a start URL")
        .to_owned()
}

#[test]
fn a_crawl_that_excludes_the_facet_parameters_is_verified() {
    let proving_ground = ProvingGround::start(SEED);
    let mut crawler = Crawler::new(&proving_ground);
    crawler.closes = vec![("/shop?", 0)];
    let pages = crawler.crawl(&start_url(&proving_ground));

    let answer = proving_ground.judge(&submission(TASK, "success", &records(&pages)));

    assert_eq!(answer["outcome"], "verified", "{answer}");
    assert_eq!(answer["findings"], json!([]));
    assert_eq!(records(&pages).len(), 24, "{answer}");
}

#[test]
fn a_crawl_within_a_path_budget_is_verified() {
    let proving_ground = ProvingGround::start(SEED);
    let mut crawler = Crawler::new(&proving_ground);
    crawler.closes = vec![("/shop?", 20)];
    let pages = crawler.crawl(&start_url(&proving_ground));

    let answer = proving_ground.judge(&submission(TASK, "success", &records(&pages)));

    assert_eq!(answer["outcome"], "verified", "{answer}");
    assert_eq!(answer["findings"], json!([]));
}

#[test]
fn a_crawl_that_follows_every_facet_is_a_false_success_with_trap_findings() {
    let proving_ground = ProvingGround::start(SEED);
    let mut crawler = Crawler::new(&proving_ground);
    crawler.limit = 300;
    let pages = crawler.crawl(&start_url(&proving_ground));

    let answer = proving_ground.judge(&submission(TASK, "success", &records(&pages)));

    assert_eq!(answer["outcome"], "false_success", "{answer}");
    assert_eq!(answer["missing"], json!([]), "{answer}");
    assert_eq!(finding_kinds(&answer), ["trap"], "{answer}");
}

#[test]
fn each_faceted_listing_links_every_value_of_each_facet_it_has_not_set() {
    let proving_ground = ProvingGround::start(SEED);
    let start = start_url(&proving_ground);
    let origin = crate::crawler::origin(&start);
    let facet_links = |path: &str| -> Vec<String> {
        let answer = proving_ground.get(&format!("{origin}{path}"));
        assert_eq!(answer.status, 200, "{path}");
        links(&answer.body, false)
            .into_iter()
            .filter(|link| link.starts_with("/shop?"))
            .collect()
    };

    let top = facet_links("/shop");
    let one = facet_links("/shop?colour=red");
    let two = facet_links("/shop?colour=red&size=m");

    // Four colours, four sizes, three materials and three sort orders.
    assert_eq!(top.len(), 14, "{top:?}");
    assert_eq!(one.len(), 10, "{one:?}");
    assert_eq!(two.len(), 6, "{two:?}");
    assert!(
        two.iter()
            .all(|link| link.starts_with("/shop?colour=red&size=m&")),
        "{two:?}"
    );
    assert_eq!(
        proving_ground
            .get(&format!("{origin}/shop?colour=red&colour=blue"))
            .status,
        404
    );
}
