//! The event stream on Collie's executable over stdio (spec #213's Seam 1;
//! ADR 0094: axis changes are Collie's): a subscriber that grants no credits
//! holds back the Runs whose axis changes its full queue cannot take, and
//! once it grants them it reads every one, in the order of their positions.
//! Credits are granted, never timed (ADR 0064).
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use std::collections::BTreeSet;
use std::time::Duration;

use goodfetch_test_support::TestEnv;
use serde_json::{Value, json};

use crate::attached::{Attached, hello};

/// How many lossless events a subscription's queue holds by default.
const QUEUE: usize = 64;

/// How long the test waits to see that nothing more is written.
const QUIET: Duration = Duration::from_millis(500);

/// Covers: story:3-8, operation:events.subscribe, event:run.axes_changed
#[test]
fn a_subscriber_that_grants_no_credits_holds_back_collies_runs_and_then_reads_every_axis_change() {
    const RUNS: usize = QUEUE + 3;
    let env = TestEnv::new();
    let mut engine = Attached::start(&env);
    engine.send(&hello("collie", "0.1"));
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

    let (mut answered, mut events) = (0, Vec::new());
    let read = |engine: &mut Attached, answered: &mut usize, events: &mut Vec<Value>| {
        let message = engine.next().unwrap();
        if message["method"] == "$/events" && message["params"]["id"] == "events" {
            events.extend(message["params"]["events"].as_array().unwrap().clone());
        } else if message["result"]["lifecycle"] == "running" {
            *answered += 1;
        } else {
            panic!("an unexpected message: {message}");
        }
    };
    while answered < QUEUE {
        read(&mut engine, &mut answered, &mut events);
    }
    let held = engine.is_quiet_for(QUIET);
    engine.send(&json!({
        "jsonrpc": "2.0", "method": "$/credit",
        "params": { "id": "events", "credits": RUNS },
    }));
    while answered < RUNS || events.len() < RUNS {
        read(&mut engine, &mut answered, &mut events);
    }
    let (rest, exited) = engine.finish();

    let positions: Vec<u64> = events
        .iter()
        .map(|event| event["position"].as_u64().unwrap())
        .collect();
    let runs: BTreeSet<&str> = events
        .iter()
        .map(|event| event["data"]["run_id"].as_str().unwrap())
        .collect();
    assert_eq!(
        (
            held,
            events.len(),
            runs.len(),
            positions.windows(2).all(|pair| pair[0] < pair[1]),
            events
                .iter()
                .all(|event| event["event"] == "run.axes_changed"
                    && event["data"]["lifecycle"] == "running"),
            exited,
        ),
        (true, RUNS, RUNS, true, true, true),
        "{rest:#?}"
    );
}
