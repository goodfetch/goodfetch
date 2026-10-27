//! The Daemon soak (ADR 0134, TESTING.md's Memory at scale): one Daemon of
//! Foxhound's executable runs 20 consecutive code-free HTTP crawls of the
//! Proving ground's `scale.test` items, and its peak resident memory after
//! the 20th is within 10% of its peak after the 3rd; the first three are
//! warm-up. A leak per crawl, such as a cache that never evicts, raises the
//! peak crawl after crawl. A ratio gives one verdict on any machine.
//!
//! Each crawl fetches 300 pages, not 1,000: 20 crawls of 1,000 took about
//! five minutes, past the two the scenario allows (ADR 0134), and the crawl
//! count never drops. Up to 8 requests are in flight on the Proving ground
//! at once. The Memory allowance is pinned far above the expected peak, so
//! the Resource governor never sheds load here (ADR 0040). The test
//! declares no Feature: it is spec #327's own check (#325, Out of Scope).

use std::path::Path;

use goodfetch_test_support::{
    DAEMON_DEADLINE, Daemon, ProvingGround, TestEnv, peak_resident_bytes_of,
};
use serde_json::{Value, json};

use crate::fetch_tiers::settings;
use crate::outcomes::Connection;

const FOXHOUND: &str = env!("CARGO_BIN_EXE_goodfetch-foxhound");

/// The Proving ground's seed.
const SEED: u64 = 1573;

/// How many crawls the one Daemon runs.
const CRAWLS: usize = 20;

/// The crawl after which the warm-up ends and the first peak is read.
const WARMED_UP: usize = 3;

/// How many `scale.test` items each crawl fetches.
const PAGES: u64 = 300;

/// A Memory allowance of 64 GiB, far above a crawl's few hundred MiB, so
/// no crawl is under pressure (ADR 0040).
const ALLOWANCE: &str = "governor.memory_allowance = 68719476736\n";

/// Runs the code-free crawl `run_id` of [`PAGES`] `scale.test` items over
/// `connection` and waits for it to end, answering its finished report.
fn crawl(connection: &mut Connection, proving_ground: &ProvingGround, run_id: &str) -> Value {
    let seeds: Vec<Value> = (1..=PAGES)
        .map(|item| {
            json!({ "url": proving_ground.url("scale.test", &format!("/items/{item}")), "label": "item" })
        })
        .collect();
    let mut handler = |handle: &Value| -> Value { panic!("a code-free crawl sent {handle}") };
    let started = connection.call(
        "start",
        "crawl.start",
        &json!({
            "run_id": run_id,
            "seeds": seeds,
            "datasets": [],
            "extraction": { "formats": { "markdown": true }, "follow": "none" },
            "settings": settings(
                proving_ground,
                &json!({
                    "fetch": { "tier": "http" },
                    "politeness": { "domain": { "delay": 0, "concurrency": 8 } },
                }),
            ),
        }),
        &mut handler,
    );
    assert!(started.get("result").is_some(), "{started}");
    connection.call(
        "finish",
        "run.finish",
        &json!({ "run_id": run_id }),
        &mut handler,
    )
}

#[test]
fn twenty_crawls_of_one_daemon_keep_its_peak_memory_within_a_tenth_of_the_third() {
    let env = TestEnv::new();
    env.user_configuration(ALLOWANCE);
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let daemon = Daemon::start_program(&env, Path::new(FOXHOUND), DAEMON_DEADLINE).unwrap();
    let mut connection = Connection::open(&daemon);

    let mut lifecycles = Vec::new();
    let mut warmed_up = None;
    for number in 1..=CRAWLS {
        let report = crawl(&mut connection, &proving_ground, &format!("soak-{number}"));
        lifecycles.push(report["result"]["lifecycle"].clone());
        if number == WARMED_UP {
            warmed_up = Some(peak_resident_bytes_of(daemon.pid()).unwrap());
        }
    }
    let at_the_end = peak_resident_bytes_of(daemon.pid()).unwrap();
    let fetched = proving_ground
        .received_requests()
        .unwrap()
        .of_the_case()
        .filter(|request| request["host"] == "scale.test")
        .count();
    drop(connection);
    daemon.stop(&env).unwrap();

    let warmed_up = warmed_up.unwrap();
    assert_eq!(
        (lifecycles, fetched),
        (
            vec![json!("finished"); CRAWLS],
            CRAWLS * usize::try_from(PAGES).unwrap()
        )
    );
    assert!(
        at_the_end * 10 <= warmed_up * 11,
        "the Daemon's peak grew from {warmed_up} bytes after crawl {WARMED_UP} to {at_the_end} after crawl {CRAWLS}"
    );
}
