//! The Daemon soak (ADR 0134, TESTING.md's Memory at scale): one Daemon of
//! Foxhound's executable runs 40 consecutive code-free HTTP crawls of the
//! Proving ground's `scale.test` items, and its private resident memory
//! over the last five crawls is within 10% of what it held over crawls 26
//! to 30, once warm. Each is the median of the Daemon's private memory
//! after each crawl of the five. A leak per crawl, such as a cache that
//! never evicts, raises it crawl after crawl. A ratio gives one verdict on
//! any machine.
//!
//! Private memory, not the whole resident set: some 90 MB of that is the
//! executable's pages read from its file, which no leak grows and which
//! dilute the ratio. The Daemon's allocator returns the pages it frees at
//! once (`MIMALLOC_PURGE_DELAY=0`), so what it freed during a crawl is not
//! read as held after it. Once warm, not after the 3rd crawl: in 50-crawl
//! runs the Daemon's private memory grew by 20 to 30 MB up to about crawl
//! 22 and held flat from crawl 26 to 50, within 3% (fix-queue 128, ADR
//! 0134's note). A median of five crawls takes no one crawl's reading for
//! the Daemon's.
//!
//! Each crawl fetches 300 pages, not 1,000: the 40 crawls took 61 s in
//! the CI test profile and 145 to 246 s in the local one under load (ADR
//! 0134), and the crawl count never drops. Up to 8 requests are in flight on the Proving
//! ground at once. The Runs' stores sit on a memory file system, which no
//! busy disk slows, and which the Daemon's private memory does not count.
//! The Memory allowance is pinned far above the expected peak, so the
//! Resource governor never sheds load here (ADR 0040). The test declares
//! no Feature: it is spec #327's own check (#325, Out of Scope).

use std::path::Path;

use goodfetch_test_support::{
    DAEMON_DEADLINE, Daemon, ProvingGround, TestEnv, private_resident_bytes_of,
};
use serde_json::{Value, json};

use crate::fetch_tiers::settings;
use crate::outcomes::Connection;

const FOXHOUND: &str = env!("CARGO_BIN_EXE_goodfetch-foxhound");

/// The Proving ground's seed.
const SEED: u64 = 1573;

/// How many crawls the one Daemon runs.
const CRAWLS: usize = 40;

/// The crawls, numbered from 1, after which the Daemon is warm and its
/// private memory is first read.
const WARMED_UP: std::ops::RangeInclusive<usize> = 26..=30;

/// The last crawls, whose private memory is held to the warm Daemon's.
const LAST: std::ops::RangeInclusive<usize> = 36..=CRAWLS;

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

/// The median of the private memory `after` holds for the crawls
/// `crawls`, numbered from 1.
fn median_of(after: &[u64], crawls: std::ops::RangeInclusive<usize>) -> u64 {
    let mut held = after[crawls.start() - 1..*crawls.end()].to_vec();
    held.sort_unstable();
    held[held.len() / 2]
}

#[test]
fn forty_crawls_of_one_daemon_keep_its_private_memory_within_a_tenth_once_warm() {
    let env = TestEnv::in_memory().with_variable("MIMALLOC_PURGE_DELAY", "0");
    env.user_configuration(ALLOWANCE);
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let daemon = Daemon::start_program(&env, Path::new(FOXHOUND), DAEMON_DEADLINE).unwrap();
    let mut connection = Connection::open(&daemon);

    let mut lifecycles = Vec::new();
    let mut after = Vec::new();
    for number in 1..=CRAWLS {
        let report = crawl(&mut connection, &proving_ground, &format!("soak-{number}"));
        lifecycles.push(report["result"]["lifecycle"].clone());
        after.push(private_resident_bytes_of(daemon.pid()).unwrap());
    }
    let fetched = proving_ground
        .received_requests()
        .unwrap()
        .of_the_case()
        .filter(|request| request["host"] == "scale.test")
        .count();
    drop(connection);
    daemon.stop(&env).unwrap();

    let (warm, last) = (median_of(&after, WARMED_UP), median_of(&after, LAST));
    assert_eq!(
        (lifecycles, fetched),
        (
            vec![json!("finished"); CRAWLS],
            CRAWLS * usize::try_from(PAGES).unwrap()
        )
    );
    assert!(
        last * 10 <= warm * 11,
        "the Daemon's private memory grew from {warm} bytes over crawls {WARMED_UP:?} to {last} over crawls {LAST:?}; after each crawl: {after:?}"
    );
}
