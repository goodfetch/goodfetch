//! A crawl's redundant requests on Foxhound's executable against the
//! Proving ground (spec #213's Seam 1; ADRs 0040, 0086 and 0094: hedged
//! requests are Foxhound's, with the HTTP fetch tiers), judged by what the
//! Proving ground saw and the Run's Provenance, never by elapsed time (ADR
//! 0064): a slow GET is hedged on a new connection once its route's
//! latency is known, a POST never is; identical GET requests in flight
//! from one Session wait for one response, never across Sessions or runs;
//! and a crawl that switches hedges on while a domain admits one request
//! at a time is refused, since the second copy could never start.
//!
//! The `pacing` family's latency follows the Proving ground's time step:
//! about 10 ms at step 0, about 1.9 s at step 95.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use std::collections::BTreeSet;
use std::path::PathBuf;

use goodfetch_protocol::home::RUN_TRACE_PROVENANCE;
use goodfetch_test_client::Attached;
use goodfetch_test_client::HomeLayout;
use goodfetch_test_support::{ProvingGround, ReceivedRequests, TestEnv};
use serde_json::{Value, json};

use crate::attached;

/// The Proving ground's seed.
const SEED: u64 = 238;

/// A time step whose answers the `pacing` family holds about two seconds.
const SLOW: u64 = 95;

/// The governor admits two HTTP fetches at once whatever the host's load,
/// so two copies, or two identical requests, are in flight together (ADR
/// 0040: a governor above normal admits none past its floor).
const TWO_FETCHES: &str = "governor.http.floor = 2\ngovernor.http.ceiling = 2\n";

/// The settings of a crawl fetched over HTTP from the Proving ground, two
/// requests at a time on its domain with no delay, beside `more`.
fn fetched_over_http(proving_ground: &ProvingGround, more: &Value) -> Value {
    let mut settings = json!({
        "fetch": { "tier": "http" },
        "network": { "resolve": { "pacing.test": [proving_ground.address().ip().to_string()] } },
        "egress": { "private_ranges": ["pacing.test"] },
        "politeness": { "domain": { "concurrency": 2, "delay": 0 } },
    });
    merge(&mut settings, more);
    settings
}

fn merge(into: &mut Value, more: &Value) {
    match (into, more) {
        (Value::Object(into), Value::Object(more)) => {
            for (key, value) in more {
                merge(into.entry(key.clone()).or_insert(Value::Null), value);
            }
        }
        (into, more) => *into = more.clone(),
    }
}

/// A crawl to start: its Run ID, seeds and settings.
struct Crawl {
    run_id: &'static str,
    seeds: Vec<Value>,
    settings: Value,
}

/// Starts each of `crawls` on `engine`, answers every handler request
/// with no Record, calling `before_answer` with its URL first, and lets
/// each Run finish; answers each `run.finish` reply, in `crawls`' order.
fn crawl_together(
    engine: &mut Attached,
    crawls: &[Crawl],
    before_answer: &mut impl FnMut(&str),
) -> Vec<Value> {
    for crawl in crawls {
        engine.send(&json!({
            "jsonrpc": "2.0", "id": format!("start {}", crawl.run_id), "method": "crawl.start",
            "params": { "run_id": crawl.run_id, "seeds": crawl.seeds, "settings": crawl.settings },
        }));
    }
    let mut finished = vec![Value::Null; crawls.len()];
    while finished.iter().any(Value::is_null) {
        let message = engine
            .next()
            .expect("the engine closed before the Runs ended");
        let id = message["id"].as_str().unwrap_or_default().to_owned();
        if message["method"] == "crawl_request.handle" {
            before_answer(message["params"]["crawl_request"]["url"].as_str().unwrap());
            engine.send(
                &json!({ "jsonrpc": "2.0", "id": message["id"], "result": { "records": [] } }),
            );
        } else if let Some(run_id) = id.strip_prefix("start ") {
            assert!(message.get("result").is_some(), "{message}");
            engine.send(&json!({
                "jsonrpc": "2.0", "id": format!("finish {run_id}"), "method": "run.finish",
                "params": { "run_id": run_id },
            }));
        } else if let Some(run_id) = id.strip_prefix("finish ") {
            let place = crawls
                .iter()
                .position(|crawl| crawl.run_id == run_id)
                .unwrap();
            finished[place] = message;
        }
    }
    finished
}

/// The requests the Proving ground saw for `target` on `pacing.test`,
/// each by its connection's number.
fn connections_for(proving_ground: &ProvingGround, target: &str) -> Vec<u64> {
    let observations = proving_ground.observations().unwrap();
    ReceivedRequests::of(&observations)
        .of_the_case()
        .filter(|request| request["host"] == "pacing.test" && request["target"] == target)
        .map(|request| request["connection"].as_u64().unwrap())
        .collect()
}

/// The Provenance entries of the Run `run_id` whose action is `action`,
/// none when it wrote no Provenance log.
#[expect(
    clippy::disallowed_methods,
    reason = "the test reads the Run's Provenance log from outside the engine, on no async worker"
)]
fn provenance(env: &TestEnv, run_id: &str, action: &str) -> Vec<Value> {
    let file: PathBuf = HomeLayout::of(env.home()).run_trace(run_id, RUN_TRACE_PROVENANCE);
    std::fs::read_to_string(file)
        .unwrap_or_default()
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .filter(|entry| entry["action"] == action)
        .collect()
}

/// A crawl that switches hedges on under a `politeness.domain.concurrency`
/// of 1, ADR 0086's default, is refused before its Run begins,
/// naming the key and the layer that switched them on (ADR 0086).
///
/// Covers: story:3-63, setting:fetch.hedge.enabled, error:setting_not_applicable
#[test]
fn a_crawl_that_hedges_while_a_domain_admits_one_request_at_a_time_is_refused() {
    let env = TestEnv::new();
    let mut engine = attached(&env);

    let refused = engine.call(
        "start",
        "crawl.start",
        &json!({
            "run_id": "hedged",
            "seeds": [{ "url": "http://pacing.test/", "label": "page" }],
            "settings": {
                "fetch": { "tier": "http", "hedge": { "enabled": true } },
                "politeness": { "domain": { "concurrency": 1 } },
            },
        }),
        &mut |_| None,
    );
    assert!(engine.finished());

    assert_eq!(
        (
            &refused["error"]["data"]["code"],
            &refused["error"]["data"]["key"],
            &refused["error"]["data"]["layer"],
        ),
        (
            &json!("setting_not_applicable"),
            &json!("fetch.hedge.enabled"),
            &json!("run"),
        ),
        "{refused}"
    );
}

/// Twenty quick answers give the route its latency; then the site slows
/// down. The slow GET is sent a second time, on a new connection, the
/// hedge counted in Usage, and Provenance records it; the slow POST after
/// it is sent once. The quick URLs serve one page, so each declares its
/// own key: the Frontier would otherwise learn that `quick` has no effect
/// and end the rest as duplicates.
///
/// Covers: story:3-63, setting:fetch.hedge.enabled, setting:fetch.hedge.max_percent, operation:crawl.start
#[test]
fn a_slow_get_is_hedged_on_a_new_connection_and_a_post_never_is() {
    let env = TestEnv::new();
    env.user_configuration(TWO_FETCHES);
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let mut engine = attached(&env);
    let quick: Vec<Value> = (1..=20)
        .map(|number| json!({ "url": proving_ground.url("pacing.test", &format!("/articles/1?quick={number}")), "label": "article", "key": format!("quick-{number}") }))
        .collect();
    let last_quick = proving_ground.url("pacing.test", "/articles/1?quick=20");
    let slow_get = proving_ground.url("pacing.test", "/articles/2?slow=get");
    let slow_post = proving_ground.url("pacing.test", "/articles/3?slow=post");
    let mut seeds = quick;
    seeds.push(json!({ "url": slow_get, "label": "article" }));
    seeds.push(
        json!({ "url": slow_post, "label": "article", "method": "POST", "body": "cmVhZA==" }),
    );

    let finished = crawl_together(
        &mut engine,
        &[Crawl {
            run_id: "hedged",
            seeds,
            settings: fetched_over_http(
                &proving_ground,
                &json!({
                    "fetch": { "hedge": { "enabled": true, "max_percent": 100 } },
                    "crawl": { "handler": { "concurrency": 1 } },
                }),
            ),
        }],
        &mut |url| {
            if url == last_quick {
                proving_ground.set_time_step(SLOW).unwrap();
            }
        },
    );
    assert!(engine.finished());

    let get = connections_for(&proving_ground, "/articles/2?slow=get");
    let hedges: Vec<Value> = provenance(&env, "hedged", "hedged")
        .into_iter()
        .map(|entry| entry["url"].clone())
        .collect();
    assert_eq!(
        (
            &finished[0]["result"]["lifecycle"],
            get.len(),
            get.iter().collect::<BTreeSet<_>>().len(),
            connections_for(&proving_ground, "/articles/3?slow=post").len(),
            hedges,
            &finished[0]["result"]["usage"]["requests"],
        ),
        // 22 crawl requests and the hedge count in Usage.
        (
            &json!("finished"),
            2,
            2,
            1,
            vec![json!(slow_get)],
            &json!(23)
        ),
    );
}

/// Two crawl requests of one URL (their keys tell them apart) in flight
/// at once from one Session reach the site once, and Provenance records
/// the response the second shared.
///
/// Covers: story:3-64, setting:fetch.coalesce
#[test]
fn identical_gets_in_flight_from_one_session_reach_the_site_once() {
    let env = TestEnv::new();
    env.user_configuration(TWO_FETCHES);
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    proving_ground.set_time_step(SLOW).unwrap();
    let mut engine = attached(&env);
    let url = proving_ground.url("pacing.test", "/articles/4");

    let finished = crawl_together(
        &mut engine,
        &[Crawl {
            run_id: "shared",
            seeds: vec![
                json!({ "url": url, "label": "article", "key": "first" }),
                json!({ "url": url, "label": "article", "key": "second" }),
            ],
            settings: fetched_over_http(
                &proving_ground,
                &json!({
                    "crawl": { "handler": { "concurrency": 2 } },
                    "session": { "pool": { "max_size": 1 } },
                }),
            ),
        }],
        &mut |_| {},
    );
    assert!(engine.finished());

    let shared = provenance(&env, "shared", "response_shared");
    assert_eq!(
        (
            &finished[0]["result"]["lifecycle"],
            connections_for(&proving_ground, "/articles/4").len(),
            shared
                .iter()
                .map(|entry| entry["url"].clone())
                .collect::<Vec<_>>(),
        ),
        (&json!("finished"), 1, vec![json!(url)]),
    );
}

/// Identical crawl requests in flight at once from two Sessions of one
/// crawl, or from two crawls, each reach the site: cookies differ between
/// Sessions, and a run shares nothing with another (ADR 0040).
///
/// Covers: story:3-64, setting:fetch.coalesce
#[test]
fn identical_gets_from_two_sessions_or_two_runs_each_reach_the_site() {
    let env = TestEnv::new();
    // The four requests are in flight together: the governor admits four
    // HTTP fetches, and the domain, whose count the three runs share, four
    // requests.
    env.user_configuration("governor.http.floor = 4\ngovernor.http.ceiling = 4\n");
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    proving_ground.set_time_step(SLOW).unwrap();
    let mut engine = attached(&env);
    let two_sessions = proving_ground.url("pacing.test", "/articles/5");
    let two_runs = proving_ground.url("pacing.test", "/articles/6");
    let four_at_a_time = json!({ "politeness": { "domain": { "concurrency": 4 } } });
    let one_session = json!({
        "politeness": { "domain": { "concurrency": 4 } },
        "session": { "pool": { "max_size": 1 } },
    });

    let finished = crawl_together(
        &mut engine,
        &[
            Crawl {
                run_id: "sessions",
                seeds: vec![
                    json!({ "url": two_sessions, "label": "article", "key": "first" }),
                    json!({ "url": two_sessions, "label": "article", "key": "second" }),
                ],
                settings: fetched_over_http(
                    &proving_ground,
                    &json!({
                        "crawl": { "handler": { "concurrency": 2 } },
                        "politeness": four_at_a_time["politeness"],
                        "session": { "pool": { "max_size": 2 } },
                    }),
                ),
            },
            Crawl {
                run_id: "first-run",
                seeds: vec![json!({ "url": two_runs, "label": "article" })],
                settings: fetched_over_http(&proving_ground, &one_session),
            },
            Crawl {
                run_id: "second-run",
                seeds: vec![json!({ "url": two_runs, "label": "article" })],
                settings: fetched_over_http(&proving_ground, &one_session),
            },
        ],
        &mut |_| {},
    );
    assert!(engine.finished());

    let shared: usize = ["sessions", "first-run", "second-run"]
        .iter()
        .map(|run_id| provenance(&env, run_id, "response_shared").len())
        .sum();
    assert_eq!(
        (
            finished
                .iter()
                .map(|reply| reply["result"]["lifecycle"].clone())
                .collect::<Vec<_>>(),
            connections_for(&proving_ground, "/articles/5").len(),
            connections_for(&proving_ground, "/articles/6").len(),
            shared,
        ),
        (vec![json!("finished"); 3], 2, 2, 0),
    );
}
