//! An attempt that panics (issue 2285, ADR 0148): it ends as a failed
//! attempt with `engine_panicked`, `ended: attempt`, through the same end
//! as every other attempt, so its place on its Connection, its Session's
//! use, its domain's turn and its Lease are given back; it is retried as
//! `crawl.retry.engine_panicked` says, and its domain does not back off.
//! The panic comes from the test's site, inside the attempt's fetch.

use std::time::Duration;

use goodfetch_engine::run_state::RunStateOptions;
use goodfetch_test_support::TestEnv;
use rstest::rstest;
use serde_json::{Value, json};

use super::{PANICS, Reply, crawl, dependencies, page_of, run_file, work_items};

/// A crawl of one domain, one work item at a time, whose second seed's
/// attempts panic: the third seed's request still goes, at once after the
/// first's, since the panic neither kept the Connection's one place, the
/// domain's turn nor its Lease, nor backed the domain off. The panicking
/// work item is tried again up to `crawl.retry.max` (2 in the tests'
/// policy) with `crawl.retry.engine_panicked` on, its default, and ends
/// `failed` at once with it off; the Run finishes with nothing leased.
///
/// Covers: story:3-41, setting:crawl.retry.engine_panicked, error:engine_panicked
#[rstest]
#[case::retried_by_default(json!({}), 3)]
#[case::not_retried_when_switched_off(json!({ "retry": { "engine_panicked": false } }), 1)]
fn an_attempt_that_panics_ends_failed_and_gives_back_what_it_held(
    #[case] retry: Value,
    #[case] attempts: i64,
) {
    let env = TestEnv::new();
    let mut crawl_settings = json!({ "handler": { "concurrency": 1 } });
    crawl_settings
        .as_object_mut()
        .unwrap()
        .extend(retry.as_object().unwrap().clone());

    let (report, sent) = crawl(
        || dependencies(&env, RunStateOptions::default()),
        async |client| {
            client
                .call(
                    "crawl.start",
                    json!({
                        "labels": { "page": {} },
                        "run_id": "panics",
                        "seeds": [
                            { "url": "https://shop.test/page/1", "label": "page" },
                            { "url": format!("https://shop.test{PANICS}"), "label": "page" },
                            { "url": "https://shop.test/page/2", "label": "page" },
                        ],
                        "settings": { "crawl": crawl_settings },
                    }),
                    &mut |_| Reply::Silent,
                )
                .await;
            let mut handler =
                |params: &Value| Reply::Records(vec![json!({ "page": page_of(params) })]);
            let report = client
                .call("run.finish", json!({ "run_id": "panics" }), &mut handler)
                .await;
            let sent: Vec<(String, tokio::time::Instant)> = client
                .arrivals
                .iter()
                .filter(|(message, _)| message["method"] == "crawl_request.handle")
                .map(|(message, at)| (page_of(&message["params"]), *at))
                .collect();
            (report, sent)
        },
    );

    let file = run_file(&env, "panics");
    let items: Vec<(String, i64, Option<String>)> = work_items(&file)
        .into_iter()
        .map(|(_, status, attempts, code)| (status, attempts, code))
        .collect();
    let details: Option<String> = file
        .query_row(
            "SELECT details FROM attempt_errors ORDER BY attempt DESC LIMIT 1",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let urls: Vec<&str> = sent.iter().map(|(url, _)| url.as_str()).collect();
    assert_eq!(
        (
            &report["result"]["lifecycle"],
            &report["result"]["requests"]["succeeded"],
            &report["result"]["requests"]["failed"],
            items,
            details.map(|details| serde_json::from_str::<Value>(&details).unwrap()),
            urls,
            sent[1].1 - sent[0].1,
        ),
        (
            &json!("finished"),
            &json!(2),
            &json!(1),
            vec![
                ("succeeded".to_owned(), 1, None),
                (
                    "failed".to_owned(),
                    attempts,
                    Some("engine_panicked".to_owned())
                ),
                ("succeeded".to_owned(), 1, None),
            ],
            Some(json!({ "ended": "attempt" })),
            vec!["https://shop.test/page/1", "https://shop.test/page/2"],
            Duration::ZERO,
        ),
        "{report}"
    );
}
