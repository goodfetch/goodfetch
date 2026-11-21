//! A crawl held to a Limit across runs through the engine's public
//! interface over the in-memory network (spec #213's Seam 3): each Crawl
//! request charges one `requests` against the Limits that hold the Run
//! before it is admitted, and once a Limit has nothing left the crawl
//! admits nothing new and stops at its next safe point, `stopped` and
//! `incomplete` for that Limit, its other requests still pending for a
//! resume (ADR 0041).

use goodfetch_engine::run_state::RunStateOptions;
use goodfetch_test_support::TestEnv;
use serde_json::{Value, json};

use super::{Reply, crawl, dependencies, page_of, run_file, seeds, work_items};

/// Covers: story:3-71
#[test]
fn a_crawl_stops_at_its_next_safe_point_once_a_limit_has_nothing_left() {
    let env = TestEnv::new();

    let (report, usage) = crawl(
        || dependencies(&env, RunStateOptions::default()),
        async |client| {
            client
                .call(
                    "usage.set",
                    json!({ "scope": "install", "usage_kind": "requests", "amount": 3 }),
                    &mut |_| Reply::Silent,
                )
                .await;
            client
                .call(
                    "crawl.start",
                    json!({ "run_id": "capped", "seeds": seeds(5), "labels": { "page": {} } }),
                    &mut |_| Reply::Silent,
                )
                .await;
            let mut handler =
                |params: &Value| Reply::Records(vec![json!({ "page": page_of(params) })]);
            let report = client
                .call("run.finish", json!({ "run_id": "capped" }), &mut handler)
                .await;
            let usage = client
                .call("usage.list", json!({}), &mut |_| Reply::Silent)
                .await;
            (report, usage)
        },
    );

    let states: Vec<String> = work_items(&run_file(&env, "capped"))
        .into_iter()
        .map(|(_, state, _, _)| state)
        .collect();
    let install = &usage["result"]["limits"][0];
    assert_eq!(
        (
            &report["result"]["lifecycle"],
            &report["result"]["completeness"],
            &report["result"]["stop_reason"],
            states,
            (&install["charged"], &install["held"], &install["left"]),
        ),
        (
            &json!("stopped"),
            &json!("incomplete"),
            &json!({
                "kind": "limit",
                "scope": { "kind": "install", "id": "" },
                "usage_kind": "requests",
            }),
            vec![
                "succeeded".to_owned(),
                "succeeded".to_owned(),
                "succeeded".to_owned(),
                "pending".to_owned(),
                "pending".to_owned(),
            ],
            (&json!(3), &json!(0), &json!(0)),
        )
    );
}
