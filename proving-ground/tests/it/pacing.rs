//! The `pacing` family: a host whose latency rises with the Proving
//! ground's time step, never the wall clock, so a crawl's adaptive delay has
//! something to lengthen against. Each answer names the hold it was given in
//! `Server-Timing`; the hold itself is a measurement, never asserted here
//! (ADR 0064).
//! Covers: story:2-33

use serde_json::json;

use crate::crawler::{Crawler, field};
use crate::proving_ground::ProvingGround;

const SEED: u64 = 1538;
const TASK: &str = "pacing/1";

fn start_url(proving_ground: &ProvingGround) -> String {
    proving_ground.task(TASK)["start_url"]
        .as_str()
        .expect("a start URL")
        .to_owned()
}

/// The hold, in milliseconds, each of the task's time steps gives the start
/// page.
fn curve(seed: u64) -> Vec<u64> {
    let proving_ground = ProvingGround::start(seed);
    let task = proving_ground.task(TASK);
    let steps = task["time_steps"]
        .as_u64()
        .expect("the task spans time steps");
    let start = start_url(&proving_ground);
    (0..steps)
        .map(|step| {
            assert_eq!(proving_ground.set_time_step(&json!(step)).status, 200);
            let answer = proving_ground.get(&start);
            assert_eq!(answer.status, 200, "{}", answer.body);
            let timing = answer
                .header("server-timing")
                .expect("a Server-Timing header");
            timing
                .strip_prefix("hold;dur=")
                .and_then(|ms| ms.parse().ok())
                .unwrap_or_else(|| panic!("an unexpected Server-Timing: {timing}"))
        })
        .collect()
}

#[test]
fn the_hold_rises_with_the_time_step_and_the_same_seed_gives_the_same_curve() {
    let first = curve(SEED);
    let again = curve(SEED);
    let other = curve(SEED + 1);

    assert_eq!(first.len(), 6, "{first:?}");
    assert!(first.windows(2).all(|pair| pair[0] < pair[1]), "{first:?}");
    assert_eq!(first, again);
    assert_ne!(first, other);
}

#[test]
fn the_hold_stays_at_its_time_step_however_often_the_page_is_read() {
    let proving_ground = ProvingGround::start(SEED);
    let start = start_url(&proving_ground);
    proving_ground.set_time_step(&json!(3));

    let holds: Vec<String> = (0..3)
        .map(|_| {
            proving_ground
                .get(&start)
                .header("server-timing")
                .expect("a Server-Timing header")
                .to_owned()
        })
        .collect();

    assert!(holds.iter().all(|hold| hold == &holds[0]), "{holds:?}");
}

#[test]
fn a_crawl_while_the_time_step_rises_is_verified_and_one_missing_an_article_is_incomplete() {
    let proving_ground = ProvingGround::start(SEED);
    proving_ground.set_time_step(&json!(2));
    let pages = Crawler::new(&proving_ground).crawl(&start_url(&proving_ground));
    let mut records: Vec<_> = pages
        .iter()
        .filter_map(|page| {
            let body = &page.answer.body;
            Some(json!({
                "id": field(body, r#"<article class="entry" data-id="([^"]+)""#)?,
                "title": field(body, r#"<h1 class="title">([^<]+)</h1>"#)?,
            }))
        })
        .collect();

    let verified =
        proving_ground.judge(&json!({ "task": TASK, "status": "success", "records": records }));
    let dropped = records.pop().unwrap();
    let incomplete = proving_ground
        .judge(&json!({ "task": TASK, "status": "needs-review", "records": records }));

    assert_eq!(verified["outcome"], "verified", "{verified}");
    assert_eq!(incomplete["outcome"], "incomplete", "{incomplete}");
    assert_eq!(incomplete["missing"], json!([dropped["id"]]));
}
