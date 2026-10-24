//! What a crawl adds to its Run's report, through the engine's public
//! interface over the in-memory network (spec #213's Seam 3; ADR 0102):
//! `run.finish` ends a drained crawl `finished` and answers how its
//! requests ended and its Records per Dataset; a crawl no seed of which
//! reached a handler, and a resume that found nothing left, say they did
//! nothing.

use goodfetch_engine::run_state::RunStateOptions;
use goodfetch_test_support::TestEnv;
use serde_json::{Value, json};

use super::{Reply, crawl, default_dependencies, dependencies, page_of, seeds};

/// Covers: story:3-153, story:3-154
#[test]
fn a_drained_crawls_report_counts_how_its_requests_ended_and_its_records() {
    let env = TestEnv::new();

    let report = crawl(
        || dependencies(&env, RunStateOptions::default()),
        async |client| {
            client
                .call(
                    "crawl.start",
                    json!({ "run_id": "counted", "seeds": seeds(4) }),
                    &mut |_| Reply::Silent,
                )
                .await;
            let mut handler = |params: &Value| match page_of(params).as_str() {
                "https://shop.test/page/3" => Reply::Skips("Out of stock."),
                "https://shop.test/page/4" => Reply::Fails("handler_failed", false),
                page => Reply::Records(vec![json!({ "page": page })]),
            };
            client
                .call("run.finish", json!({ "run_id": "counted" }), &mut handler)
                .await
        },
    );

    let report = &report["result"];
    assert_eq!(
        (
            &report["lifecycle"],
            &report["requests"],
            &report["records"],
            &report["nothing_done"],
        ),
        (
            &json!("finished"),
            &json!({
                "succeeded": 2,
                "failed": 1,
                "skipped": 1,
                "blocked": 0,
                "pending": 0,
                "by_skip_reason": [{ "name": "handler", "count": 1 }],
                "by_error_code": [{ "name": "handler_failed", "count": 1 }],
            }),
            &json!([{ "dataset": "pages", "committed": 2, "dropped": 0, "rejected": 0 }]),
            &Value::Null,
        )
    );
}

/// Covers: story:3-156
#[test]
fn a_crawl_no_seed_of_which_reached_a_handler_did_nothing() {
    let env = TestEnv::new();

    let report = crawl(
        || dependencies(&env, RunStateOptions::default()),
        async |client| {
            client
                .call(
                    "crawl.start",
                    json!({ "run_id": "refused", "seeds": [{ "url": "ftp://shop.test/", "label": "page" }] }),
                    &mut |_| Reply::Silent,
                )
                .await;
            client
                .call("run.finish", json!({ "run_id": "refused" }), &mut |_| {
                    Reply::Silent
                })
                .await
        },
    );

    assert_eq!(
        (
            &report["result"]["lifecycle"],
            &report["result"]["nothing_done"],
        ),
        (
            &json!("finished"),
            &json!({
                "reason": "no_seed_handled",
                "seeds": [{ "reason": "unsupported_scheme", "count": 1 }],
            })
        )
    );
}

/// A keep-alive crawl checkpointed once drained, resumed on another
/// machine from that Checkpoint: nothing is left to deliver.
/// Covers: story:3-156, operation:run.resume
#[test]
fn a_resume_that_found_nothing_left_did_nothing() {
    let (first, second) = (TestEnv::new(), TestEnv::new());
    let taken = first.folder().join("drained.sqlite");
    let location = taken.display().to_string();
    let checkpoint = location.clone();
    crawl(
        || default_dependencies(&first, RunStateOptions::default()),
        async |client| {
            client
                .call(
                    "crawl.start",
                    json!({ "run_id": "drained", "seeds": seeds(1), "settings": { "crawl": { "keep_alive": true } } }),
                    &mut |_| Reply::Silent,
                )
                .await;
            let mut answer =
                |params: &Value| Reply::Records(vec![json!({ "page": page_of(params) })]);
            let page = client
                .call("run.outcomes", json!({ "run_id": "drained" }), &mut answer)
                .await;
            assert_eq!(page["result"]["outcomes"][0]["kind"], "succeeded", "{page}");
            client
                .call(
                    "run.checkpoint",
                    json!({ "run_id": "drained", "location": location }),
                    &mut answer,
                )
                .await;
        },
    );

    let finished = crawl(
        || default_dependencies(&second, RunStateOptions::default()),
        async |client| {
            client
                .call(
                    "run.resume",
                    json!({ "run_id": "drained", "checkpoint": checkpoint }),
                    &mut |_| Reply::Silent,
                )
                .await;
            client
                .call("run.finish", json!({ "run_id": "drained" }), &mut |_| {
                    Reply::Silent
                })
                .await
        },
    );

    assert_eq!(
        (
            &finished["result"]["lifecycle"],
            &finished["result"]["nothing_done"],
            &finished["result"]["requests"]["succeeded"],
        ),
        (
            &json!("finished"),
            &json!({ "reason": "nothing_pending" }),
            &json!(1),
        )
    );
}
