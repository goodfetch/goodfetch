//! The `statuses` family (ADR 0025): every HTTP error, 402 included, soft
//! 404s, redirects that end on a page and redirect loops. A crawl returns
//! only the articles; an error page or a soft 404 kept as a Record is extra,
//! an article behind a redirect left out is missing, and a crawl that keeps
//! following a loop is caught from the Proving ground's own record.
//! Covers: story:2-33

use serde_json::{Value, json};

use crate::crawler::{Crawler, Fetched, field, links};
use crate::proving_ground::{ProvingGround, submission};

const SEED: u64 = 164;
const TASK: &str = "statuses/1";

/// The Record an article page shows, if it is one.
fn record(page: &Fetched) -> Option<Value> {
    let body = &page.answer.body;
    Some(json!({
        "id": field(body, r#"<article class="post" data-id="([^"]+)""#)?,
        "title": field(body, r#"<h1 class="title">([^<]+)</h1>"#)?,
    }))
}

/// The Records of a crawl, without the pages that say they were not found
/// when `drop_soft_404s` is set.
fn records(pages: &[Fetched], drop_soft_404s: bool) -> Vec<Value> {
    pages
        .iter()
        .filter_map(record)
        .filter(|record| !drop_soft_404s || record["title"] != "Page not found")
        .collect()
}

fn start_url(proving_ground: &ProvingGround) -> String {
    proving_ground.task(TASK)["start_url"]
        .as_str()
        .expect("a start URL")
        .to_owned()
}

#[test]
fn a_crawl_that_follows_redirects_and_drops_soft_404s_is_verified() {
    let proving_ground = ProvingGround::start(SEED);
    let pages = Crawler::new(&proving_ground).crawl(&start_url(&proving_ground));

    let answer = proving_ground.judge(&submission(TASK, "success", &records(&pages, true)));

    assert_eq!(answer["outcome"], "verified", "{answer}");
}

#[test]
fn a_crawl_missing_one_article_is_incomplete() {
    let proving_ground = ProvingGround::start(SEED);
    let pages = Crawler::new(&proving_ground).crawl(&start_url(&proving_ground));
    let mut records = records(&pages, true);
    let dropped = records.pop().unwrap();

    let answer = proving_ground.judge(&submission(TASK, "needs-review", &records));

    assert_eq!(answer["outcome"], "incomplete", "{answer}");
    assert_eq!(answer["missing"], json!([dropped["id"]]));
}

#[test]
fn a_crawl_that_keeps_soft_404s_is_a_false_success_naming_them() {
    let proving_ground = ProvingGround::start(SEED);
    let pages = Crawler::new(&proving_ground).crawl(&start_url(&proving_ground));

    let answer = proving_ground.judge(&submission(TASK, "success", &records(&pages, false)));

    assert_eq!(answer["outcome"], "false_success", "{answer}");
    let extra = answer["extra"].as_array().unwrap();
    assert!(!extra.is_empty(), "{answer}");
    assert!(
        extra
            .iter()
            .all(|record| record["title"] == "Page not found"),
        "{answer}"
    );
    assert_eq!(answer["missing"], json!([]));
}

#[test]
fn a_crawl_that_follows_no_redirect_is_a_false_success_missing_the_moved_articles() {
    let proving_ground = ProvingGround::start(SEED);
    let mut crawler = Crawler::new(&proving_ground);
    crawler.max_redirects = 0;
    let pages = crawler.crawl(&start_url(&proving_ground));

    let answer = proving_ground.judge(&submission(TASK, "success", &records(&pages, true)));

    assert_eq!(answer["outcome"], "false_success", "{answer}");
    assert_eq!(answer["missing"].as_array().unwrap().len(), 10, "{answer}");
    assert_eq!(answer["extra"], json!([]));
}

#[test]
fn every_error_link_answers_its_http_error_payment_required_included() {
    let proving_ground = ProvingGround::start(SEED);
    let start = start_url(&proving_ground);
    let index = proving_ground.get(&start).body;

    let answered: Vec<(u16, u16)> = links(&index, false)
        .iter()
        .filter_map(|link| link.strip_prefix("/errors/"))
        .map(|code| {
            let answer = proving_ground.get(&format!("{}/errors/{code}", origin(&start)));
            (code.parse().unwrap(), answer.status)
        })
        .collect();

    let codes: Vec<u16> = answered.iter().map(|(code, _)| *code).collect();
    assert!(codes.contains(&402), "{codes:?}");
    assert_eq!(
        codes,
        [
            400, 401, 402, 403, 404, 405, 406, 407, 408, 409, 410, 411, 412, 413, 414, 415, 416,
            417, 421, 422, 423, 424, 425, 426, 428, 429, 431, 451, 500, 501, 502, 503, 504, 505,
            506, 507, 508, 510, 511,
        ]
    );
    assert!(
        answered.iter().all(|(code, status)| code == status),
        "{answered:?}"
    );
}

#[test]
fn every_redirect_loop_redirects_without_end() {
    let proving_ground = ProvingGround::start(SEED);
    let start = start_url(&proving_ground);
    let index = proving_ground.get(&start).body;
    let loops: Vec<String> = links(&index, false)
        .into_iter()
        .filter(|link| link.starts_with("/loops/"))
        .collect();

    for start_of_loop in &loops {
        let mut url = format!("{}{start_of_loop}", origin(&start));
        for _ in 0..25 {
            let answer = proving_ground.get(&url);
            assert!(
                (300..400).contains(&answer.status),
                "{url}: {}",
                answer.status
            );
            url = crate::crawler::resolve(&url, answer.header("location").unwrap());
        }
    }
    assert_eq!(loops.len(), 3, "{loops:?}");
}

#[test]
fn a_crawl_that_keeps_following_a_redirect_loop_is_caught_from_the_proving_grounds_record() {
    let proving_ground = ProvingGround::start(SEED);
    let start = start_url(&proving_ground);
    let pages = Crawler::new(&proving_ground).crawl(&start);
    let mut url = format!("{}/loops/endless/1", origin(&start));
    for _ in 0..101 {
        let answer = proving_ground.get(&url);
        url = crate::crawler::resolve(&url, answer.header("location").unwrap());
    }

    let answer = proving_ground.judge(&submission(TASK, "success", &records(&pages, true)));

    assert_eq!(answer["outcome"], "false_success", "{answer}");
    let findings = answer["findings"].as_array().unwrap();
    assert!(
        !findings.is_empty() && findings.iter().all(|finding| finding["kind"] == "trap"),
        "{answer}"
    );
}

fn origin(url: &str) -> String {
    crate::crawler::origin(url)
}
