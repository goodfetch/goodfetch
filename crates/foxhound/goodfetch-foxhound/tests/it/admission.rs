//! A crawl's admission on Foxhound's executable over stdio (spec #213's
//! Seam 1; ADR 0094: the per-domain queues are Foxhound's): the leases in
//! flight at once, one per domain key per turn under the global cap, keys
//! under the full Public Suffix List or a crawl's own `crawl.domain_keys`,
//! priority within a domain only, the queues journaled in the run file and
//! released once drained, an Operator policy cap on the rate cap, and
//! `crawl.domain_order`'s value, explained, refused when invalid and
//! locked by an Operator policy (#2197).
#![expect(
    clippy::disallowed_methods,
    reason = "a test writes the Operator policy itself; its SQL statements are its own, on no Shared path (ADR 0140)"
)]
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use std::collections::BTreeSet;
use std::time::Duration;

use goodfetch_test_client::HomeLayout;
use goodfetch_test_support::{TestEnv, cli};
use rstest::rstest;
use serde_json::{Value, json};

use crate::run_file;
use crate::site::Site;

/// The Resource governor's HTTP admissions, fixed at every crawl's handler
/// concurrency here or above: under pressure the governor lowers a kind's
/// target towards its floor (ADR 0040), and a turn would then wait for
/// room the crawl's settings never named.
const GOVERNOR: &str = "governor.http.floor = 3\ngovernor.http.ceiling = 3\n";

fn seed(url: &str, priority: i64) -> Value {
    json!({ "url": url, "label": "page", "priority": priority })
}

/// Crawls `seeds` under `settings` and answers, in turn, each set of
/// requests the engine keeps in flight at once: it holds every request
/// until the crawl's progress (`run.progress`) says that the requests held
/// here are all it has in flight and that it admits no more while they
/// are, then answers them all. Returns each set's URLs and the Run report.
/// A quiet stretch would say nothing of the engine's state: a loaded
/// machine outlasts it between two requests of one turn.
fn in_flight_at_once(
    (env, site): (&TestEnv, &Site),
    seeds: &[Value],
    settings: &Value,
) -> (Vec<BTreeSet<String>>, Value) {
    env.user_configuration(GOVERNOR);
    let mut engine = site.engine(env);
    engine.send(&json!({
        "jsonrpc": "2.0", "id": "start", "method": "crawl.start",
        "params": { "run_id": "fair", "seeds": seeds, "settings": settings },
    }));
    let mut turns = Vec::new();
    let mut held: Vec<Value> = Vec::new();
    let (mut started, mut asked) = (false, false);
    // The latest progress read, until the turn it showed is answered.
    let mut progress: Option<Value> = None;
    let report = loop {
        if progress
            .as_ref()
            .is_some_and(|progress| is_whole(progress, &held))
        {
            turns.push(held.iter().map(url_of).collect());
            for request in held.drain(..) {
                engine.send(
                    &json!({ "jsonrpc": "2.0", "id": request["id"], "result": { "records": [] } }),
                );
            }
            progress = None;
        }
        if started && !held.is_empty() && !asked {
            // Past a read that showed no whole turn, the next answers once
            // the crawl changed: it admits more, or a request it sent
            // arrives here meanwhile, which that same read then shows.
            let mut params = json!({ "run_id": "fair" });
            if let Some(read) = &progress {
                params["after"] = read["position"].clone();
            }
            engine.send(&json!({ "jsonrpc": "2.0", "id": "progress", "method": "run.progress", "params": params }));
            asked = true;
        }
        let message = engine.next().unwrap();
        if message["method"] == "crawl_request.handle" {
            held.push(message);
        } else if message["id"] == "start" {
            started = true;
            engine.send(&json!({ "jsonrpc": "2.0", "id": "finish", "method": "run.finish", "params": { "run_id": "fair" } }));
        } else if message["id"] == "progress" {
            asked = false;
            progress = Some(message["result"].clone());
        } else if message["id"] == "finish" {
            break message;
        }
    };
    assert!(engine.finished());
    (turns, report)
}

/// Whether `progress`, one moment of the crawl, shows `held` as a whole
/// turn: the requests in flight then were exactly those held, each with
/// its handler, and the crawl admitted no more while they were, its
/// handlers at their concurrency or nothing pending.
fn is_whole(progress: &Value, held: &[Value]) -> bool {
    let in_flight: BTreeSet<&str> = progress["in_flight"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|request| request["phase"] == "handling")
        .filter_map(|request| request["request_id"].as_str())
        .collect();
    let holding: BTreeSet<&str> = held
        .iter()
        .filter_map(|request| request["params"]["crawl_request"]["id"].as_str())
        .collect();
    let count = |name: &str| progress["requests"][name].as_u64().unwrap_or(0);
    let handling = count("handling");
    !held.is_empty()
        && in_flight == holding
        && count("fetching") == 0
        && handling == u64::try_from(held.len()).unwrap()
        && (Some(handling) == progress["concurrency"]["handler_concurrency"].as_u64()
            || count("pending") == 0)
}

/// The URL of the crawl request `request` asks to handle.
fn url_of(request: &Value) -> String {
    request["params"]["crawl_request"]["url"]
        .as_str()
        .unwrap()
        .to_owned()
}

fn urls(urls: &[&str]) -> BTreeSet<String> {
    urls.iter().map(|url| (*url).to_owned()).collect()
}

/// The heavy domain's requests wait for the small ones, which hold none in
/// flight, so both orders give the turns.
///
/// Covers: story:3-47, story:3-48, story:3-49, story:3-50, operation:crawl.start, setting:crawl.domain_order
#[rstest]
#[case::by_default(json!({ "crawl": { "handler": { "concurrency": 3 } } }))]
#[case::round_robin(json!({ "crawl": { "handler": { "concurrency": 3 }, "domain_order": "round_robin" } }))]
fn each_turn_leases_one_request_per_domain_key_under_the_global_cap(#[case] settings: Value) {
    let env = TestEnv::new();
    let site = Site::start(&env);
    let seeds = [
        seed(&site.url("heavy.test", "/1"), 0),
        seed(&site.url("www.heavy.test", "/2"), 0),
        seed(&site.url("shop.heavy.test", "/3"), 0),
        seed(&site.url("heavy.test", "/4"), 0),
        seed(&site.url("heavy.test", "/urgent"), 5),
        seed(&site.url("alice.github.io", "/1"), 0),
        seed(&site.url("bob.github.io", "/1"), 0),
    ];

    let (turns, report) = in_flight_at_once((&env, &site), &seeds, &settings);

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
                    &site.url("heavy.test", "/urgent"),
                    &site.url("alice.github.io", "/1"),
                    &site.url("bob.github.io", "/1"),
                ]),
                urls(&[
                    &site.url("heavy.test", "/1"),
                    &site.url("www.heavy.test", "/2"),
                    &site.url("shop.heavy.test", "/3"),
                ]),
                urls(&[&site.url("heavy.test", "/4")]),
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
    let site = Site::start(&env);
    let seeds = [
        seed(&site.url("alice.github.io", "/1"), 0),
        seed(&site.url("bob.github.io", "/1"), 0),
        seed(&site.url("other.test", "/1"), 0),
    ];

    let (turns, _) = in_flight_at_once(
        (&env, &site),
        &seeds,
        &json!({
            "crawl": { "handler": { "concurrency": 2 }, "domain_keys": { "github.io": "github" } },
        }),
    );

    assert_eq!(
        turns.first(),
        Some(&urls(&[
            &site.url("alice.github.io", "/1"),
            &site.url("other.test", "/1")
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
    let layout = HomeLayout::of(env.home());
    std::fs::create_dir_all(layout.per_major()).unwrap();
    std::fs::write(
        layout.operator_policy(),
        "version = \"0.1\"\n\n[cap.admission]\nmax_requests_per_minute = 1\n",
    )
    .unwrap();
    let site = Site::start(&env);
    let mut engine = site.engine(&env);
    engine.send(&json!({
        "jsonrpc": "2.0", "id": "start", "method": "crawl.start",
        "params": { "run_id": "paced", "seeds": [seed(&site.url("a.test", "/1"), 0), seed(&site.url("b.test", "/1"), 0)] },
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
    assert!(engine.finished());

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
                (site.url("a.test", "/1"), "succeeded".to_owned()),
                (site.url("b.test", "/1"), "pending".to_owned()),
            ]
        )
    );
}

/// `crawl.domain_order` as `config explain` shows it: its value, its
/// layer, and the Operator policy that locked it, if one did.
fn domain_order_explained(env: &TestEnv) -> Value {
    let printed = cli(env, env!("CARGO_BIN_EXE_goodfetch-foxhound"))
        .args(["config", "explain", "crawl.domain_order", "--json"])
        .run()
        .unwrap();
    let explained: Value = serde_json::from_str(&printed.stdout).unwrap();
    let setting = &explained["settings"][0];
    json!([
        setting["key"],
        setting["value"],
        setting["layer"],
        setting.get("locked_by")
    ])
}

/// `crawl.domain_order` is `fewest_in_flight` by default; a crawl naming
/// no order of the two is refused with `setting_invalid`; and an Operator
/// policy that locks it at `round_robin` shows in `config explain` and
/// refuses a crawl that sets it, even to the locked value, with
/// `setting_locked`, while one that leaves it unset starts (positive
/// control).
///
/// Covers: setting:crawl.domain_order, command:config.explain, error:setting_invalid, error:setting_locked, operation:crawl.start
#[test]
fn crawl_domain_order_is_explained_refused_when_invalid_and_locked_by_an_operator_policy() {
    let env = TestEnv::new();
    let by_default = domain_order_explained(&env);
    let layout = HomeLayout::of(env.home());
    std::fs::create_dir_all(layout.per_major()).unwrap();
    std::fs::write(
        layout.operator_policy(),
        "version = \"0.1\"\n\n[lock.crawl]\ndomain_order = \"round_robin\"\n",
    )
    .unwrap();
    let locked = domain_order_explained(&env);
    let site = Site::start(&env);
    let mut engine = site.engine(&env);
    let mut started = |run_id: &str, order: Option<&str>| {
        let settings = order.map_or_else(
            || json!({}),
            |order| json!({ "crawl": { "domain_order": order } }),
        );
        let started = engine.call(
            run_id,
            "crawl.start",
            &json!({
                "run_id": run_id,
                "seeds": [{ "url": site.url("ordered.test", "/1"), "label": "page" }],
                "settings": settings,
            }),
            &mut |_| None,
        );
        if started.get("result").is_some() {
            engine.call(
                &format!("{run_id}-abort"),
                "run.abort",
                &json!({ "run_id": run_id }),
                &mut |_| None,
            );
        }
        started
            .get("error")
            .map(|error| error["data"]["code"].clone())
    };

    let refused = (
        started("invalid", Some("nearest_first")),
        started("other", Some("fewest_in_flight")),
        started("same", Some("round_robin")),
        started("unset", None),
    );
    assert!(engine.finished());

    assert_eq!(
        (by_default, locked, refused),
        (
            json!(["crawl.domain_order", "fewest_in_flight", "default", null]),
            json!([
                "crawl.domain_order",
                "round_robin",
                "operator_policy",
                "operator_policy"
            ]),
            (
                Some(json!("setting_invalid")),
                Some(json!("setting_locked")),
                Some(json!("setting_locked")),
                None
            ),
        )
    );
}
