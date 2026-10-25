use serde_json::{Value, json};

use super::*;
use crate::env::TestEnv;

/// What a Proving ground's observations hold after a browser loaded one
/// page and then another of `catalog.test`: the engine read the origin's
/// robots.txt between them, and the browser asked for its icon.
fn observations() -> Value {
    json!({
        "requests": [
            { "host": "catalog.test", "target": "/catalog/1" },
            { "host": "catalog.test", "target": "/robots.txt" },
            { "host": "catalog.test", "target": "/favicon.ico" },
            { "host": "catalog.test", "target": "/catalog/2" },
        ],
        "connections": [],
    })
}

fn targets<'a>(requests: impl Iterator<Item = &'a Value>) -> Vec<&'a str> {
    requests
        .map(|request| request["target"].as_str().unwrap())
        .collect()
}

#[test]
fn the_requests_of_the_case_leave_out_the_robots_txt_read_and_the_icon() {
    let observations = observations();

    let received = ReceivedRequests::of(&observations);

    assert_eq!(
        targets(received.of_the_case()),
        ["/catalog/1", "/catalog/2"]
    );
}

#[test]
fn every_request_keeps_the_robots_txt_read_and_the_icon_in_arrival_order() {
    let observations = observations();

    let received = ReceivedRequests::of(&observations);

    assert_eq!(
        targets(received.every()),
        ["/catalog/1", "/robots.txt", "/favicon.ico", "/catalog/2"]
    );
}

#[test]
fn a_robots_txt_read_returns_once_the_runs_provenance_records_one() {
    let env = TestEnv::new();
    let trace = env.home().join("0.0/runs/read/trace");
    std::fs::create_dir_all(&trace).unwrap();
    std::fs::write(
        trace.join("provenance.jsonl"),
        "{\"action\":\"page_weakly_checked\"}\n{\"action\":\"robots_txt\"}\n",
    )
    .unwrap();

    robots_txt_read(&env, "read");

    assert_eq!(robots_txt_readings(&env, "read").len(), 1);
}
