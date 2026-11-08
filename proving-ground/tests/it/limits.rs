//! The `limits` family (ADR 0025): the same 24 items behind three rules, one
//! per task. `limits/1` rate-limits each address with `429` and
//! `Retry-After`, and flags an address that keeps asking before that time;
//! `limits/2` lets each address fetch ten pages and flags the one that asks
//! for more; `limits/3` takes a while to answer and allows two requests in
//! flight at once. The crawls that need several addresses are in
//! `extra_loopback`.
//! Covers: story:2-33
#![expect(
    clippy::disallowed_types,
    reason = "the in-flight test opens its own connections to send requests at once"
)]

use std::io::{Read as _, Write as _};
use std::net::TcpStream;

use serde_json::{Value, json};

use crate::crawler::{Crawler, Fetched, field, finding_kinds};
use crate::proving_ground::{ProvingGround, submission};

const SEED: u64 = 164;

/// Each item's Record, once.
pub(crate) fn records(pages: &[Fetched]) -> Vec<Value> {
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

pub(crate) fn start_url(proving_ground: &ProvingGround, task: &str) -> String {
    proving_ground.task(task)["start_url"]
        .as_str()
        .expect("a start URL")
        .to_owned()
}

#[test]
fn a_crawl_that_waits_each_retry_after_is_verified() {
    let proving_ground = ProvingGround::start(SEED);
    let pages = Crawler::new(&proving_ground).crawl(&start_url(&proving_ground, "limits/1"));

    let answer = proving_ground.judge(&submission("limits/1", "success", &records(&pages)));

    assert_eq!(answer["outcome"], "verified", "{answer}");
}

#[test]
fn the_eleventh_request_of_an_address_is_rate_limited_with_retry_after() {
    let proving_ground = ProvingGround::start(SEED);
    let start = start_url(&proving_ground, "limits/1");

    let statuses: Vec<u16> = (0..11).map(|_| proving_ground.get(&start).status).collect();
    let limited = proving_ground.get(&start);

    assert_eq!(statuses[..10], [200; 10]);
    assert_eq!(statuses[10], 429);
    assert_eq!(limited.status, 429);
    assert_eq!(limited.header("retry-after"), Some("1"));
}

#[test]
fn a_crawl_that_ignores_retry_after_is_flagged_and_caught() {
    let proving_ground = ProvingGround::start(SEED);
    let mut crawler = Crawler::new(&proving_ground);
    crawler.honour_retry_after = false;
    let start = start_url(&proving_ground, "limits/1");
    let pages = crawler.crawl(&start);

    let answer = proving_ground.judge(&submission("limits/1", "success", &records(&pages)));

    assert_eq!(answer["outcome"], "false_success", "{answer}");
    assert_eq!(finding_kinds(&answer), ["retry_after_ignored"], "{answer}");
    assert!(
        !answer["missing"].as_array().unwrap().is_empty(),
        "{answer}"
    );
    assert_eq!(proving_ground.get(&start).status, 403);
}

#[test]
fn an_address_asking_for_an_eleventh_page_is_flagged_and_refused_from_then_on() {
    let proving_ground = ProvingGround::start(SEED);
    let start = start_url(&proving_ground, "limits/2");

    let statuses: Vec<u16> = (0..13).map(|_| proving_ground.get(&start).status).collect();

    assert_eq!(statuses[..10], [200; 10]);
    assert_eq!(statuses[10..], [403; 3]);
    assert!(proving_ground.get(&start).body.contains("flagged"));
}

#[test]
fn a_crawl_from_one_address_left_to_review_is_incomplete() {
    let proving_ground = ProvingGround::start(SEED);
    let pages = Crawler::new(&proving_ground).crawl(&start_url(&proving_ground, "limits/2"));

    let answer = proving_ground.judge(&submission("limits/2", "needs-review", &records(&pages)));

    assert_eq!(answer["outcome"], "incomplete", "{answer}");
    assert_eq!(answer["missing"].as_array().unwrap().len(), 15, "{answer}");
}

#[test]
fn a_crawl_from_one_address_that_claims_success_is_a_false_success() {
    let proving_ground = ProvingGround::start(SEED);
    let pages = Crawler::new(&proving_ground).crawl(&start_url(&proving_ground, "limits/2"));

    let answer = proving_ground.judge(&submission("limits/2", "success", &records(&pages)));

    assert_eq!(answer["outcome"], "false_success", "{answer}");
}

#[test]
fn a_crawl_with_one_request_in_flight_is_verified() {
    let proving_ground = ProvingGround::start(SEED);
    let pages = Crawler::new(&proving_ground).crawl(&start_url(&proving_ground, "limits/3"));

    let answer = proving_ground.judge(&submission("limits/3", "success", &records(&pages)));

    assert_eq!(answer["outcome"], "verified", "{answer}");
}

#[test]
fn a_crawl_missing_one_item_is_incomplete() {
    let proving_ground = ProvingGround::start(SEED);
    let pages = Crawler::new(&proving_ground).crawl(&start_url(&proving_ground, "limits/3"));
    let mut records = records(&pages);
    let dropped = records.pop().unwrap();

    let answer = proving_ground.judge(&submission("limits/3", "failure", &records));

    assert_eq!(answer["outcome"], "incomplete", "{answer}");
    assert_eq!(answer["missing"], json!([dropped["id"]]));
}

#[test]
fn a_third_request_in_flight_is_refused_and_caught() {
    let proving_ground = ProvingGround::start(SEED);
    let start = start_url(&proving_ground, "limits/3");
    let pages = Crawler::new(&proving_ground).crawl(&start);
    let host = start
        .strip_prefix("http://")
        .and_then(|rest| rest.split(':').next())
        .unwrap();

    let mut connections: Vec<TcpStream> = (0..3)
        .map(|_| TcpStream::connect(proving_ground.address()).unwrap())
        .collect();
    for connection in &mut connections {
        write!(
            connection,
            "GET / HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n"
        )
        .unwrap();
    }
    let mut statuses: Vec<String> = connections
        .iter_mut()
        .map(|connection| {
            let mut response = String::new();
            connection.read_to_string(&mut response).unwrap();
            response.split(' ').nth(1).unwrap().to_owned()
        })
        .collect();
    statuses.sort();
    let answer = proving_ground.judge(&submission("limits/3", "success", &records(&pages)));

    assert_eq!(statuses, ["200", "200", "503"]);
    assert_eq!(answer["outcome"], "false_success", "{answer}");
    assert_eq!(finding_kinds(&answer), ["over_in_flight_cap"], "{answer}");
}
