//! The Resource governor on Foxhound's executable over stdio (spec #213's
//! Seam 1; ADR 0040): two crawls of one engine take each attempt's HTTP
//! admission from its governor, whose fixed cap holds both, and what one
//! gives back goes to the other in turn.

use std::time::Duration;

use goodfetch_test_support::TestEnv;
use serde_json::{Value, json};

use crate::crawl::Attached;

/// How long the engine must send nothing more for a turn to be whole.
const QUIET: Duration = Duration::from_millis(500);

/// The user configuration of a home whose engines keep `in_flight` HTTP
/// admissions at once whatever the machine: each engine is held at
/// pressure (`governor.pressure` at 1 %), where its governor admits only
/// its floor, and the floor and ceiling are fixed at `in_flight`. A test
/// that kills an engine at one of its first `in_flight` handler requests,
/// none answered, needs them all in flight: a governor sized by a loaded
/// machine's memory once admitted one, and the kill waited for a second
/// request that never came.
pub(crate) fn in_flight_whatever_the_machine(in_flight: u32) -> String {
    format!(
        "governor.pressure = 1\ngovernor.http.floor = {in_flight}\ngovernor.http.ceiling = {in_flight}\n"
    )
}

fn start(engine: &mut Attached, run_id: &str, hosts: &[&str]) {
    let seeds: Vec<Value> = hosts
        .iter()
        .map(|host| json!({ "url": format!("https://{host}/"), "label": "page" }))
        .collect();
    engine.send(&json!({
        "jsonrpc": "2.0", "id": format!("start-{run_id}"), "method": "crawl.start",
        "params": {
            "run_id": run_id,
            "seeds": seeds,
            "settings": { "crawl": { "handler": { "concurrency": 3 } } },
        },
    }));
}

/// Every handler request the engine sends until it is quiet, held.
fn turn(engine: &mut Attached) -> Vec<Value> {
    let mut held = Vec::new();
    while !engine.is_quiet_for(QUIET) {
        let message = engine.next().unwrap();
        if message["method"] == "crawl_request.handle" {
            held.push(message);
        }
    }
    held
}

/// The Runs `held`'s requests are of, in order.
fn runs(held: &[Value]) -> Vec<String> {
    let mut runs: Vec<String> = held
        .iter()
        .map(|request| request["params"]["run_id"].as_str().unwrap().to_owned())
        .collect();
    runs.sort();
    runs
}

fn answer(engine: &mut Attached, held: Vec<Value>) {
    for request in held {
        engine.send(&json!({ "jsonrpc": "2.0", "id": request["id"], "result": { "records": [] } }));
    }
}

/// With `governor.http` fixed at 2, the crawl that started first takes
/// both admissions though it could run 3 handlers; the second waits; once
/// the first gives them back, they go to each crawl in turn, and once the
/// first is done, both to the second.
///
/// Covers: story:3-55
#[test]
fn two_crawls_take_the_governors_http_admissions_in_turn() {
    let env = TestEnv::new();
    env.user_configuration("governor.http.floor = 2\ngovernor.http.ceiling = 2\n");
    let mut engine = Attached::start(&env);

    start(&mut engine, "first", &["a.test", "b.test", "c.test"]);
    let alone = turn(&mut engine);
    start(&mut engine, "second", &["d.test", "e.test", "f.test"]);
    let waiting = turn(&mut engine);
    let mut turns = vec![runs(&alone), runs(&waiting)];
    answer(&mut engine, alone);
    for _ in 0..3 {
        let held = turn(&mut engine);
        turns.push(runs(&held));
        answer(&mut engine, held);
    }
    for run_id in ["first", "second"] {
        engine.send(&json!({ "jsonrpc": "2.0", "id": run_id, "method": "run.finish", "params": { "run_id": run_id } }));
    }
    assert!(engine.finish());

    assert_eq!(
        turns,
        [
            vec!["first".to_owned(), "first".to_owned()],
            Vec::new(),
            vec!["first".to_owned(), "second".to_owned()],
            vec!["second".to_owned(), "second".to_owned()],
            Vec::new(),
        ]
    );
}

/// Follows the progress of `run_id` from its first value to the first that
/// holds a wait group of `reason`, and answers that group.
fn wait_group(engine: &mut Attached, run_id: &str, reason: &str) -> Value {
    let mut params = json!({ "run_id": run_id });
    loop {
        engine.send(
            &json!({ "jsonrpc": "2.0", "id": "progress", "method": "run.progress", "params": params }),
        );
        let progress = loop {
            let message = engine.next().unwrap();
            if message["id"] == "progress" {
                break message["result"].clone();
            }
        };
        if let Some(group) = progress["waiting"]
            .as_array()
            .and_then(|groups| groups.iter().find(|group| group["reason"] == reason))
        {
            return group.clone();
        }
        assert_eq!(progress["last"], false, "the crawl ended: {progress}");
        params["after"] = progress["position"].clone();
    }
}

/// A crawl whose governor admits nothing more, `governor.http` fixed at 1
/// with one request in flight, says that its due request waits under
/// `governor`.
///
/// Covers: story:3-162, operation:run.progress
#[test]
fn a_crawl_the_governor_admits_nothing_more_for_says_its_due_requests_wait_under_governor() {
    let env = TestEnv::new();
    env.user_configuration("governor.http.floor = 1\ngovernor.http.ceiling = 1\n");
    let mut engine = Attached::start(&env);

    start(&mut engine, "held", &["a.test", "b.test"]);
    let held = turn(&mut engine);
    let group = wait_group(&mut engine, "held", "governor");
    answer(&mut engine, held);
    let _ = turn(&mut engine);
    assert!(engine.finish());

    assert_eq!(
        group,
        json!({ "reason": "governor", "count": 1, "domains": [{ "domain": "b.test" }] })
    );
}
