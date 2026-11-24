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
use rstest::rstest;
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
/// with what `handler` makes of its params, calling `engine` itself when
/// it needs, and lets each Run finish; answers each `run.finish` reply,
/// in `crawls`' order.
fn crawl_together(
    engine: &mut Attached,
    crawls: &[Crawl],
    handler: &mut impl FnMut(&mut Attached, &Value) -> Value,
) -> Vec<Value> {
    for crawl in crawls {
        engine.send(&json!({
            "jsonrpc": "2.0", "id": format!("start {}", crawl.run_id), "method": "crawl.start",
            "params": { "run_id": crawl.run_id, "seeds": crawl.seeds, "settings": crawl.settings, "labels": { "article": {} } },
        }));
    }
    let mut finished = vec![Value::Null; crawls.len()];
    while finished.iter().any(Value::is_null) {
        let message = engine
            .next()
            .expect("the engine closed before the Runs ended");
        let id = message["id"].as_str().unwrap_or_default().to_owned();
        if message["method"] == "crawl_request.handle" {
            let mut reply = handler(engine, &message["params"]);
            reply["jsonrpc"] = json!("2.0");
            reply["id"] = message["id"].clone();
            engine.send(&reply);
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

/// A handler's answer with no Record.
fn no_record() -> Value {
    json!({ "result": { "records": [] } })
}

/// A handler's answer that asks for its attempt to run again.
fn retry() -> Value {
    json!({ "error": {
        "code": -32003, "message": "try again",
        "data": { "code": "retry_requested", "retryable": true },
    } })
}

/// Declares the Effect `order` through the attempt `handle` gave, and
/// closes it as returned before anything was sent for it: from then on,
/// its work item declares an Effect (ADR 0106).
fn declare_an_effect(engine: &mut Attached, handle: &Value) {
    let opened = engine.call(
        "open",
        "effect.open",
        &json!({
            "run_id": handle["run_id"],
            "key": "order",
            "attempt": {
                "crawl_request_id": handle["crawl_request"]["id"],
                "lease_epoch": handle["lease_epoch"],
            },
        }),
        &mut |_| None,
    );
    assert_eq!(opened["result"]["answer"], "perform", "{opened}");
    let closed = engine.call(
        "close",
        "effect.close",
        &json!({
            "run_id": handle["run_id"],
            "entry": opened["result"]["entry"],
            "outcome": { "ended": "returned" },
        }),
        &mut |_| None,
    );
    assert_eq!(closed["result"]["judged"], "done", "{closed}");
}

/// Adds `seed` to the crawl `run_id` from outside any attempt, so it stays
/// whatever the attempt that adds it answers.
fn add_seed(engine: &mut Attached, run_id: &Value, seed: &Value) {
    let added = engine.call(
        "seed",
        "run.add_seed",
        &json!({ "run_id": run_id, "seed": seed }),
        &mut |_| None,
    );
    assert!(added.get("result").is_some(), "{added}");
}

/// The requests the Proving ground saw for `target` on `pacing.test`,
/// each by its connection's number.
fn connections_for(proving_ground: &ProvingGround, target: &str) -> Vec<u64> {
    let observations = proving_ground.observations().unwrap();
    ReceivedRequests::of(&observations)
        .of_the_case()
        .filter(|request| request.host == "pacing.test" && request.target == target)
        .map(|request| request.connection)
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
            "labels": { "page": {} },
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
    let mut engine = crate::site::engine_on(&env, &proving_ground);
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
        &mut |_, handle| {
            if handle["crawl_request"]["url"] == last_quick {
                proving_ground.set_time_step(SLOW).unwrap();
            }
            no_record()
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

/// A work item that declared an Effect is never hedged (ADRs 0040 and
/// 0106). Its first attempt, quick and alone, declares one, adds twenty
/// quick seeds that go first and a slow GET of its own that declares none,
/// then asks to run again; the quick answers give the route its latency,
/// then the site slows down. Its retry sends its slow GET once, while the
/// slow GET of the work item with no Effect after it is hedged.
///
/// Covers: story:3-63, setting:fetch.hedge.enabled, operation:effect.open
#[test]
fn a_work_item_that_declared_an_effect_sends_a_slow_get_once() {
    let env = TestEnv::new();
    env.user_configuration(TWO_FETCHES);
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let mut engine = crate::site::engine_on(&env, &proving_ground);
    let declares = proving_ground.url("pacing.test", "/articles/7?slow=effect");
    let declares_none = proving_ground.url("pacing.test", "/articles/8?slow=read");
    let last_quick = proving_ground.url("pacing.test", "/articles/1?quick=20");
    let mut handled = Vec::new();

    let finished = crawl_together(
        &mut engine,
        &[Crawl {
            run_id: "hedged",
            seeds: vec![json!({ "url": declares, "label": "article" })],
            settings: fetched_over_http(
                &proving_ground,
                &json!({
                    "fetch": { "hedge": { "enabled": true, "max_percent": 100 } },
                    "crawl": { "handler": { "concurrency": 1 } },
                }),
            ),
        }],
        &mut |engine, handle| {
            let url = &handle["crawl_request"]["url"];
            handled.push(url.clone());
            if *url == declares && handle["attempt"] == 1 {
                declare_an_effect(engine, handle);
                for number in 1..=20 {
                    let quick =
                        proving_ground.url("pacing.test", &format!("/articles/1?quick={number}"));
                    add_seed(
                        engine,
                        &handle["run_id"],
                        &json!({ "url": quick, "label": "article", "key": format!("quick-{number}"), "priority": 1 }),
                    );
                }
                add_seed(
                    engine,
                    &handle["run_id"],
                    &json!({ "url": declares_none, "label": "article" }),
                );
                return retry();
            }
            if *url == last_quick {
                proving_ground.set_time_step(SLOW).unwrap();
            }
            no_record()
        },
    );
    assert!(engine.finished());

    let hedges: Vec<Value> = provenance(&env, "hedged", "hedged")
        .into_iter()
        .map(|entry| entry["url"].clone())
        .collect();
    let slowed = handled.iter().position(|url| *url == last_quick);
    assert_eq!(
        (
            &finished[0]["result"]["lifecycle"],
            // The retry ran once the site was slow.
            handled.iter().rposition(|url| *url == declares) > slowed,
            connections_for(&proving_ground, "/articles/7?slow=effect").len(),
            connections_for(&proving_ground, "/articles/8?slow=read").len(),
            hedges,
        ),
        (&json!("finished"), true, 2, 2, vec![json!(declares_none)]),
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
    let mut engine = crate::site::engine_on(&env, &proving_ground);
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
        &mut |_, _| no_record(),
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

/// Identical GETs in flight from one Session each reach the site when one
/// comes from a work item that declared an Effect, and share one response
/// when neither did (ADRs 0040 and 0106). The seed's first attempt, alone,
/// declares an Effect or not, slows the site down, adds a seed of the same
/// URL and asks to run again: its retry and the added seed are then in
/// flight together.
///
/// Covers: story:3-64, setting:fetch.coalesce
#[rstest]
/// Covers: operation:effect.open
#[case::from_a_work_item_that_declared_an_effect(true, 3, 0)]
#[case::from_work_items_that_declared_none(false, 2, 1)]
fn identical_gets_share_a_response_only_when_no_effect_was_declared(
    #[case] declares: bool,
    #[case] requests: usize,
    #[case] shared: usize,
) {
    let env = TestEnv::new();
    env.user_configuration(TWO_FETCHES);
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let mut engine = crate::site::engine_on(&env, &proving_ground);
    let url = proving_ground.url("pacing.test", "/articles/9");
    let mut first = true;

    let finished = crawl_together(
        &mut engine,
        &[Crawl {
            run_id: "shared",
            seeds: vec![json!({ "url": url, "label": "article", "key": "retried" })],
            settings: fetched_over_http(
                &proving_ground,
                &json!({
                    "crawl": { "handler": { "concurrency": 2 } },
                    "session": { "pool": { "max_size": 1 } },
                }),
            ),
        }],
        &mut |engine, handle| {
            if !first {
                return no_record();
            }
            first = false;
            if declares {
                declare_an_effect(engine, handle);
            }
            proving_ground.set_time_step(SLOW).unwrap();
            add_seed(
                engine,
                &handle["run_id"],
                &json!({ "url": url, "label": "article", "key": "added" }),
            );
            retry()
        },
    );
    assert!(engine.finished());

    assert_eq!(
        (
            &finished[0]["result"]["lifecycle"],
            connections_for(&proving_ground, "/articles/9").len(),
            provenance(&env, "shared", "response_shared").len(),
        ),
        (&json!("finished"), requests, shared),
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
    let mut engine = crate::site::engine_on(&env, &proving_ground);
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
        &mut |_, _| no_record(),
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
