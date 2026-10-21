//! A crawl's admission on Foxhound's executable over stdio (spec #213's
//! Seam 1; ADR 0094: the per-domain queues are Foxhound's): the leases in
//! flight at once, one per domain key per turn under the global cap, keys
//! under the full Public Suffix List or a crawl's own `crawl.domain_keys`,
//! priority within a domain only, the queues journaled in the run file and
//! released once drained, and an Operator policy cap on the rate cap.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use std::collections::BTreeSet;
use std::time::Duration;

use goodfetch_test_support::TestEnv;
use rusqlite::Connection;
use serde_json::{Value, json};

use crate::crawl::Attached;

/// How long the engine must send nothing more for a turn to be whole.
const QUIET: Duration = Duration::from_millis(300);

fn seed(url: &str, priority: i64) -> Value {
    json!({ "url": url, "label": "page", "priority": priority })
}

fn run_file(env: &TestEnv, run_id: &str) -> Connection {
    Connection::open(
        env.home()
            .join("0.0")
            .join("runs")
            .join(run_id)
            .join("run.sqlite"),
    )
    .unwrap()
}

/// Crawls `seeds` under `settings` and answers, in turn, each set of
/// requests the engine keeps in flight at once: it holds every request
/// until the engine sends no more, then answers them all. Returns each
/// set's URLs and the Run report.
fn in_flight_at_once(
    env: &TestEnv,
    seeds: &[Value],
    settings: &Value,
) -> (Vec<BTreeSet<String>>, Value) {
    let mut engine = Attached::start(env);
    engine.send(&json!({
        "jsonrpc": "2.0", "id": "start", "method": "crawl.start",
        "params": { "run_id": "fair", "seeds": seeds, "settings": settings },
    }));
    let mut turns = Vec::new();
    let mut held = Vec::new();
    let report = loop {
        if !held.is_empty() && engine.is_quiet_for(QUIET) {
            turns.push(
                held.iter()
                    .map(|request: &Value| {
                        request["params"]["crawl_request"]["url"]
                            .as_str()
                            .unwrap()
                            .to_owned()
                    })
                    .collect(),
            );
            for request in held.drain(..) {
                engine.send(
                    &json!({ "jsonrpc": "2.0", "id": request["id"], "result": { "records": [] } }),
                );
            }
            continue;
        }
        let message = engine.next().unwrap();
        if message["method"] == "crawl_request.handle" {
            held.push(message);
        } else if message["id"] == "start" {
            engine.send(&json!({ "jsonrpc": "2.0", "id": "finish", "method": "run.finish", "params": { "run_id": "fair" } }));
        } else if message["id"] == "finish" {
            break message;
        }
    };
    assert!(engine.finish());
    (turns, report)
}

fn urls(urls: &[&str]) -> BTreeSet<String> {
    urls.iter().map(|url| (*url).to_owned()).collect()
}

/// Covers: story:3-47, story:3-48, story:3-49, story:3-50, operation:crawl.start
#[test]
fn each_turn_leases_one_request_per_domain_key_under_the_global_cap() {
    let env = TestEnv::new();
    let seeds = [
        seed("https://heavy.test/1", 0),
        seed("https://www.heavy.test/2", 0),
        seed("https://shop.heavy.test/3", 0),
        seed("https://heavy.test/4", 0),
        seed("https://heavy.test/urgent", 5),
        seed("https://alice.github.io/1", 0),
        seed("https://bob.github.io/1", 0),
    ];

    let (turns, report) = in_flight_at_once(
        &env,
        &seeds,
        &json!({ "crawl": { "handler": { "concurrency": 3 } } }),
    );

    let file = run_file(&env, "fair");
    let keys: Vec<String> = file
        .prepare("SELECT DISTINCT domain_key FROM work_items ORDER BY domain_key")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    let journaled: i64 = file
        .query_row("SELECT count(*) FROM domains", [], |row| row.get(0))
        .unwrap();
    assert_eq!(
        (turns, &report["result"]["lifecycle"], keys, journaled),
        (
            vec![
                urls(&[
                    "https://heavy.test/urgent",
                    "https://alice.github.io/1",
                    "https://bob.github.io/1",
                ]),
                urls(&[
                    "https://heavy.test/1",
                    "https://www.heavy.test/2",
                    "https://shop.heavy.test/3",
                ]),
                urls(&["https://heavy.test/4"]),
            ],
            &json!("finished"),
            vec![
                "alice.github.io".to_owned(),
                "bob.github.io".to_owned(),
                "heavy.test".to_owned(),
            ],
            0,
        )
    );
}

/// Covers: story:3-48, setting:crawl.domain_keys
#[test]
fn a_crawls_own_domain_keys_put_two_sites_in_one_queue() {
    let env = TestEnv::new();
    let seeds = [
        seed("https://alice.github.io/1", 0),
        seed("https://bob.github.io/1", 0),
        seed("https://other.test/1", 0),
    ];

    let (turns, _) = in_flight_at_once(
        &env,
        &seeds,
        &json!({
            "crawl": { "handler": { "concurrency": 2 }, "domain_keys": { "github.io": "github" } },
        }),
    );

    assert_eq!(
        turns.first(),
        Some(&urls(&[
            "https://alice.github.io/1",
            "https://other.test/1"
        ]))
    );
}

/// An Operator policy's cap bounds a crawl's rate even though no layer set
/// one: at one request a minute, the second waits though the handler
/// concurrency has room.
///
/// Covers: story:3-51, setting:admission.max_requests_per_minute, operation:run.abort
#[test]
fn an_operator_policy_cap_bounds_the_rate_of_a_crawl_that_set_none() {
    let env = TestEnv::new();
    let folder = env.home().join("0.0");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(
        folder.join("operator-policy.toml"),
        "version = \"0.1\"\n\n[cap.admission]\nmax_requests_per_minute = 1\n",
    )
    .unwrap();
    let mut engine = Attached::start(&env);
    engine.send(&json!({
        "jsonrpc": "2.0", "id": "start", "method": "crawl.start",
        "params": { "run_id": "paced", "seeds": [seed("https://a.test/1", 0), seed("https://b.test/1", 0)] },
    }));
    let first = loop {
        let message = engine.next().unwrap();
        if message["method"] == "crawl_request.handle" {
            break message;
        }
    };
    engine.send(&json!({ "jsonrpc": "2.0", "id": first["id"], "result": { "records": [] } }));

    let held = engine.is_quiet_for(Duration::from_secs(1));
    let report = engine.call(
        "abort",
        "run.abort",
        &json!({ "run_id": "paced" }),
        &mut |_| None,
    );
    assert!(engine.finish());

    let statuses: Vec<(String, String)> = run_file(&env, "paced")
        .prepare("SELECT url, status FROM work_items ORDER BY position")
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(
        (held, &report["result"]["lifecycle"], statuses),
        (
            true,
            &json!("stopped"),
            vec![
                ("https://a.test/1".to_owned(), "succeeded".to_owned()),
                ("https://b.test/1".to_owned(), "pending".to_owned()),
            ]
        )
    );
}
