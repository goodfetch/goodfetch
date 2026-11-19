//! The raw catalog call (ADR 0099, G8), the `raw-call` client conformance
//! case: `call(operation).params(…)` on the Connection and on each value
//! reaches an operation the SDK does not generate, here Foxhound's from a
//! Collie program, its params and result the wire's JSON in snake case,
//! under the main path's protections. The Daemon is the Scripted engine as
//! a Foxhound engine, playing the case's script without the signal, which
//! Rust has no option for, as Python's twin plays it; it fails on a
//! message it does not expect and on a step left unused, so its clean
//! exit says nothing was retried or sent unasked. TypeScript's
//! `raw-call.test.ts` and Python's `test_raw_call.py` are its twins.
//! Covers: story:11-181

use std::time::Duration;

use goodfetch_collie::{Collie, EngineChoice, Error, ErrorCode, blocking};
use goodfetch_test_support::{SCRIPTED_DAEMON_TOKEN, TestEnv, real_time_runtime};
use serde_json::{Value, json};

use super::engine_choice::{hello, played_whole, scripted};
use super::options;

/// The case's script without the signal's call.
const SCRIPT: &str = "sdk-python/run/raw-call.json";

/// The Run the script starts.
const RUN_ID: &str = "01ke6p4ym0j8x3v5wq2rtbn7hc";

/// What each call of the case answered, in the order the case makes them.
struct Played {
    listed: Result<Value, Error>,
    failed: Result<Value, Error>,
    named: Result<Value, Error>,
    other_namespace: Result<Value, Error>,
    own_param: Result<Value, Error>,
    deadline: Result<Value, Error>,
    reloaded: Result<Value, Error>,
    clicked: Result<Value, Error>,
    on_closed: [Result<Value, Error>; 3],
}

/// The connection's options: the Daemon of `env`'s home, which the
/// Scripted engine is.
fn daemon(env: &TestEnv) -> goodfetch_collie::ConnectOptions {
    options(env).engine(EngineChoice::Daemon)
}

/// The code of the error `result` holds.
fn code(result: &Result<Value, Error>) -> ErrorCode {
    match result {
        Ok(value) => panic!("the call answered {value}"),
        Err(error) => error.code(),
    }
}

/// Asserts what the case's calls answered, whichever face made them.
fn assert_played(played: &Played) {
    assert_eq!(
        played.listed.as_ref().unwrap(),
        &json!({
            "runs": [{
                "run_id": RUN_ID,
                "location": format!("/runs/{RUN_ID}"),
                "kind": "crawl",
                "needs": "foxhound",
                "tags": ["nightly"],
                "lifecycle": "running",
                "started_at": "2026-11-19T12:30:00Z",
                "presence": "present",
                "replayed": false,
            }]
        }),
        "the Connection's call answers the engine's result unchanged"
    );
    assert_eq!(
        [&played.failed, &played.reloaded, &played.clicked].map(|answer| answer.as_ref().ok()),
        [
            Some(&json!({ "requests": [] })),
            Some(&json!({ "response": null })),
            Some(&json!({})),
        ],
        "a value's call fills its own parameter"
    );
    assert_eq!(
        played.named.as_ref().ok(),
        Some(&json!({ "requests": [] })),
        "a parameter naming a second value takes its ID"
    );
    let Err(other_namespace) = &played.other_namespace else {
        panic!("{:?}", played.other_namespace);
    };
    assert_eq!(
        (
            other_namespace.code().as_str(),
            other_namespace.group().as_str(),
            other_namespace.retryable(),
            other_namespace.operation(),
        ),
        ("invalid_params", "program", false, Some("page.goto")),
        "an operation of another namespace fails before it is sent"
    );
    assert_eq!(
        code(&played.own_param).as_str(),
        "invalid_params",
        "the value's own parameter is the value's to fill"
    );
    assert_eq!(
        code(&played.deadline),
        ErrorCode::DEADLINE_EXCEEDED,
        "the timeout is sent as the operation's own, and its failure is the engine's"
    );
    assert_eq!(
        played.on_closed.each_ref().map(code),
        [ErrorCode::CLOSED; 3],
        "a call on a closed value fails with closed, unsent"
    );
}

/// The `raw-call` case through the async API.
/// Covers: story:11-181
#[test]
fn the_raw_call_reaches_operations_the_sdk_does_not_generate() {
    let env = TestEnv::new();
    let (process, _line) = scripted(&env, SCRIPT, &hello(SCRIPTED_DAEMON_TOKEN), "socket");

    let played = real_time_runtime().block_on(async {
        let collie = Collie::connect(daemon(&env)).await.unwrap();
        let listed = collie
            .call("run.list")
            .params(json!({ "lifecycle": "running", "tags": ["nightly"] }))
            .await;
        let run = collie.start().await.unwrap();
        let failed = run
            .call("run.failed_requests")
            .params(json!({ "blocked": true }))
            .await;
        let named = collie
            .call("run.failed_requests")
            .params(json!({ "run_id": run.id(), "blocked": false }))
            .await;
        let other_namespace = run
            .call("page.goto")
            .params(json!({ "url": "https://shop.test/" }))
            .await;
        let own_param = run
            .call("run.usage")
            .params(json!({ "run_id": "another" }))
            .await;
        let deadline = run
            .call("run.finish")
            .timeout(Duration::from_millis(2))
            .await;
        let page = run.new_page().await.unwrap();
        let reloaded = page.call("page.reload").await;
        let clicked = page.locator("#consent").call("locator.click").await;
        run.close().await.unwrap();
        let on_closed_run = run.call("run.usage").await;
        let on_closed_page = page.call("page.reload").await;
        collie.close().await.unwrap();
        let on_closed_connection = collie.call("run.list").await;
        Played {
            listed,
            failed,
            named,
            other_namespace,
            own_param,
            deadline,
            reloaded,
            clicked,
            on_closed: [on_closed_run, on_closed_page, on_closed_connection],
        }
    });

    played_whole(&process);
    assert_played(&played);
}

/// The same case through `blocking`, whose `call` ends in `send()`.
/// Covers: story:11-181
#[test]
fn the_blocking_raw_call_reaches_operations_the_sdk_does_not_generate() {
    let env = TestEnv::new();
    let (process, _line) = scripted(&env, SCRIPT, &hello(SCRIPTED_DAEMON_TOKEN), "socket");

    let collie = blocking::Collie::connect(daemon(&env)).unwrap();
    let listed = collie
        .call("run.list")
        .params(json!({ "lifecycle": "running", "tags": ["nightly"] }))
        .send();
    let run = collie.start().send().unwrap();
    let failed = run
        .call("run.failed_requests")
        .params(json!({ "blocked": true }))
        .send();
    let named = collie
        .call("run.failed_requests")
        .params(json!({ "run_id": run.id(), "blocked": false }))
        .send();
    let other_namespace = run
        .call("page.goto")
        .params(json!({ "url": "https://shop.test/" }))
        .send();
    let own_param = run
        .call("run.usage")
        .params(json!({ "run_id": "another" }))
        .send();
    let deadline = run
        .call("run.finish")
        .timeout(Duration::from_millis(2))
        .send();
    let page = run.new_page().send().unwrap();
    let reloaded = page.call("page.reload").send();
    let clicked = page.locator("#consent").call("locator.click").send();
    run.close().unwrap();
    let on_closed_run = run.call("run.usage").send();
    let on_closed_page = page.call("page.reload").send();
    collie.close().unwrap();
    let on_closed_connection = collie.call("run.list").send();

    played_whole(&process);
    assert_played(&Played {
        listed,
        failed,
        named,
        other_namespace,
        own_param,
        deadline,
        reloaded,
        clicked,
        on_closed: [on_closed_run, on_closed_page, on_closed_connection],
    });
}
