//! A crawl stopping at its next safe point through the engine's public
//! interface over the in-memory network (spec #213's Seam 3), on the paused
//! clock: once its `timeout.run` passes it admits nothing new, each attempt
//! in flight commits or is abandoned under its lease, and the Run stops
//! `stopped` and `incomplete` with the stop reason `timeout`.

use std::time::Duration;

use goodfetch_engine::run_state::RunStateOptions;
use goodfetch_test_support::TestEnv;
use serde_json::{Value, json};

use super::{Reply, crawl_connected, dependencies, page_of, run_file, work_items};

fn seeds(urls: &[&str]) -> Value {
    urls.iter()
        .map(|url| json!({ "url": url, "label": "page" }))
        .collect()
}

/// Covers: story:3-52, setting:timeout.run
#[test]
fn a_crawl_past_its_run_timeout_admits_nothing_new_and_stops_with_the_stop_reason_timeout() {
    let env = TestEnv::new();

    let report = crawl_connected(
        &json!({ "timeout": { "run": 15_000 } }),
        || dependencies(&env, RunStateOptions::default()),
        async |client| {
            client
                .call(
                    "crawl.start",
                    json!({
                        "labels": { "page": {} },
                        "run_id": "timed",
                        "seeds": seeds(&["https://a.test/1", "https://b.test/1", "https://c.test/1"]),
                        "settings": { "admission": { "max_requests_per_minute": 6 } },
                    }),
                    &mut |_| Reply::Silent,
                )
                .await;
            let mut handler =
                |params: &Value| Reply::Records(vec![json!({ "page": page_of(params) })]);
            client
                .call("run.finish", json!({ "run_id": "timed" }), &mut handler)
                .await
        },
    );

    assert_eq!(
        (
            (
                &report["result"]["lifecycle"],
                &report["result"]["completeness"],
                &report["result"]["stop_reason"],
            ),
            work_items(&run_file(&env, "timed")),
        ),
        (
            (
                &json!("stopped"),
                &json!("incomplete"),
                &json!({ "kind": "timeout" })
            ),
            vec![
                ("1".to_owned(), "succeeded".to_owned(), 1, None),
                ("2".to_owned(), "succeeded".to_owned(), 1, None),
                ("3".to_owned(), "pending".to_owned(), 0, None),
            ],
        )
    );
}

/// The Run waits for its two attempts' leases, 11 s, past its 5 s
/// timeout, and admits neither work item again.
///
/// Covers: story:3-52
#[test]
fn an_attempt_in_flight_at_the_run_timeout_ends_under_its_lease_before_the_run_stops() {
    let env = TestEnv::new();

    let (report, arrivals) = crawl_connected(
        &json!({ "timeout": { "run": 5_000 } }),
        || dependencies(&env, RunStateOptions::default()),
        async |client| {
            client
                .call(
                    "crawl.start",
                    json!({
                        "labels": { "page": {} },
                        "run_id": "held",
                        "seeds": seeds(&["https://a.test/1", "https://a.test/2"]),
                    }),
                    &mut |_| Reply::Silent,
                )
                .await;
            let report = client
                .call("run.finish", json!({ "run_id": "held" }), &mut |_| {
                    Reply::Silent
                })
                .await;
            let first = client.arrivals.first().map(|(_, at)| *at).unwrap();
            let arrivals: Vec<(String, Duration)> = client
                .arrivals
                .iter()
                .map(|(message, at)| (message["method"].as_str().unwrap().to_owned(), *at - first))
                .collect();
            (report, arrivals)
        },
    );

    assert_eq!(
        (
            &report["result"]["stop_reason"],
            (
                &report["result"]["started_at"],
                &report["result"]["ended_at"]
            ),
            arrivals,
            work_items(&run_file(&env, "held")),
        ),
        (
            &json!({ "kind": "timeout" }),
            (
                &json!("2026-01-05T09:00:00Z"),
                &json!("2026-01-05T09:00:11Z")
            ),
            vec![
                ("crawl_request.handle".to_owned(), Duration::ZERO),
                ("crawl_request.handle".to_owned(), Duration::ZERO),
            ],
            vec![
                (
                    "1".to_owned(),
                    "pending".to_owned(),
                    1,
                    Some("handler_lost".to_owned())
                ),
                (
                    "2".to_owned(),
                    "pending".to_owned(),
                    1,
                    Some("handler_lost".to_owned())
                ),
            ],
        )
    );
}
