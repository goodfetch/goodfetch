//! A crawl's outcomes read through the engine's public interface over the
//! in-memory network (spec #213's Seam 3), on the test `Clock` and
//! `Entropy` with blocking work inline: `run.outcomes` pages each ending in
//! commit order, with its filters run in the engine, from the first, after
//! a position or from the next commit; `run.failed_requests` lists the
//! latest `failed`, and with `blocked` also `blocked`, outcomes; and both
//! read a Run no program holds. What the handler does is scripted.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

mod readmission;

use goodfetch_engine::Dependencies;
use goodfetch_engine::run_state::RunStateOptions;
use goodfetch_test_support::TestEnv;
use rstest::rstest;
use serde_json::{Value, json};

use crate::policy::CrawlPolicy;
use crate::start::tests::{POLICY, Reply, configure, crawl, default_dependencies};

/// The handler takes one work item at a time, so the commit order is the
/// seeds' order and each test can name it.
fn one_at_a_time(env: &TestEnv) -> Dependencies {
    configure(
        env,
        CrawlPolicy {
            handler_concurrency: 1,
            ..POLICY
        },
    );
    default_dependencies(env, RunStateOptions::default())
}

/// Five seeds, of two Labels, each of which ends another way.
fn seeds() -> Value {
    json!([
        { "url": "https://shop.test/a", "label": "page" },
        { "url": "https://shop.test/b", "label": "page" },
        { "url": "https://shop.test/c", "label": "list" },
        { "url": "https://shop.test/d", "label": "page" },
        { "url": "https://shop.test/e", "label": "list" },
    ])
}

/// `a` commits two Records, `b` commits none on its second attempt, `c`
/// fails with the handler's own error, `d` is skipped and `e` blocked.
fn handler(params: &Value) -> Reply {
    let attempt = params["attempt"].as_u64();
    match params["crawl_request"]["url"].as_str() {
        Some("https://shop.test/a") => Reply::Records(vec![json!({ "n": 1 }), json!({ "n": 2 })]),
        Some("https://shop.test/b") if attempt == Some(1) => Reply::Fails("retry_requested", true),
        Some("https://shop.test/b") => Reply::Records(Vec::new()),
        Some("https://shop.test/c") => Reply::Fails("handler_failed", true),
        Some("https://shop.test/d") => Reply::Skips("not a product page"),
        _ => Reply::Blocked("challenge"),
    }
}

const ENDED_AT: &str = "2026-01-05T09:00:00Z";

fn succeeded_a() -> Value {
    json!({
        "position": 1,
        "request": { "id": "1", "url": "https://shop.test/a", "label": "page", "depth": 0 },
        "attempts": 1, "errors": [], "ended_at": ENDED_AT,
        "kind": "succeeded",
        "records": [{ "dataset": "pages", "committed": 2, "dropped": 0, "rejected": 0 }],
    })
}

fn succeeded_b() -> Value {
    json!({
        "position": 2,
        "request": { "id": "2", "url": "https://shop.test/b", "label": "page", "depth": 0 },
        "attempts": 2,
        "errors": [{ "attempt": 1, "code": "retry_requested", "message": "the handler failed" }],
        "ended_at": ENDED_AT,
        "kind": "succeeded", "records": [], "nothing_committed": "none_pushed",
    })
}

fn failed_c() -> Value {
    json!({
        "position": 3,
        "request": { "id": "3", "url": "https://shop.test/c", "label": "list", "depth": 0 },
        "attempts": 1,
        "errors": [{ "attempt": 1, "code": "handler_failed", "message": "the handler failed" }],
        "ended_at": ENDED_AT,
        "kind": "failed",
        "error": { "attempt": 1, "code": "handler_failed", "message": "the handler failed" },
    })
}

fn skipped_d() -> Value {
    json!({
        "position": 4,
        "request": { "id": "4", "url": "https://shop.test/d", "label": "page", "depth": 0 },
        "attempts": 1, "errors": [], "ended_at": ENDED_AT,
        "kind": "skipped", "reason": "handler", "message": "not a product page",
    })
}

fn blocked_e() -> Value {
    json!({
        "position": 5,
        "request": { "id": "5", "url": "https://shop.test/e", "label": "list", "depth": 0 },
        "attempts": 1, "errors": [], "ended_at": ENDED_AT,
        "kind": "blocked", "cause": "challenge",
    })
}

/// Crawls [`seeds`] to its end with [`handler`], then makes the call
/// `method` with `params` on the ended Run `shop`, and answers its reply.
fn on_the_ended_crawl(env: &TestEnv, method: &str, params: Value) -> Value {
    crawl(
        || one_at_a_time(env),
        async |client| {
            client
                .call(
                    "crawl.start",
                    json!({ "run_id": "shop", "seeds": seeds() }),
                    &mut |_| Reply::Silent,
                )
                .await;
            client
                .call("run.finish", json!({ "run_id": "shop" }), &mut handler)
                .await;
            client.call(method, params, &mut |_| Reply::Silent).await
        },
    )
}

/// Covers: story:3-149, operation:run.outcomes
#[test]
fn every_ending_of_a_crawl_is_read_in_commit_order_with_its_attempts_and_records() {
    let env = TestEnv::new();

    let page = on_the_ended_crawl(&env, "run.outcomes", json!({ "run_id": "shop" }));

    assert_eq!(
        page["result"],
        json!({
            "outcomes": [succeeded_a(), succeeded_b(), failed_c(), skipped_d(), blocked_e()],
            "next": 5,
            "ended": true,
        })
    );
}

/// The filters run in the engine: a page holds only what they take, and
/// its `next` passes what they left out.
///
/// Covers: story:3-149, operation:run.outcomes
#[rstest]
#[case::kinds(json!({ "kinds": ["failed", "blocked"] }), vec![failed_c(), blocked_e()])]
#[case::labels(json!({ "labels": ["list"] }), vec![failed_c(), blocked_e()])]
#[case::kinds_and_labels(json!({ "kinds": ["succeeded", "skipped"], "labels": ["page"] }), vec![succeeded_a(), succeeded_b(), skipped_d()])]
/// Covers: story:12-219
#[case::codes_of_the_last_attempt(json!({ "codes": ["handler_failed"] }), vec![failed_c()])]
/// Covers: story:12-219
#[case::codes_of_an_earlier_attempt(json!({ "codes": ["retry_requested", "lease_lost"] }), vec![succeeded_b()])]
/// Covers: story:12-219
#[case::codes_and_kinds(json!({ "codes": ["retry_requested"], "kinds": ["failed"] }), Vec::new())]
#[case::after_a_position(json!({ "after": 3 }), vec![skipped_d(), blocked_e()])]
#[case::after_now(json!({ "after": "now" }), Vec::new())]
fn a_filtered_read_holds_only_the_outcomes_its_filters_take(
    #[case] filters: Value,
    #[case] expected: Vec<Value>,
) {
    let env = TestEnv::new();
    let mut params = json!({ "run_id": "shop" });
    params
        .as_object_mut()
        .unwrap()
        .extend(filters.as_object().unwrap().clone());

    let page = on_the_ended_crawl(&env, "run.outcomes", params);

    assert_eq!(
        page["result"],
        json!({ "outcomes": expected, "next": 5, "ended": true })
    );
}

/// Covers: story:3-150, operation:run.failed_requests
#[rstest]
#[case::failed(false, vec![failed_c()])]
#[case::failed_and_blocked(true, vec![failed_c(), blocked_e()])]
fn the_failed_requests_are_those_whose_latest_outcome_failed(
    #[case] blocked: bool,
    #[case] expected: Vec<Value>,
) {
    let env = TestEnv::new();

    let failed = on_the_ended_crawl(
        &env,
        "run.failed_requests",
        json!({ "run_id": "shop", "blocked": blocked }),
    );

    assert_eq!(failed["result"], json!({ "requests": expected }));
}

/// A live crawl's reader waits for the next commit, and `after: "now"`
/// starts from it: the outcomes committed before the call are not read.
///
/// Covers: story:3-149, operation:run.outcomes
#[test]
fn a_reader_from_now_on_reads_only_the_endings_committed_after_its_call() {
    let env = TestEnv::new();

    let (first, from_now) = crawl(
        || one_at_a_time(&env),
        async |client| {
            client
                .call(
                    "crawl.start",
                    json!({ "run_id": "live", "seeds": seeds() }),
                    &mut |_| Reply::Silent,
                )
                .await;
            let first = client
                .call(
                    "run.outcomes",
                    json!({ "run_id": "live" }),
                    &mut |params: &Value| match params["crawl_request"]["id"].as_str() {
                        Some("1") => handler(params),
                        _ => Reply::Silent,
                    },
                )
                .await;
            let from_now = client
                .call(
                    "run.outcomes",
                    json!({ "run_id": "live", "after": "now" }),
                    &mut |params: &Value| match params["crawl_request"]["id"].as_str() {
                        Some("2") => Reply::Records(vec![json!({ "page": "b" })]),
                        _ => Reply::Silent,
                    },
                )
                .await;
            client
                .call("run.abort", json!({ "run_id": "live" }), &mut |_| {
                    Reply::Silent
                })
                .await;
            (first, from_now)
        },
    );

    let positions = |page: &Value| -> Vec<(Value, Value)> {
        page["result"]["outcomes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|outcome| {
                (
                    outcome["request"]["id"].clone(),
                    outcome["position"].clone(),
                )
            })
            .collect()
    };
    assert_eq!(
        (
            positions(&first),
            &first["result"]["ended"],
            positions(&from_now),
            &from_now["result"]["ended"],
        ),
        (
            vec![(json!("1"), json!(1))],
            &json!(false),
            vec![(json!("2"), json!(2))],
            &json!(false),
        )
    );
}

/// An ended Run no engine works is read from its run file by an engine
/// that never ran it, with no writer lock taken: a new engine on the same
/// home reads every outcome, and the sequence says it has ended.
///
/// Covers: story:3-149, story:3-150, operation:run.outcomes, operation:run.failed_requests
#[test]
fn a_run_no_program_holds_is_read_from_its_committed_state() {
    let env = TestEnv::new();
    on_the_ended_crawl(&env, "engine.info", json!({}));

    let (page, failed) = crawl(
        || one_at_a_time(&env),
        async |client| {
            let page = client
                .call(
                    "run.outcomes",
                    json!({ "run_id": "shop", "kinds": ["failed"] }),
                    &mut |_| Reply::Silent,
                )
                .await;
            let failed = client
                .call(
                    "run.failed_requests",
                    json!({ "run_id": "shop" }),
                    &mut |_| Reply::Silent,
                )
                .await;
            (page, failed)
        },
    );

    assert_eq!(
        (&page["result"], &failed["result"]),
        (
            &json!({ "outcomes": [failed_c()], "next": 5, "ended": true }),
            &json!({ "requests": [failed_c()] }),
        )
    );
}

/// Covers: error:run_not_found
#[test]
fn reading_the_outcomes_of_no_run_fails_with_run_not_found() {
    let env = TestEnv::new();

    let failed = crawl(
        || one_at_a_time(&env),
        async |client| {
            client
                .call("run.outcomes", json!({ "run_id": "never" }), &mut |_| {
                    Reply::Silent
                })
                .await
        },
    );

    assert_eq!(failed["error"]["data"]["code"], "run_not_found");
}

/// A `site_preferences_refused` that escapes the handler, raised by its
/// own navigation the site's preferences refused, ends the request
/// `skipped` with that refusal's reason, not `failed`, and is never
/// retried (ADR 0093).
///
/// Covers: story:6-244, operation:run.outcomes
#[rstest]
#[case::robots_txt("robots_txt")]
#[case::crawl_delay("crawl_delay")]
#[case::use_signals("use_signals")]
fn a_refusal_escaping_the_handler_ends_its_request_skipped_with_its_reason(
    #[case] reason: &'static str,
) {
    let env = TestEnv::new();
    let details = json!({ "url": "https://shop.test/private", "reason": reason });

    let page = crawl(
        || one_at_a_time(&env),
        async |client| {
            let seeds = json!([{ "url": "https://shop.test/a", "label": "page" }]);
            client
                .call(
                    "crawl.start",
                    json!({ "run_id": "refused", "seeds": seeds }),
                    &mut |_| Reply::Silent,
                )
                .await;
            let mut handler =
                |_: &Value| Reply::FailsWith("site_preferences_refused", false, details.clone());
            client
                .call("run.finish", json!({ "run_id": "refused" }), &mut handler)
                .await;
            client
                .call("run.outcomes", json!({ "run_id": "refused" }), &mut |_| {
                    Reply::Silent
                })
                .await
        },
    );

    assert_eq!(
        page["result"]["outcomes"],
        json!([{
            "position": 1,
            "request": { "id": "1", "url": "https://shop.test/a", "label": "page", "depth": 0 },
            "attempts": 1, "errors": [], "ended_at": ENDED_AT,
            "kind": "skipped", "reason": reason,
        }])
    );
}
