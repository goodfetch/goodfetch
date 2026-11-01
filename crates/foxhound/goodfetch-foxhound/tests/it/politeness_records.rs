//! Politeness records, on Foxhound's executable over stdio (spec #565's
//! seam: the protocol against the Proving ground; ADR 0086): each
//! Politeness or Site-preferences value that presses a site harder than its
//! built-in default, and any `site_preferences.robots_txt_user_agent`, is
//! recorded once in the Audit log as `politeness_weakened`, with its value,
//! its built-in default and the layer that set it, and is listed in the Run
//! report's `weakened`; the built-in default or a gentler value records
//! nothing. A Record fetched while `site_preferences.respect_robots_txt` is
//! off, from a URL its origin's robots.txt disallows, carries
//! `robots_txt: "ignored"` in its Provenance; one from an allowed URL
//! carries no mark, and with it on the disallowed URL is never fetched.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use goodfetch_test_support::{ProvingGround, ReceivedRequests, TestEnv, cli};
use rstest::rstest;
use serde_json::{Value, json};

use crate::crawl::Attached;
use crate::frontier::{Inside, crawl_from};

const FOXHOUND: &str = env!("CARGO_BIN_EXE_goodfetch-foxhound");

/// The Proving ground's seed.
const SEED: u64 = 684;

/// The `access-policy` family's first task's host, whose robots.txt
/// disallows `/private/` to the crawl.
pub(crate) const HOST: &str = "1.access-policy.test";

/// The settings of a crawl of `HOST` on the `http` tier, with `more` merged
/// in, key by key at the top level.
pub(crate) fn settings(proving_ground: &ProvingGround, more: &Value) -> Value {
    let ip = proving_ground.address().ip().to_string();
    let mut settings = json!({
        "fetch": { "tier": "http" },
        "network": { "resolve": { HOST: [ip] } },
        "egress": { "private_ranges": [HOST] },
    });
    if let (Some(settings), Some(more)) = (settings.as_object_mut(), more.as_object()) {
        for (key, value) in more {
            match (settings.get_mut(key), value) {
                (Some(Value::Object(at)), Value::Object(value)) => at.extend(value.clone()),
                _ => {
                    settings.insert(key.clone(), value.clone());
                }
            }
        }
    }
    settings
}

/// `ctx.push(dataset, record)` from the attempt `handle` gave: its reply.
fn push(inside: &mut Inside<'_>, handle: &Value, data: &Value) -> Value {
    inside.call(
        "crawl_request.push",
        &json!({
            "run_id": handle["run_id"],
            "crawl_request_id": handle["crawl_request"]["id"],
            "lease_epoch": handle["lease_epoch"],
            "dataset": "pages",
            "data": data,
        }),
    )
}

/// Crawls `paths` of `HOST` as `run_id` under `settings`, each page's
/// handler pushing its URL to the Dataset `pages`; answers the Run report.
pub(crate) fn crawl(
    engine: &mut Attached,
    proving_ground: &ProvingGround,
    run_id: &str,
    paths: &[&str],
    settings: &Value,
) -> Value {
    let seeds: Vec<Value> = paths
        .iter()
        .map(|path| json!({ "url": proving_ground.url(HOST, path), "label": "page" }))
        .collect();
    let finished = crawl_from(
        engine,
        &json!({
            "run_id": run_id,
            "seeds": seeds,
            "settings": settings,
            "datasets": [{ "name": "pages" }],
        }),
        &mut |inside, handle| {
            let url = handle["crawl_request"]["url"].clone();
            let pushed = push(inside, handle, &json!({ "url": url }));
            assert!(pushed.get("result").is_some(), "{pushed}");
            json!({ "result": { "records": [] } })
        },
    );
    assert_eq!(finished["result"]["lifecycle"], "finished", "{finished}");
    finished
}

/// Each `politeness_weakened` entry of `env`'s Audit log, as its Run, the
/// layer that set it and its details.
fn politeness_weakened(env: &TestEnv) -> Vec<Value> {
    let exported = cli(env, FOXHOUND).args(["audit", "export"]).run().unwrap();
    exported
        .stdout
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .filter(|entry| entry["kind"] == "politeness_weakened")
        .map(|entry| json!([entry["run_id"], entry["layer"], entry["details"]]))
        .collect()
}

/// The keys of a Run report's `weakened` that are Politeness or
/// Site-preferences keys, each with its value and layer.
fn weakened_politeness(report: &Value) -> Vec<Value> {
    report["result"]["weakened"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|weakened| {
            weakened["key"].as_str().is_some_and(|key| {
                key.starts_with("politeness.") || key.starts_with("site_preferences.")
            })
        })
        .map(|weakened| json!([weakened["key"], weakened["value"], weakened["layer"]]))
        .collect()
}

/// Each value that presses a site harder than its built-in default, set on
/// its own by the run, writes one `politeness_weakened` entry with the key,
/// value, default and layer, and is listed in `weakened`.
///
/// Covers: story:4-117, setting:politeness.domain.concurrency, setting:politeness.domain.delay, setting:politeness.backoff.enabled, setting:politeness.backoff.initial, setting:politeness.backoff.max, setting:site_preferences.respect_robots_txt, setting:site_preferences.robots_txt_user_agent, setting:politeness.sharing
#[rstest]
#[case::concurrency_above_one(json!({ "politeness": { "domain": { "concurrency": 2 } } }), "politeness.domain.concurrency", json!(2), json!(1))]
#[case::delay_below_a_second(json!({ "politeness": { "domain": { "delay": 500 } } }), "politeness.domain.delay", json!(500), json!(1_000))]
#[case::backoff_off(json!({ "politeness": { "backoff": { "enabled": false } } }), "politeness.backoff.enabled", json!(false), json!(true))]
#[case::backoff_initial_shortened(json!({ "politeness": { "backoff": { "initial": 1_000 } } }), "politeness.backoff.initial", json!(1_000), json!(2_000))]
#[case::backoff_max_shortened(json!({ "politeness": { "backoff": { "max": 30_000 } } }), "politeness.backoff.max", json!(30_000), json!(60_000))]
#[case::robots_txt_off(json!({ "site_preferences": { "respect_robots_txt": false } }), "site_preferences.respect_robots_txt", json!(false), json!(true))]
#[case::robots_txt_user_agent(json!({ "site_preferences": { "robots_txt_user_agent": "operator-bot" } }), "site_preferences.robots_txt_user_agent", json!("operator-bot"), Value::Null)]
#[case::politeness_not_shared(json!({ "politeness": { "sharing": "run" } }), "politeness.sharing", json!("run"), json!("shared"))]
fn a_value_pressing_a_site_harder_is_recorded_once_with_its_default_and_layer(
    #[case] more: Value,
    #[case] key: &str,
    #[case] value: Value,
    #[case] default: Value,
) {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let mut engine = Attached::start(&env);

    let report = crawl(
        &mut engine,
        &proving_ground,
        "pressing",
        &["/catalog/1"],
        &settings(&proving_ground, &more),
    );
    assert!(engine.finish());

    assert_eq!(
        (weakened_politeness(&report), politeness_weakened(&env)),
        (
            vec![json!([key, value, "run"])],
            vec![json!([
                "pressing",
                "run",
                { "setting": key, "value": value, "default": default },
            ])],
        ),
        "{report}"
    );
}

/// The built-in default, or a value gentler than it, records nothing and
/// weakens nothing (positive control).
///
/// Covers: story:4-117, setting:politeness.domain.concurrency, setting:politeness.domain.delay, setting:politeness.backoff.initial, setting:site_preferences.respect_robots_txt
#[test]
fn the_built_in_default_or_a_gentler_value_records_nothing() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let mut engine = Attached::start(&env);

    let report = crawl(
        &mut engine,
        &proving_ground,
        "gentle",
        &["/catalog/1"],
        &settings(
            &proving_ground,
            &json!({
                "politeness": {
                    "domain": { "concurrency": 1, "delay": 2_000 },
                    "backoff": { "enabled": true, "initial": 5_000, "max": 60_000 },
                },
                "site_preferences": { "respect_robots_txt": true },
            }),
        ),
    );
    assert!(engine.finish());

    assert_eq!(
        (weakened_politeness(&report), politeness_weakened(&env)),
        (Vec::<Value>::new(), Vec::<Value>::new()),
        "{report}"
    );
}

/// A value from user configuration is recorded with that layer.
///
/// Covers: story:4-117, setting:politeness.domain.concurrency
#[test]
fn a_value_from_user_configuration_is_recorded_with_its_layer() {
    let env = TestEnv::new();
    env.user_configuration("politeness.domain.concurrency = 3\n");
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let mut engine = Attached::start(&env);

    let report = crawl(
        &mut engine,
        &proving_ground,
        "configured",
        &["/catalog/1"],
        &settings(&proving_ground, &json!({})),
    );
    assert!(engine.finish());

    let layer = report["result"]["weakened"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|weakened| weakened["key"] == "politeness.domain.concurrency")
        .map(|weakened| weakened["layer"].clone());
    assert_eq!(
        politeness_weakened(&env),
        vec![json!([
            "configured",
            layer,
            { "setting": "politeness.domain.concurrency", "value": 3, "default": 1 },
        ])],
        "{report}"
    );
    assert_ne!(layer, Some(json!("run")), "{report}");
}

/// Every Record of the Run `run_id`'s Dataset `pages`, as `run.records`
/// reads them, granted credit for all at once.
pub(crate) fn records(engine: &mut Attached, run_id: &str) -> Vec<Value> {
    engine.send(&json!({
        "jsonrpc": "2.0", "id": "records", "method": "run.records",
        "params": { "run_id": run_id, "dataset": "pages" },
    }));
    engine.send(&json!({
        "jsonrpc": "2.0", "method": "$/credit", "params": { "id": "records", "credits": 1000 },
    }));
    let mut read = Vec::new();
    loop {
        let message = engine
            .next()
            .expect("the engine closed before the reader answered");
        if message["method"] == "$/record" {
            read.push(message["params"]["record"].clone());
        } else if message["id"] == "records" {
            assert!(message.get("result").is_some(), "{message}");
            return read;
        }
    }
}

/// The path of each Record's URL, with its Provenance's robots.txt mark,
/// sorted by path.
pub(crate) fn marks(records: &[Value]) -> Vec<(String, Value)> {
    let mut marks: Vec<(String, Value)> = records
        .iter()
        .map(|record| {
            let url = record["value"]["url"].as_str().unwrap();
            let rest = url.split_once(HOST).unwrap().1;
            let path = rest[rest.find('/').unwrap()..].to_owned();
            let mark = record["provenance"]
                .get("robots_txt")
                .cloned()
                .unwrap_or(Value::Null);
            (path, mark)
        })
        .collect();
    marks.sort_by(|left, right| left.0.cmp(&right.0));
    marks
}

/// The paths `HOST` received in the case, robots.txt left out.
fn pages_received(proving_ground: &ProvingGround) -> Vec<String> {
    let observations = proving_ground.observations().unwrap();
    let mut received: Vec<String> = ReceivedRequests::of(&observations)
        .of_the_case()
        .filter(|request| request["host"] == HOST && request["target"] != "/robots.txt")
        .map(|request| request["target"].as_str().unwrap().to_owned())
        .collect();
    received.sort();
    received
}

/// With `site_preferences.respect_robots_txt` off, the Record of a URL its
/// origin's robots.txt disallows carries `robots_txt: "ignored"` in its
/// Provenance, and the Record of an allowed URL no mark; with it on, the
/// disallowed URL is never fetched and gives no Record.
///
/// Covers: story:4-118, setting:site_preferences.respect_robots_txt, operation:run.records
#[test]
fn a_record_fetched_against_robots_txt_is_marked_ignored_in_its_provenance() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let paths = ["/catalog/1", "/private/catalog/1"];

    let mut engine = Attached::start(&env);
    crawl(
        &mut engine,
        &proving_ground,
        "ignoring",
        &paths,
        &settings(
            &proving_ground,
            &json!({ "site_preferences": { "respect_robots_txt": false } }),
        ),
    );
    let ignoring = records(&mut engine, "ignoring");
    let received_ignoring = pages_received(&proving_ground);
    crawl(
        &mut engine,
        &proving_ground,
        "respecting",
        &paths,
        &settings(&proving_ground, &json!({})),
    );
    let respecting = records(&mut engine, "respecting");
    let received_respecting = pages_received(&proving_ground);
    assert!(engine.finish());

    assert_eq!(
        (
            marks(&ignoring),
            received_ignoring,
            marks(&respecting),
            received_respecting,
        ),
        (
            vec![
                ("/catalog/1".to_owned(), Value::Null),
                ("/private/catalog/1".to_owned(), json!("ignored")),
            ],
            vec!["/catalog/1".to_owned(), "/private/catalog/1".to_owned()],
            vec![("/catalog/1".to_owned(), Value::Null)],
            vec![
                "/catalog/1".to_owned(),
                "/catalog/1".to_owned(),
                "/private/catalog/1".to_owned(),
            ],
        )
    );
}
