//! A crawl held to a Limit across runs through the engine's public
//! interface over the in-memory network (spec #213's Seam 3): each Crawl
//! request charges one `requests` against the Limits that hold the Run
//! before it is admitted, and once a Limit has nothing left the crawl
//! admits nothing new and stops at its next safe point, `stopped` and
//! `incomplete` for that Limit, its other requests still pending for a
//! resume (ADR 0041). The Limits a `crawl.start` gives hold the crawl on
//! its own `run` scope as `run.start`'s hold a Run (ADRs 0080 and 0102).

use goodfetch_engine::run_state::RunStateOptions;
use goodfetch_test_support::TestEnv;
use serde_json::{Value, json};

use super::{Reply, crawl, dependencies, page_of, run_file, seeds, work_items};

/// `crawl.start` of the crawl `run_id` over five seeds, holding it to
/// `amount` requests.
fn start_with(run_id: &str, amount: u64) -> Value {
    json!({
        "run_id": run_id,
        "seeds": seeds(5),
        "labels": { "page": {} },
        "limits": [{ "usage_kind": "requests", "amount": amount }],
    })
}

/// The handler of the tests' crawls: one record per page.
fn one_record(params: &Value) -> Reply {
    Reply::Records(vec![json!({ "page": page_of(params) })])
}

#[test]
fn a_limit_given_at_a_crawls_start_stops_it_on_its_run_scope() {
    let env = TestEnv::new();

    let report = crawl(
        || dependencies(&env, RunStateOptions::default()),
        async |client| {
            client
                .call("crawl.start", start_with("capped", 3), &mut |_| {
                    Reply::Silent
                })
                .await;
            client
                .call("run.finish", json!({ "run_id": "capped" }), &mut one_record)
                .await
        },
    );

    assert_eq!(
        (
            &report["result"]["lifecycle"],
            &report["result"]["stop_reason"],
        ),
        (
            &json!("stopped"),
            &json!({
                "kind": "limit",
                "scope": { "kind": "run", "id": "capped" },
                "usage_kind": "requests",
            }),
        ),
        "{report:#?}"
    );
}

/// A start naming the crawl again above its Limit is refused before it
/// resumes, and one under the ID of a crawl its Limit stopped is refused
/// with `limit_reached` while that Limit stays used up (ADR 0102).
#[test]
fn a_crawl_start_naming_the_crawl_again_is_refused_above_its_limit_and_once_it_is_used_up() {
    let env = TestEnv::new();

    let (raised, used_up) = crawl(
        || dependencies(&env, RunStateOptions::default()),
        async |client| {
            client
                .call("crawl.start", start_with("held", 2), &mut |_| Reply::Silent)
                .await;
            client
                .call("run.finish", json!({ "run_id": "held" }), &mut one_record)
                .await;
            let raised = client
                .call("crawl.start", start_with("held", 8), &mut |_| Reply::Silent)
                .await;
            let used_up = client
                .call("crawl.start", start_with("held", 2), &mut |_| Reply::Silent)
                .await;
            (raised, used_up)
        },
    );

    assert_eq!(
        (
            &raised["error"]["data"]["code"],
            &used_up["error"]["data"]["code"],
            &used_up["error"]["data"]["scope"],
        ),
        (
            &json!("limit_raise_refused"),
            &json!("limit_reached"),
            &json!({ "kind": "run", "id": "held" }),
        ),
        "{raised:#?}\n{used_up:#?}"
    );
}

#[test]
fn a_crawl_start_refuses_two_limits_on_one_usage_kind_as_invalid_params() {
    let env = TestEnv::new();

    let refused = crawl(
        || dependencies(&env, RunStateOptions::default()),
        async |client| {
            let params = json!({
                "run_id": "refused",
                "seeds": seeds(1),
                "limits": [
                    { "usage_kind": "requests", "amount": 1 },
                    { "usage_kind": "requests", "amount": 2 },
                ],
            });
            client
                .call("crawl.start", params, &mut |_| Reply::Silent)
                .await
        },
    );

    assert_eq!(refused["error"]["code"], json!(-32602), "{refused:#?}");
}

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
