//! Run conformance cases of `run.retry_failed` on Foxhound's executable
//! over stdio (spec #581's seam: the protocol; ADR 0102; golden scenario
//! 07): once a crawl finished, `run.retry_failed` re-admits only the
//! requests whose latest outcome is `failed`, each keeping its ID, Label,
//! Payload and depth with a fresh `crawl.retry.max`, answers how many and
//! returns the Run to `running` in the same Run, its calling Connection
//! serving the handlers; with `blocked: true` it also takes the blocked
//! ones; and with nothing to retry it answers 0 and leaves the Run as it
//! is.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use std::collections::BTreeMap;

use goodfetch_test_client::Attached;
use goodfetch_test_support::TestEnv;
use serde_json::{Value, json};

use crate::site::{Site, path_of};

/// The Run ID of the crawl the cases retry.
const RUN_ID: &str = "retried";

/// A handler's failure with the Error code `code` and `details`.
fn failure(code: &str, details: &Value) -> Value {
    let mut data = json!({ "code": code, "retryable": true });
    data.as_object_mut()
        .unwrap()
        .extend(details.as_object().unwrap().clone());
    json!({ "error": { "code": -32003, "message": code, "data": data } })
}

/// One Record of `pages` naming the request's URL.
fn kept(request: &Value) -> Value {
    json!({ "result": { "records": [{ "dataset": "pages", "data": { "url": request["url"] } }] } })
}

/// What a handler was handed: each request's ID, URL, Label and Payload,
/// in the order it was asked.
#[derive(Default)]
struct Handed(Vec<Value>);

impl Handed {
    fn note(&mut self, request: &Value) {
        self.0.push(json!({
            "id": request["id"],
            "url": request["url"],
            "label": request["label"],
            "payload": request.get("payload").cloned().unwrap_or(Value::Null),
        }));
    }

    /// How many times each URL was handed.
    fn counts(&self) -> BTreeMap<String, usize> {
        let mut counts = BTreeMap::new();
        for request in &self.0 {
            *counts
                .entry(request["url"].as_str().unwrap().to_owned())
                .or_default() += 1;
        }
        counts
    }
}

/// Sends `method` with `params` as the call `id` and answers each crawl
/// request `handle` takes, given what it is handed, until the call's
/// reply, which it answers.
fn serve(
    engine: &mut Attached,
    id: &str,
    method: &str,
    params: &Value,
    handle: &mut impl FnMut(&mut Attached, &Value) -> Value,
) -> Value {
    engine.send(&json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }));
    loop {
        let message = engine.next().expect("the engine closed before replying");
        if message["method"] == "crawl_request.handle" {
            let mut reply = handle(engine, &message["params"]);
            reply["jsonrpc"] = json!("2.0");
            reply["id"] = message["id"].clone();
            engine.send(&reply);
        } else if message["id"] == id {
            return message;
        }
    }
}

/// `run.retry_failed` of the crawl, with `blocked`, answered once the
/// Run's handlers are served until it ends again: its answer, and the
/// report `run.finish` then returns.
fn retry_and_finish(
    engine: &mut Attached,
    blocked: bool,
    handle: &mut impl FnMut(&mut Attached, &Value) -> Value,
) -> (Value, Value) {
    let retried = serve(
        engine,
        &format!("retry-{blocked}"),
        "run.retry_failed",
        &json!({ "run_id": RUN_ID, "blocked": blocked, "settings": settings() }),
        handle,
    );
    let finished = serve(
        engine,
        &format!("finish-{blocked}"),
        "run.finish",
        &json!({ "run_id": RUN_ID }),
        handle,
    );
    (retried, finished)
}

/// One request at a time, and one retry of a failed attempt.
fn settings() -> Value {
    json!({ "crawl": { "handler": { "concurrency": 1 }, "retry": { "max": 1, "handler_failed": true } } })
}

/// Each listed request's latest outcome, by URL: its kind, ID, Label,
/// depth and attempts.
fn latest(outcomes: &Value) -> BTreeMap<String, Value> {
    outcomes["result"]["outcomes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|outcome| {
            (
                outcome["request"]["url"].as_str().unwrap().to_owned(),
                json!({
                    "kind": outcome["kind"],
                    "id": outcome["request"]["id"],
                    "label": outcome["request"]["label"],
                    "depth": outcome["request"]["depth"],
                    "attempts": outcome["attempts"],
                }),
            )
        })
        .collect()
}

/// A finished crawl's failures, a seed with a Payload and a request its
/// handler enqueued at depth 1, are re-admitted by `run.retry_failed` in
/// the same Run, which answers 2 and runs again: each is handed to the
/// handler with its ID, Label and Payload, and spends a fresh
/// `crawl.retry.max` of 1, so two more attempts, counted on from its
/// first two. The skipped and blocked requests stay as they ended. With
/// `blocked: true` the next retry takes both failures again and the
/// blocked request, whose latest outcomes are then `succeeded`, each
/// keeping its depth, every Record committed once in `pages`; and a retry
/// with nothing failed answers 0, the Run left finished.
///
/// Covers: story:6-306, operation:run.retry_failed, operation:run.outcomes, operation:run.failed_requests, setting:crawl.retry.max
#[test]
#[expect(
    clippy::too_many_lines,
    reason = "one test drives one Run through its first crawl and two retries, whose endings it compares together"
)]
fn retry_failed_re_admits_only_the_failures_with_their_identity_and_a_fresh_retry_budget() {
    let env = TestEnv::new();
    let site = Site::start(&env);
    let mut engine = site.engine(&env);
    let book = json!({ "isbn": "9780000000001" });
    let flaky = json!({ "note": "Ünïcødé ✓" });
    let mut first = Handed::default();
    let start = json!({
        "run_id": RUN_ID,
        "datasets": [{ "name": "pages" }],
        "labels": { "shelf": {}, "book": {} },
        "seeds": [
            { "url": site.page("/shelf"), "label": "shelf" },
            { "url": site.page("/flaky"), "label": "book", "payload": flaky },
            { "url": site.page("/skipped"), "label": "book" },
            { "url": site.page("/blocked"), "label": "book" },
        ],
        "settings": settings(),
    });
    let failing = |engine: &mut Attached, handle: &Value, handed: &mut Handed| {
        let request = &handle["crawl_request"];
        handed.note(request);
        match path_of(&request["url"]) {
            "/shelf" => {
                let enqueued = serve(
                    engine,
                    "enqueue",
                    "crawl_request.enqueue",
                    &json!({
                        "run_id": handle["run_id"],
                        "crawl_request_id": request["id"],
                        "lease_epoch": handle["lease_epoch"],
                        "url": site.page("/book/1"),
                        "label": "book",
                        "payload": book,
                    }),
                    &mut |_, _| json!({}),
                );
                assert_eq!(enqueued["result"]["kind"], json!("added"), "{enqueued}");
                kept(request)
            }
            "/skipped" => {
                json!({ "result": { "skipped": { "message": "out of season" } } })
            }
            "/blocked" => failure("blocked", &json!({ "cause": "captcha" })),
            _ => failure("handler_failed", &json!({})),
        }
    };
    let started = serve(
        &mut engine,
        "start",
        "crawl.start",
        &start,
        &mut |engine, handle| failing(engine, handle, &mut first),
    );
    assert_eq!(started["result"]["run_id"], json!(RUN_ID), "{started}");
    let finished = serve(
        &mut engine,
        "finish",
        "run.finish",
        &json!({ "run_id": RUN_ID }),
        &mut |engine, handle| failing(engine, handle, &mut first),
    );
    assert_eq!(
        finished["result"]["lifecycle"],
        json!("finished"),
        "{finished}"
    );

    let mut second = Handed::default();
    let (retried, refinished) = retry_and_finish(&mut engine, false, &mut |engine, handle| {
        failing(engine, handle, &mut second)
    });
    let after_second = latest(&serve(
        &mut engine,
        "outcomes-2",
        "run.outcomes",
        &json!({ "run_id": RUN_ID, "kinds": ["failed", "blocked", "skipped"] }),
        &mut |_, _| json!({}),
    ));

    let mut third = Handed::default();
    let (retried_blocked, last) = retry_and_finish(&mut engine, true, &mut |_, handle| {
        third.note(&handle["crawl_request"]);
        kept(&handle["crawl_request"])
    });
    let nothing = serve(
        &mut engine,
        "nothing",
        "run.retry_failed",
        &json!({ "run_id": RUN_ID, "blocked": true }),
        &mut |_, _| json!({}),
    );
    let report = serve(
        &mut engine,
        "report",
        "run.report",
        &json!({ "run_id": RUN_ID }),
        &mut |_, _| json!({}),
    );
    let after_third = latest(&serve(
        &mut engine,
        "outcomes-3",
        "run.outcomes",
        &json!({ "run_id": RUN_ID, "kinds": ["succeeded"] }),
        &mut |_, _| json!({}),
    ));
    let failed = serve(
        &mut engine,
        "failed",
        "run.failed_requests",
        &json!({ "run_id": RUN_ID, "blocked": true }),
        &mut |_, _| json!({}),
    );
    assert!(engine.finished());

    let first_of = |url: &str| {
        first
            .0
            .iter()
            .find(|request| request["url"] == json!(url))
            .cloned()
            .unwrap()
    };
    let (book_1, flaky_seed) = (
        first_of(&site.page("/book/1")),
        first_of(&site.page("/flaky")),
    );
    // Each failure handed again as it was first: its ID, Label and Payload.
    assert_eq!(
        (
            &retried["result"],
            &refinished["result"]["lifecycle"],
            second.counts(),
            second.0.contains(&book_1) && second.0.contains(&flaky_seed),
            &book_1["payload"],
            &flaky_seed["payload"],
        ),
        (
            &json!({ "readmitted": 2 }),
            &json!("finished"),
            BTreeMap::from([(site.page("/book/1"), 2), (site.page("/flaky"), 2),]),
            true,
            &book,
            &flaky,
        ),
        "{retried} {refinished}"
    );
    // A fresh budget: two attempts more, counted on from the first two.
    assert_eq!(
        (
            &after_second[&site.page("/book/1")],
            &after_second[&site.page("/flaky")],
            &after_second[&site.page("/blocked")]["kind"],
            &after_second[&site.page("/skipped")]["kind"],
        ),
        (
            &json!({ "kind": "failed", "id": book_1["id"], "label": "book", "depth": 1, "attempts": 4 }),
            &json!({ "kind": "failed", "id": flaky_seed["id"], "label": "book", "depth": 0, "attempts": 4 }),
            &json!("blocked"),
            &json!("skipped"),
        )
    );
    // `blocked: true` takes the blocked request too; then nothing is left.
    assert_eq!(
        (
            &retried_blocked["result"],
            &last["result"]["lifecycle"],
            third.counts(),
            &after_third[&site.page("/book/1")]["depth"],
            &after_third[&site.page("/blocked")]["id"],
            &failed["result"]["requests"],
            &nothing["result"],
            &report["result"]["lifecycle"],
        ),
        (
            &json!({ "readmitted": 3 }),
            &json!("finished"),
            BTreeMap::from([
                (site.page("/blocked"), 1),
                (site.page("/book/1"), 1),
                (site.page("/flaky"), 1),
            ]),
            &json!(1),
            &first_of(&site.page("/blocked"))["id"],
            &json!([]),
            &json!({ "readmitted": 0 }),
            &json!("finished"),
        ),
        "{last}"
    );
    // Every Record of the Run in its one Dataset, each request's once.
    assert_eq!(
        last["result"]["records"],
        json!([{ "dataset": "pages", "committed": 4, "dropped": 0, "rejected": 0 }]),
        "{last}"
    );
}

/// On a crawl still running, `crawl.keep_alive` holding it idle once its
/// one request failed, `run.retry_failed` re-admits that request into the
/// live Run at once: its handler is asked for it again, under its ID, and
/// the Run, let finish, commits its Record once.
///
/// Covers: story:6-306, operation:run.retry_failed, setting:crawl.keep_alive
#[test]
fn retry_failed_re_admits_a_failure_into_a_crawl_still_running() {
    let env = TestEnv::new();
    let site = Site::start(&env);
    let mut engine = site.engine(&env);
    let mut handed = Handed::default();
    let mut handle = |_: &mut Attached, handle: &Value| {
        let request = &handle["crawl_request"];
        handed.note(request);
        if handed.0.len() == 1 {
            failure("handler_failed", &json!({}))
        } else {
            kept(request)
        }
    };
    let start = json!({
        "run_id": "live",
        "datasets": [{ "name": "pages" }],
        "seeds": [{ "url": site.page("/once"), "label": "page" }],
        "settings": { "crawl": { "keep_alive": true, "retry": { "max": 0 } } },
    });
    serve(&mut engine, "start", "crawl.start", &start, &mut handle);
    let failed = serve(
        &mut engine,
        "failed",
        "run.outcomes",
        &json!({ "run_id": "live", "kinds": ["failed"] }),
        &mut handle,
    );
    let retried = serve(
        &mut engine,
        "retry",
        "run.retry_failed",
        &json!({ "run_id": "live" }),
        &mut handle,
    );
    let succeeded = serve(
        &mut engine,
        "succeeded",
        "run.outcomes",
        &json!({ "run_id": "live", "kinds": ["succeeded"] }),
        &mut handle,
    );
    let finished = serve(
        &mut engine,
        "finish",
        "run.finish",
        &json!({ "run_id": "live" }),
        &mut handle,
    );
    assert!(engine.finished());

    assert_eq!(
        (
            &failed["result"]["outcomes"][0]["kind"],
            &retried["result"],
            handed.counts(),
            handed.0[0]["id"] == handed.0[1]["id"],
            &succeeded["result"]["outcomes"][0]["request"]["id"],
            &finished["result"]["lifecycle"],
            &finished["result"]["records"],
        ),
        (
            &json!("failed"),
            &json!({ "readmitted": 1 }),
            BTreeMap::from([(site.page("/once"), 2)]),
            true,
            &handed.0[0]["id"],
            &json!("finished"),
            &json!([{ "dataset": "pages", "committed": 1, "dropped": 0, "rejected": 0 }]),
        ),
        "{finished}"
    );
}
