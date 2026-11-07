//! The event stream on Collie's executable over stdio (spec #213's Seam 1;
//! ADR 0094: axis changes are Collie's): a subscriber that grants no credits
//! holds back none of Collie's Runs; its full queue falls behind, and once
//! it grants credits it reads the axes of every Run in one `events.resync`
//! (ADR 0024). Credits are granted, never timed (ADR 0064).

use std::collections::BTreeSet;

use goodfetch_protocol::catalog::Product;
use goodfetch_test_support::TestEnv;
use serde_json::json;

use goodfetch_test_client::{Attached, hello};

use crate::COLLIE;

/// How many lossless events a subscription's queue holds by default.
const QUEUE: usize = 64;

/// Covers: story:3-8, operation:events.subscribe, event:events.resync
#[test]
fn a_subscriber_that_grants_no_credits_holds_back_none_of_collies_runs_and_then_reads_their_axes_in_a_resync()
 {
    const RUNS: usize = QUEUE + 3;
    let env = TestEnv::new();
    let mut engine = Attached::start(&env, COLLIE);
    engine.send(&hello(Product::Collie));
    let _ = engine.next();
    engine.send(
        &json!({ "jsonrpc": "2.0", "id": "events", "method": "events.subscribe", "params": {} }),
    );
    for index in 0..RUNS {
        engine.send(&json!({
            "jsonrpc": "2.0", "id": index, "method": "run.start",
            "params": { "run_id": format!("run-{index}") },
        }));
    }

    // No credit is granted yet, so every message is a Run's answer: one
    // the subscriber held back would never come.
    let answered = (0..RUNS)
        .filter(|_| engine.next().unwrap()["result"]["lifecycle"] == "running")
        .count();
    engine.send(&json!({
        "jsonrpc": "2.0", "method": "$/credit",
        "params": { "id": "events", "credits": RUNS },
    }));
    let batch = engine.next().unwrap();
    let (rest, exited) = engine.finish();

    let events = batch["params"]["events"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let runs: BTreeSet<(&str, &str)> = events
        .first()
        .and_then(|resync| resync["data"]["runs"].as_array())
        .into_iter()
        .flatten()
        .filter_map(|run| Some((run["run_id"].as_str()?, run["lifecycle"].as_str()?)))
        .collect();
    let started: BTreeSet<String> = (0..RUNS).map(|index| format!("run-{index}")).collect();
    assert_eq!(
        (
            answered,
            batch["method"].clone(),
            events.len(),
            events.first().map(|resync| resync["event"].clone()),
            runs.iter()
                .map(|(run_id, _)| (*run_id).to_owned())
                .collect::<BTreeSet<_>>(),
            runs.iter().all(|(_, lifecycle)| *lifecycle == "running"),
            exited,
        ),
        (
            RUNS,
            json!("$/events"),
            1,
            Some(json!("events.resync")),
            started,
            true,
            true,
        ),
        "{batch:#?} {rest:#?}"
    );
}
