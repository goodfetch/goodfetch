//! A crawl's Verdict (ADR 0029), through the engine's public interface over
//! the in-memory network: Block classification switched off is a check the
//! crawl's results rely on, so its Run ends `needs_review` with a reason
//! naming `fetch.block_classification`; a control switched off that removes
//! no check on the results, such as `provenance.log`, is listed in
//! `weakened` only; with nothing switched off the crawl is `verified`; and
//! a finished crawl that returns to `running` and ends again is given its
//! Verdict anew (ADR 0102).

use goodfetch_engine::run_state::RunStateOptions;
use goodfetch_test_support::TestEnv;
use serde_json::{Value, json};

use super::{Reply, crawl, dependencies, page_of, seeds};

/// Answers each page with one Record naming it.
fn answer_all(params: &Value) -> Reply {
    Reply::Records(vec![json!({ "page": page_of(params) })])
}

/// The report of the crawl `run_id` of two seeds, started with `settings`,
/// once `run.finish` drained it.
fn finished_with(run_id: &str, settings: &Value) -> Value {
    let env = TestEnv::new();
    let report = crawl(
        || dependencies(&env, RunStateOptions::default()),
        async |client| {
            client
                .call(
                    "crawl.start",
                    json!({ "run_id": run_id, "seeds": seeds(2), "settings": settings }),
                    &mut |_| Reply::Silent,
                )
                .await;
            client
                .call("run.finish", json!({ "run_id": run_id }), &mut answer_all)
                .await
        },
    );
    report["result"].clone()
}

/// The keys of a report's `weakened` list.
fn weakened(report: &Value) -> Vec<&str> {
    report["weakened"]
        .as_array()
        .map(|listed| {
            listed
                .iter()
                .filter_map(|setting| setting["key"].as_str())
                .collect()
        })
        .unwrap_or_default()
}

/// The codes of a report's `verdict_reasons`.
fn reasons(report: &Value) -> Vec<&str> {
    report["verdict_reasons"]
        .as_array()
        .map(|given| {
            given
                .iter()
                .filter_map(|reason| reason["code"].as_str())
                .collect()
        })
        .unwrap_or_default()
}

/// Covers: story:11-128
#[test]
fn a_crawl_with_block_classification_off_needs_review_naming_the_key() {
    let report = finished_with(
        "unclassified",
        &json!({ "fetch": { "block_classification": false } }),
    );

    assert_eq!(
        (
            &report["lifecycle"],
            &report["verdict"],
            reasons(&report),
            report["verdict_reasons"][0]["message"]
                .as_str()
                .is_some_and(|message| message.contains("`fetch.block_classification`")),
            weakened(&report),
        ),
        (
            &json!("finished"),
            &json!("needs_review"),
            vec!["fetch.block_classification"],
            true,
            vec!["fetch.block_classification"],
        ),
        "{report}"
    );
}

/// Covers: story:11-128
#[test]
fn a_crawl_with_only_provenance_off_is_verified_with_the_key_in_weakened() {
    let report = finished_with("unlogged", &json!({ "provenance": { "log": false } }));

    assert_eq!(
        (&report["verdict"], reasons(&report), weakened(&report)),
        (
            &json!("verified"),
            Vec::<&str>::new(),
            vec!["provenance.log"]
        ),
        "{report}"
    );
}

/// Covers: story:11-128
#[test]
fn a_crawl_with_nothing_switched_off_is_verified_with_no_reason() {
    let report = finished_with("checked", &json!({}));

    assert_eq!(
        (
            &report["verdict"],
            &report["verdict_reasons"],
            weakened(&report)
        ),
        (&json!("verified"), &Value::Null, Vec::<&str>::new()),
        "{report}"
    );
}

/// A crawl that ended `verified`, whose failed request is retried with
/// Block classification switched off: the Run returns to `running`, and
/// its next end gives the Verdict anew, `needs_review` naming the key.
/// The other way round stays `needs_review`, since `weakened` keeps every
/// switch the Run ran with (ADR 0016).
///
/// Covers: story:11-128, story:6-306, operation:run.retry_failed
#[test]
fn a_retried_crawl_is_given_its_verdict_anew_when_it_ends_again() {
    let env = TestEnv::new();

    let (first, retried, second) = crawl(
        || dependencies(&env, RunStateOptions::default()),
        async |client| {
            client
                .call(
                    "crawl.start",
                    json!({ "run_id": "again", "seeds": seeds(1) }),
                    &mut |_| Reply::Silent,
                )
                .await;
            let first = client
                .call("run.finish", json!({ "run_id": "again" }), &mut |_| {
                    Reply::Fails("handler_failed", false)
                })
                .await;
            let retried = client
                .call(
                    "run.retry_failed",
                    json!({
                        "run_id": "again",
                        "settings": { "fetch": { "block_classification": false } },
                    }),
                    &mut answer_all,
                )
                .await;
            let second = client
                .call("run.finish", json!({ "run_id": "again" }), &mut answer_all)
                .await;
            (first, retried, second)
        },
    );

    assert_eq!(
        (
            &first["result"]["verdict"],
            &first["result"]["verdict_reasons"],
            &retried["result"],
            &second["result"]["lifecycle"],
            &second["result"]["verdict"],
            reasons(&second["result"]),
        ),
        (
            &json!("verified"),
            &Value::Null,
            &json!({ "readmitted": 1 }),
            &json!("finished"),
            &json!("needs_review"),
            vec!["fetch.block_classification"],
        ),
        "{first}\n{second}"
    );
}
