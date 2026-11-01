//! An Operator policy over a crawl's settings on Foxhound's executable
//! (spec #581's story 314; ADRs 0028 and 0086): a cap on
//! `crawl.max_requests` refuses a crawl asking for more, or for no bound,
//! with `setting_locked` naming the cap, and starts one within it; `config
//! explain` names the policy as the layer of each value it locked or
//! capped. That the policy's lock on `site_preferences.respect_robots_txt`
//! refuses a crawl turning it off is `robots_txt`'s case.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]
#![expect(
    clippy::disallowed_methods,
    reason = "the tests write the Goodfetch home's Operator policy themselves"
)]

use goodfetch_test_support::{TestEnv, cli};
use serde_json::{Value, json};

use crate::crawl::Attached;

const FOXHOUND: &str = env!("CARGO_BIN_EXE_goodfetch-foxhound");

/// Writes an Operator policy that caps `crawl.max_requests` at 100 and
/// locks robots.txt respected.
fn capping_and_locking(env: &TestEnv) {
    let folder = env.home().join("0.0");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(
        folder.join("operator-policy.toml"),
        "version = \"0.1\"\n\n[cap.crawl]\nmax_requests = 100\n\n[lock.site_preferences]\nrespect_robots_txt = true\n",
    )
    .unwrap();
}

/// What `crawl.start` answered for a crawl asking `max_requests`, aborted
/// at once when it started, its seed's work item never answered.
fn started_asking(engine: &mut Attached, run_id: &str, max_requests: &Value) -> Value {
    let started = engine.call(
        run_id,
        "crawl.start",
        &json!({
            "run_id": run_id,
            "seeds": [{ "url": "https://capped.test/1", "label": "page" }],
            "settings": { "crawl": { "max_requests": max_requests } },
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
}

/// A crawl asking for more requests than the Operator policy's cap, or for
/// no bound, is refused at its start with `setting_locked` naming the cap;
/// one asking for fewer starts (positive control).
///
/// Covers: story:6-314, setting:crawl.max_requests, error:setting_locked, operation:crawl.start
#[test]
fn a_cap_on_crawl_max_requests_refuses_a_crawl_asking_for_more() {
    let env = TestEnv::new();
    capping_and_locking(&env);
    let mut engine = Attached::start(&env);

    let more = started_asking(&mut engine, "more", &json!(1_000));
    let unbounded = started_asking(&mut engine, "unbounded", &Value::Null);
    let fewer = started_asking(&mut engine, "fewer", &json!(50));
    assert!(engine.finish());

    let cap = json!({ "key": "crawl.max_requests", "value": 100, "layer": "operator_policy" });
    assert_eq!(
        (
            (
                &more["error"]["data"]["code"],
                &more["error"]["data"]["limit"]
            ),
            (
                &unbounded["error"]["data"]["code"],
                &unbounded["error"]["data"]["limit"]
            ),
            fewer.get("error"),
        ),
        (
            (&json!("setting_locked"), &cap),
            (&json!("setting_locked"), &cap),
            None,
        ),
        "{more} {unbounded} {fewer}"
    );
}

/// `config explain` names the Operator policy as the layer of the value it
/// capped and of the one it locked, with the cap and the lock.
///
/// Covers: story:6-314, command:config.explain, setting:crawl.max_requests, setting:site_preferences.respect_robots_txt
#[test]
fn config_explain_names_the_operator_policy_as_the_layer_of_a_capped_and_a_locked_key() {
    let env = TestEnv::new();
    capping_and_locking(&env);

    let printed = cli(&env, FOXHOUND)
        .args([
            "config",
            "explain",
            "crawl.max_requests",
            "site_preferences.respect_robots_txt",
            "--json",
        ])
        .run()
        .unwrap();

    let explained: Value = serde_json::from_str(&printed.stdout).unwrap();
    let settings: Vec<Value> = explained["settings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|setting| {
            json!([
                setting["key"],
                setting["value"],
                setting["layer"],
                setting.get("locked_by"),
                setting.get("cap"),
            ])
        })
        .collect();
    assert_eq!(
        settings,
        [
            json!(["crawl.max_requests", 100, "operator_policy", null, 100]),
            json!([
                "site_preferences.respect_robots_txt",
                true,
                "operator_policy",
                "operator_policy",
                null
            ]),
        ],
        "{explained}"
    );
}
