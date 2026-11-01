//! Completeness names every reason a crawl is not `complete`, on Foxhound's
//! executable over stdio (spec #581's crawl conformance cases; ADRs 0005,
//! 0045, 0048, 0107 and 0120; pain 10): a crawl with no reason is
//! `complete`; pages left `blocked` at `payment_required` or
//! `consent_required` leave it `unverified`, a bound's cutoff or a stop
//! `incomplete`, and both kinds at once `incomplete`; each reason is listed
//! in the report with its count, the same when an engine started afterwards
//! reads the report back. A crawl of nothing says why it did nothing.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use goodfetch_test_support::TestEnv;
use rstest::rstest;
use serde_json::{Value, json};

use crate::crawl::Attached;

/// A seed of the crawl, on a host no request reaches: the handler answers
/// each page without fetching it.
fn seed(path: &str) -> Value {
    json!({ "url": format!("https://shop.test/{path}"), "label": "page" })
}

/// The handler's answer for `params`'s page: a Record, or the `blocked`
/// failure with the cause its path names.
fn answer(params: &Value) -> Value {
    let url = params["crawl_request"]["url"].as_str().unwrap();
    let cause = ["payment_required", "consent_required"]
        .into_iter()
        .find(|cause| url.contains(cause));
    match cause {
        Some(cause) => json!({ "error": { "code": -32003, "message": "blocked", "data": {
            "code": "blocked", "retryable": false, "cause": cause,
        } } }),
        None => {
            json!({ "result": { "records": [{ "dataset": "pages", "data": { "url": url } }] } })
        }
    }
}

/// A reason of the report: its code, its message and its count.
fn reason(code: &str, message: &str, count: u64) -> Value {
    json!({ "code": code, "message": message, "count": count })
}

/// The reason a cause gives, having left `count` pages at it.
fn left_at(cause: &str, count: u64) -> Value {
    let (pages, were, them) = if count == 1 {
        ("page", "was", "it")
    } else {
        ("pages", "were", "them")
    };
    reason(
        cause,
        &format!(
            "{count} {pages} in the crawl's scope {were} left `blocked` with `{cause}`; its `blocked` outcomes list {them}."
        ),
        count,
    )
}

/// The Completeness of the crawl `run_id` of `seeds` under the crawl
/// settings `settings`, and its reasons, as `run.finish` answered them and
/// as an engine started afterwards reads them.
fn finished(run_id: &str, seeds: &[Value], settings: &Value) -> (Value, Value, Value) {
    let env = TestEnv::new();
    let mut engine = Attached::start(&env);
    engine.call(
        "start",
        "crawl.start",
        &json!({ "run_id": run_id, "seeds": seeds, "settings": { "crawl": settings } }),
        &mut |_| None,
    );
    let report = engine.call(
        "finish",
        "run.finish",
        &json!({ "run_id": run_id }),
        &mut |params| Some(answer(params)),
    );
    assert!(engine.finish());
    assert_eq!(report["result"]["lifecycle"], "finished", "{report}");
    let mut engine = Attached::start(&env);
    let again = engine.call(
        "again",
        "run.finish",
        &json!({ "run_id": run_id }),
        &mut |_| None,
    );
    assert!(engine.finish());
    let axis = |report: &Value| {
        (
            report["result"]["completeness"].clone(),
            report["result"]["completeness_reasons"].clone(),
        )
    };
    assert_eq!(axis(&report), axis(&again), "read back");
    let (completeness, reasons) = axis(&report);
    (
        completeness,
        reasons,
        report["result"]["nothing_done"].clone(),
    )
}

/// Each reason gives its Completeness, listed with its count; none leaves
/// the crawl `complete`, and a reason of each kind leaves it `incomplete`.
///
/// Covers: story:6-309, operation:run.finish, block-cause:payment_required, block-cause:consent_required
#[rstest]
#[case::none(&["a", "b"], json!({}), "complete", Value::Null)]
#[case::payment_required(
    &["payment_required/1", "payment_required/2", "a"],
    json!({}),
    "unverified",
    json!([left_at("payment_required", 2)]),
)]
#[case::consent_required(&["consent_required", "a"], json!({}), "unverified", json!([left_at("consent_required", 1)]))]
/// Covers: setting:crawl.max_requests
#[case::both(
    &["payment_required", "consent_required", "a"],
    json!({ "max_requests": 2 }),
    "incomplete",
    json!([
        left_at("consent_required", 1),
        reason("crawl.max_requests", "`crawl.max_requests` refused 1 URL the crawl would otherwise have admitted.", 1),
        left_at("payment_required", 1),
    ]),
)]
fn each_reason_gives_its_completeness_with_its_count(
    #[case] paths: &[&str],
    #[case] settings: Value,
    #[case] completeness: &str,
    #[case] reasons: Value,
) {
    let seeds: Vec<Value> = paths.iter().map(|path| seed(path)).collect();

    let (given, listed, nothing_done) = finished("reasons", &seeds, &settings);

    assert_eq!(
        (given, listed, nothing_done),
        (json!(completeness), reasons, Value::Null)
    );
}

/// A stopped crawl is `incomplete`, its `stopped` reason counting the
/// requests it left pending.
///
/// Covers: story:6-309, operation:run.abort
#[test]
fn a_stopped_crawl_is_incomplete_counting_the_requests_it_left() {
    let env = TestEnv::new();
    let mut engine = Attached::start(&env);
    engine.call(
        "start",
        "crawl.start",
        &json!({ "run_id": "stopped", "seeds": [seed("a"), seed("b"), seed("c")] }),
        &mut |_| None,
    );
    let report = engine.call(
        "abort",
        "run.abort",
        &json!({ "run_id": "stopped" }),
        &mut |_| None,
    );
    assert!(engine.finish());

    assert_eq!(
        (
            &report["result"]["lifecycle"],
            &report["result"]["completeness"],
            &report["result"]["completeness_reasons"],
        ),
        (
            &json!("stopped"),
            &json!("incomplete"),
            &json!([reason(
                "stopped",
                "It stopped early: aborted by its Client.",
                3
            )]),
        )
    );
}

/// A crawl none of whose seeds reached a handler reports `nothingDone`,
/// counting its seeds by why each did not.
///
/// Covers: story:6-309, story:3-156, setting:crawl.max_requests
#[test]
fn a_crawl_of_nothing_says_why_it_did_nothing() {
    let seeds = [
        json!({ "url": "ftp://shop.test/a", "label": "page" }),
        seed("b"),
        seed("c"),
    ];

    let (completeness, reasons, nothing_done) =
        finished("nothing", &seeds, &json!({ "max_requests": 0 }));

    assert_eq!(
        (completeness, reasons, nothing_done),
        (
            json!("incomplete"),
            json!([reason(
                "crawl.max_requests",
                "`crawl.max_requests` refused 2 URLs the crawl would otherwise have admitted.",
                2
            )]),
            json!({
                "reason": "no_seed_handled",
                "seeds": [
                    { "reason": "max_requests", "count": 2 },
                    { "reason": "unsupported_scheme", "count": 1 },
                ],
            }),
        )
    );
}
