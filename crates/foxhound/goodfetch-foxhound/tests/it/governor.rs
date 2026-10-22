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
