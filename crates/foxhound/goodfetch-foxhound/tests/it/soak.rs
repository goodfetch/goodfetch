//! The Daemon soak (ADR 0134, TESTING.md's Memory at scale): one Daemon of
//! Foxhound's executable runs 40 consecutive code-free HTTP crawls of the
//! Proving ground's `scale.test` items, and a line fitted by least squares
//! to its live heap after each of crawls 26 to 40, once warm, climbs at
//! most 32 KiB a crawl, about 110 bytes a page. A leak per crawl, such as a
//! cache that never evicts or a registry that keeps each ended Run's
//! report, raises it crawl after crawl, wherever in those fifteen crawls it
//! starts; a slope in bytes gives one verdict on any machine.
//!
//! The live heap, not the private memory the operating system sees: the
//! bytes the Daemon's own allocations hold, as its executable's allocator
//! counts them, which `daemon.status` answers (#1734). The allocator keeps
//! pages it freed, per thread and by the megabyte, so the private memory of
//! a Daemon with no leak swung by up to 25 MiB over these crawls and
//! climbed up to 1.4 MiB a crawl, while its live heap stayed flat (#1729's
//! heap profiles): a slope or ratio of private memory either fails clean
//! runs or misses a leak of #1729's size, about 110 KB a Run. The live heap
//! of clean runs climbs a few KiB a crawl, the bound sits far above that,
//! and a planted leak of 110 KB a Run climbs about 110 KiB a crawl, over
//! three times the bound (ADR 0134's note gives the runs). The private
//! memory after each crawl is read too, and a failure prints it beside the
//! heap. The crawls before the 26th are left out as warm-up, as when the
//! soak read private memory, whose growth stopped about crawl 22 (fix-queue
//! 128).
//!
//! Each crawl fetches 300 pages, not 1,000: the 40 crawls took 61 s in
//! the CI test profile and 145 to 246 s in the local one under load (ADR
//! 0134), and the crawl count never drops. Up to 8 requests are in flight on the Proving
//! ground at once. The Runs' stores sit on a memory file system, which no
//! busy disk slows. The Memory allowance is pinned far above the expected
//! peak, so the Resource governor never sheds load here (ADR 0040). The
//! test declares no Feature: it is spec #327's own check (#325, Out of
//! Scope).

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

/// The crawls, numbered from 1, whose live heap a line is fitted to: every
/// crawl once the Daemon is warm.
const FITTED: std::ops::RangeInclusive<usize> = 26..=CRAWLS;

/// The steepest that line may climb, in bytes a crawl: 32 KiB, far above
/// clean runs' few KiB and under a third of a leak of 110 KB a Run (the
/// module's comment).
const MOST_GROWTH_A_CRAWL: i128 = 32 * 1024;

/// How many `scale.test` items each crawl fetches.
const PAGES: u64 = 300;

/// A Memory allowance of 64 GiB, far above a crawl's few hundred MiB, so
/// no crawl is under pressure (ADR 0040).
const ALLOWANCE: &str = "governor.memory_allowance = 68719476736\n";

/// Fails on any handler request: a code-free crawl sends none.
fn no_handler(handle: &Value) -> Value {
    panic!("a code-free crawl sent {handle}")
}

/// Runs the code-free crawl `run_id` of [`PAGES`] `scale.test` items over
/// `connection` and waits for it to end, answering its finished report.
fn crawl(connection: &mut Connection, proving_ground: &ProvingGround, run_id: &str) -> Value {
    let seeds: Vec<Value> = (1..=PAGES)
        .map(|item| {
            json!({ "url": proving_ground.url("scale.test", &format!("/items/{item}")), "label": "item" })
        })
        .collect();
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
        &mut no_handler,
    );
    assert!(started.get("result").is_some(), "{started}");
    connection.call(
        "finish",
        "run.finish",
        &json!({ "run_id": run_id }),
        &mut no_handler,
    )
}

/// The Daemon's live heap, as its `daemon.status` over `connection`
/// answers it.
fn live_heap(connection: &mut Connection) -> u64 {
    let status = connection.call("status", "daemon.status", &json!({}), &mut no_handler);
    status["result"]["heap"]
        .as_u64()
        .unwrap_or_else(|| panic!("the Daemon answered no live heap: {status}"))
}

/// The slope of the least-squares line through the live heap `after`
/// holds for the crawls `crawls`, numbered from 1, in bytes a crawl,
/// rounded toward zero.
fn slope_of(after: &[u64], crawls: std::ops::RangeInclusive<usize>) -> i128 {
    let points: Vec<(i128, i128)> = crawls
        .map(|crawl| (i128::try_from(crawl).unwrap(), i128::from(after[crawl - 1])))
        .collect();
    let count = i128::try_from(points.len()).unwrap();
    let total: i128 = points.iter().map(|(crawl, _)| crawl).sum();
    // Each crawl's distance from the mean crawl, times `count`, so that the
    // arithmetic stays in integers: the slope is the sum of each distance
    // times its heap over the sum of each distance times its crawl.
    let distance = |crawl: i128| count * crawl - total;
    let rise: i128 = points
        .iter()
        .map(|(crawl, held)| distance(*crawl) * held)
        .sum();
    let run: i128 = points
        .iter()
        .map(|(crawl, _)| distance(*crawl) * crawl)
        .sum();
    rise / run
}

#[test]
fn forty_crawls_of_one_daemon_keep_its_live_heap_flat_once_warm() {
    let env = TestEnv::in_memory();
    env.user_configuration(ALLOWANCE);
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let daemon = Daemon::start_program(&env, Path::new(FOXHOUND), DAEMON_DEADLINE).unwrap();
    let mut connection = Connection::open(&daemon);

    let mut lifecycles = Vec::new();
    let (mut heap, mut private) = (Vec::new(), Vec::new());
    for number in 1..=CRAWLS {
        let report = crawl(&mut connection, &proving_ground, &format!("soak-{number}"));
        lifecycles.push(report["result"]["lifecycle"].clone());
        heap.push(live_heap(&mut connection));
        private.push(private_resident_bytes_of(daemon.pid()).unwrap());
    }
    let fetched = proving_ground
        .received_requests()
        .unwrap()
        .of_the_case()
        .filter(|request| request["host"] == "scale.test")
        .count();
    drop(connection);
    daemon.stop(&env).unwrap();

    assert_eq!(
        (lifecycles, fetched),
        (
            vec![json!("finished"); CRAWLS],
            CRAWLS * usize::try_from(PAGES).unwrap()
        )
    );
    let slope = slope_of(&heap, FITTED);
    assert!(
        slope <= MOST_GROWTH_A_CRAWL,
        "the Daemon's live heap climbed {slope} bytes a crawl over crawls {FITTED:?}, more than {MOST_GROWTH_A_CRAWL}; its live heap after each crawl: {heap:?}; its private memory: {private:?}"
    );
}
