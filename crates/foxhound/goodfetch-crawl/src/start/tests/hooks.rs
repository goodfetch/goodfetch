//! A crawl's Hooks in code at the Record stage (ADR 0104): each Record an
//! attempt is about to commit goes to each `before_record_commit` Hook its
//! Label and Dataset reach, one `crawl_hook.handle` call at a time, under
//! `timeout.hook`; a Hook that raises or overruns fails the attempt, which
//! then commits nothing it staged and is retried as its failure says.

use goodfetch_engine::run_state::RunStateOptions;
use goodfetch_test_support::TestEnv;
use rstest::rstest;
use rusqlite::Connection;
use serde_json::{Value, json};

use super::{Client, Reply, crawl, dependencies, page_of, records, run_file, seeds, work_items};

/// Whether `params` are a Hook call's, not a handler request's.
fn is_hook(params: &Value) -> bool {
    params.get("hook").is_some()
}

/// Each failed attempt's number, Error code and details, null for none,
/// in order.
fn attempt_errors(connection: &Connection) -> Vec<(i64, String, Value)> {
    connection
        .prepare("SELECT attempt, code, details FROM attempt_errors ORDER BY work_item, attempt")
        .unwrap()
        .query_map([], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get::<_, Option<String>>(2)?
                    .map_or(Value::Null, |details| {
                        serde_json::from_str(&details).unwrap()
                    }),
            ))
        })
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

/// Starts the crawl `run_id` of one seed of the Label `page`, set as
/// `label`, under `settings`, with `hooks`, and finishes it, answering
/// every handler request and Hook call with `handler`.
async fn crawled(
    client: &mut Client,
    run_id: &str,
    (label, settings, hooks): (Value, Value, Value),
    mut handler: impl FnMut(&Value) -> Reply,
) {
    let start = json!({
        "run_id": run_id,
        "seeds": seeds(1),
        "labels": { "page": label },
        "settings": settings,
        "hooks": hooks,
    });
    let started = client.call("crawl.start", start, &mut handler).await;
    assert!(started.get("result").is_some(), "{started}");
    client
        .call("run.finish", json!({ "run_id": run_id }), &mut handler)
        .await;
}

/// The one Record-stage Hook `stamp`.
fn stamp() -> Value {
    json!([{ "name": "stamp", "stage": "before_record_commit" }])
}

/// Each Record the attempt is about to commit, those it pushed and then
/// those its handler answered, goes to each Record-stage Hook its Dataset
/// reaches, in the order declared, with the attempt it belongs to; one
/// scoped to another Dataset gets none.
///
/// Covers: story:3-168
#[test]
fn each_record_about_to_commit_goes_through_its_hooks_in_order() {
    let env = TestEnv::new();

    let hooked = crawl(
        || dependencies(&env, RunStateOptions::default()),
        async |client| {
            let hooks = json!([
                { "name": "stamp", "stage": "before_record_commit" },
                { "name": "reviews_only", "stage": "before_record_commit", "datasets": ["reviews"] },
                { "name": "sign", "stage": "before_record_commit", "labels": ["page"] },
            ]);
            crawled(client, "ordered", (json!({}), json!({}), hooks), |params| {
                if is_hook(params) {
                    return Reply::Records(Vec::new());
                }
                let push = json!({
                    "run_id": params["run_id"],
                    "crawl_request_id": params["crawl_request"]["id"],
                    "lease_epoch": params["lease_epoch"],
                    "dataset": "pages",
                    "records": [{ "n": 1 }],
                });
                Reply::Calls(
                    vec![("crawl_request.push", push)],
                    Box::new(Reply::Records(vec![json!({ "n": 2 })])),
                )
            })
            .await;
            client.hooked.clone()
        },
    );

    let calls: Vec<_> = hooked
        .iter()
        .map(|params| {
            (
                params["hook"].as_str().unwrap().to_owned(),
                params["stage"].clone(),
                params["record"].clone(),
                (params["attempt"].clone(), page_of(params)),
            )
        })
        .collect();
    let call = |hook: &str, n| {
        (
            hook.to_owned(),
            json!("before_record_commit"),
            json!({ "dataset": "pages", "data": { "n": n } }),
            (json!(1), "https://shop.test/page/1".to_owned()),
        )
    };
    let committed: Vec<_> = records(&run_file(&env, "ordered"))
        .into_iter()
        .map(|(_, _, _, data)| data)
        .collect();
    assert_eq!(
        (calls, committed),
        (
            vec![
                call("stamp", 1),
                call("sign", 1),
                call("stamp", 2),
                call("sign", 2)
            ],
            vec![json!({ "n": 1 }), json!({ "n": 2 })],
        )
    );
}

/// A Hook's call waits under its Label's own `timeout.hook`, lower than
/// the Run's: past it the call is cancelled and the attempt fails with
/// `deadline_exceeded`, naming the setting, the Label's value and the
/// stage it waited for, and is retried, its next attempt committing once.
///
/// Covers: story:3-169, setting:timeout.hook, error:deadline_exceeded
#[test]
fn a_hook_past_its_labels_timeout_fails_the_attempt_which_is_retried() {
    let env = TestEnv::new();

    let cancels = crawl(
        || dependencies(&env, RunStateOptions::default()),
        async |client| {
            let label = json!({ "settings": { "timeout": { "hook": 100 } } });
            let settings = json!({ "timeout": { "hook": 5_000 } });
            crawled(client, "slow", (label, settings, stamp()), |params| match (
                is_hook(params),
                params["attempt"].as_u64(),
            ) {
                (true, Some(1)) => Reply::Silent,
                _ => Reply::Records(vec![json!({ "attempt": params["attempt"] })]),
            })
            .await;
            client
                .arrivals
                .iter()
                .filter(|(message, _)| message["method"] == "$/cancel")
                .count()
        },
    );

    let file = run_file(&env, "slow");
    let (_, code, details) = attempt_errors(&file).remove(0);
    assert_eq!(
        (
            cancels,
            code,
            details["limit"].clone(),
            details["waited_for"].clone(),
            records(&file).len(),
            work_items(&file),
        ),
        (
            1,
            "deadline_exceeded".to_owned(),
            json!({ "key": "timeout.hook", "value": 100, "layer": "run", "origin": "settings" }),
            json!("answer of a `before_record_commit` Hook"),
            1,
            vec![("1".to_owned(), "succeeded".to_owned(), 2, None)],
        )
    );
}

/// A Hook that raises fails the attempt with `hook_failed`, naming the
/// stage, the Hook and the Label with the program's exception. Its own
/// exception, a bug, is retried only where `crawl.retry.handler_failed`
/// says, a Label's own value before the Run's.
///
/// Covers: story:3-170, error:hook_failed, setting:crawl.retry.handler_failed
#[rstest]
#[case::not_retried_by_default(json!({}), ("failed", 1, Some("hook_failed")))]
#[case::retried_where_its_label_says(
    json!({ "settings": { "crawl": { "retry": { "handler_failed": true } } } }),
    ("succeeded", 2, None)
)]
fn a_hooks_own_exception_is_retried_only_where_its_label_says(
    #[case] label: Value,
    #[case] (status, attempts, code): (&str, i64, Option<&str>),
) {
    let env = TestEnv::new();

    crawl(
        || dependencies(&env, RunStateOptions::default()),
        async |client| {
            crawled(client, "raises", (label, json!({}), stamp()), |params| {
                match (is_hook(params), params["attempt"].as_u64()) {
                    (true, Some(1)) => Reply::FailsWith(
                        "hook_failed",
                        true,
                        json!({ "exception_type": "TypeError", "exception_message": "no price", "exception_stack": "at stamp" }),
                    ),
                    _ => Reply::Records(vec![json!({ "attempt": params["attempt"] })]),
                }
            })
            .await;
        },
    );

    let file = run_file(&env, "raises");
    assert_eq!(
        (
            attempt_errors(&file),
            records(&file).len(),
            work_items(&file)
        ),
        (
            vec![(
                1,
                "hook_failed".to_owned(),
                json!({
                    "stage": "before_record_commit",
                    "hook": "stamp",
                    "label": "page",
                    "exception_type": "TypeError",
                    "exception_message": "no price",
                    "exception_stack": "at stamp",
                    "wrapped_code": null,
                }),
            )],
            usize::try_from(attempts - 1).unwrap(),
            vec![(
                "1".to_owned(),
                status.to_owned(),
                attempts,
                code.map(str::to_owned)
            )],
        )
    );
}

/// A Goodfetch Error code raised inside a Hook is retried as its own
/// retryable flag says, and draws on `crawl.retry.max` with the handler's
/// failures: a handler failure and then two Hook failures use the Run's two
/// retries up.
///
/// Covers: story:3-170, setting:crawl.retry.max
#[test]
fn a_hooks_wrapped_code_is_retried_by_its_flag_on_the_runs_retries() {
    let env = TestEnv::new();

    crawl(
        || dependencies(&env, RunStateOptions::default()),
        async |client| {
            crawled(client, "wrapped", (json!({}), json!({}), stamp()), |params| {
                match (is_hook(params), params["attempt"].as_u64()) {
                    (false, Some(1)) => Reply::Fails("retry_requested", true),
                    (false, _) => Reply::Records(vec![json!({ "attempt": params["attempt"] })]),
                    (true, _) => Reply::FailsWith(
                        "hook_failed",
                        true,
                        json!({ "exception_type": "GoodfetchError", "exception_message": "busy", "wrapped_code": "retry_requested" }),
                    ),
                }
            })
            .await;
        },
    );

    let file = run_file(&env, "wrapped");
    let errors: Vec<_> = attempt_errors(&file)
        .into_iter()
        .map(|(attempt, code, details)| (attempt, code, details["wrapped_code"].clone()))
        .collect();
    assert_eq!(
        (errors, records(&file), work_items(&file)),
        (
            vec![
                (1, "retry_requested".to_owned(), Value::Null),
                (2, "hook_failed".to_owned(), json!("retry_requested")),
                (3, "hook_failed".to_owned(), json!("retry_requested")),
            ],
            Vec::new(),
            vec![(
                "1".to_owned(),
                "failed".to_owned(),
                3,
                Some("hook_failed".to_owned())
            )],
        )
    );
}

/// A Record-stage Hook that raises discards everything its attempt staged:
/// the Records it pushed and answered, its enqueue and its Key-value store
/// write commit nothing, and the retried attempt, which stages none of
/// them, commits only its own Record.
///
/// Covers: story:3-171
#[test]
fn a_failed_record_stage_commits_nothing_its_attempt_staged() {
    let env = TestEnv::new();

    crawl(
        || dependencies(&env, RunStateOptions::default()),
        async |client| {
            crawled(client, "discard", (json!({}), json!({}), stamp()), |params| {
                let attempt = params["attempt"].clone();
                if is_hook(params) {
                    return if attempt == 1 {
                        Reply::Fails("retry_requested", true)
                    } else {
                        Reply::Records(Vec::new())
                    };
                }
                if attempt != 1 {
                    return Reply::Records(vec![json!({ "attempt": attempt })]);
                }
                let run_id = &params["run_id"];
                let id = &params["crawl_request"]["id"];
                let epoch = &params["lease_epoch"];
                let staged = vec![
                    (
                        "crawl_request.push",
                        json!({ "run_id": run_id, "crawl_request_id": id, "lease_epoch": epoch, "dataset": "pages", "records": [{ "pushed": 1 }] }),
                    ),
                    (
                        "crawl_request.enqueue",
                        json!({ "run_id": run_id, "crawl_request_id": id, "lease_epoch": epoch, "url": "https://shop.test/discarded", "label": "page" }),
                    ),
                    (
                        "key_value_store.set",
                        json!({ "run_id": run_id, "key": "discarded", "value": 1, "attempt": { "crawl_request_id": id, "lease_epoch": epoch } }),
                    ),
                ];
                Reply::Calls(staged, Box::new(Reply::Records(vec![json!({ "attempt": 1 })])))
            })
            .await;
        },
    );

    let file = run_file(&env, "discard");
    let kept: i64 = file
        .query_row("SELECT COUNT(*) FROM key_values", [], |row| row.get(0))
        .unwrap();
    let committed: Vec<_> = records(&file)
        .into_iter()
        .map(|(_, _, _, data)| data)
        .collect();
    let items: Vec<_> = work_items(&file)
        .into_iter()
        .map(|(_, status, attempts, _)| (status, attempts))
        .collect();
    assert_eq!(
        (committed, kept, items),
        (
            vec![json!({ "attempt": 2 })],
            0,
            vec![("succeeded".to_owned(), 2)],
        )
    );
}

/// Each request and page stage Hook and handler request the engine sent,
/// in order, as `(method, hook, stage, attempt)`: the hook and stage are
/// null on a handler request.
fn stages_sent(client: &Client) -> Vec<(String, Value, Value, Value)> {
    client
        .arrivals
        .iter()
        .map(|(message, _)| message)
        .filter(|message| {
            message["method"] == "crawl_hook.handle" || message["method"] == "crawl_request.handle"
        })
        .map(|message| {
            let params = &message["params"];
            (
                message["method"].as_str().unwrap().to_owned(),
                params["hook"].clone(),
                params["stage"].clone(),
                params["attempt"].clone(),
            )
        })
        .collect()
}

/// A Hook call of `hook` at `stage` on attempt `attempt`, as
/// [`stages_sent`] lists it.
fn hook_call(hook: &str, stage: &str, attempt: u32) -> (String, Value, Value, Value) {
    (
        "crawl_hook.handle".to_owned(),
        json!(hook),
        json!(stage),
        json!(attempt),
    )
}

/// The handler request of attempt `attempt`, as [`stages_sent`] lists it.
fn handler_call(attempt: u32) -> (String, Value, Value, Value) {
    (
        "crawl_request.handle".to_owned(),
        Value::Null,
        Value::Null,
        json!(attempt),
    )
}

/// On the `http` tier, a request goes through its before-request Hooks,
/// then its after-response Hooks once its document arrived, then its
/// page-ready Hooks just before its handler: each Hook its Label reaches
/// called once, in the order declared, before the next stage. A Hook
/// scoped to another Label, and a navigation Hook on a tier that does not
/// navigate, are sent nothing.
///
/// Covers: story:3-168
#[test]
fn a_requests_hooks_run_per_stage_in_the_order_declared_before_its_handler() {
    let env = TestEnv::new();

    let sent = crawl(
        || dependencies(&env, RunStateOptions::default()),
        async |client| {
            let start = json!({
                "run_id": "staged",
                "seeds": seeds(1),
                "labels": { "page": {}, "detail": {} },
                "hooks": [
                    { "name": "sign", "stage": "before_request" },
                    { "name": "detail_only", "stage": "before_request", "labels": ["detail"] },
                    { "name": "look", "stage": "after_response" },
                    { "name": "steer", "stage": "before_navigation" },
                    { "name": "ready", "stage": "page_ready", "labels": ["page"] },
                    { "name": "seal", "stage": "before_request", "labels": ["page"] },
                    { "name": "landed", "stage": "after_navigation" },
                ],
            });
            let mut handler = |_: &Value| Reply::Records(vec![json!({ "n": 1 })]);
            let started = client.call("crawl.start", start, &mut handler).await;
            assert!(started.get("result").is_some(), "{started}");
            client
                .call("run.finish", json!({ "run_id": "staged" }), &mut handler)
                .await;
            stages_sent(client)
        },
    );

    assert_eq!(
        (sent, records(&run_file(&env, "staged")).len()),
        (
            vec![
                hook_call("sign", "before_request", 1),
                hook_call("seal", "before_request", 1),
                hook_call("look", "after_response", 1),
                hook_call("ready", "page_ready", 1),
                handler_call(1),
            ],
            1,
        )
    );
}

/// Before request is also the stage before a retry: an attempt the
/// handler failed is fetched again after its before-request Hook ran once
/// more, with the new attempt's number.
///
/// Covers: story:3-168
#[test]
fn a_before_request_hook_runs_again_before_each_retry() {
    let env = TestEnv::new();

    let sent = crawl(
        || dependencies(&env, RunStateOptions::default()),
        async |client| {
            let hooks = json!([{ "name": "sign", "stage": "before_request" }]);
            crawled(
                client,
                "retried",
                (json!({}), json!({}), hooks),
                |params| match (is_hook(params), params["attempt"].as_u64()) {
                    (false, Some(1)) => Reply::Fails("retry_requested", true),
                    _ => Reply::Records(vec![json!({ "attempt": params["attempt"] })]),
                },
            )
            .await;
            stages_sent(client)
        },
    );

    assert_eq!(
        sent,
        vec![
            hook_call("sign", "before_request", 1),
            handler_call(1),
            hook_call("sign", "before_request", 2),
            handler_call(2),
        ]
    );
}

/// A Hook at a request or page stage that raises fails its attempt with
/// `hook_failed`, naming its stage and the Hook, and the handler is never
/// sent the request.
///
/// Covers: story:3-170, error:hook_failed
#[rstest]
#[case::before_request("before_request")]
#[case::after_response("after_response")]
#[case::page_ready("page_ready")]
fn a_request_stage_hook_that_raises_fails_its_attempt_before_the_handler(#[case] stage: &str) {
    let env = TestEnv::new();

    let sent = crawl(
        || dependencies(&env, RunStateOptions::default()),
        async |client| {
            let hooks = json!([{ "name": "check", "stage": stage }]);
            crawled(client, "raises", (json!({}), json!({}), hooks), |params| {
                if is_hook(params) {
                    Reply::FailsWith(
                        "hook_failed",
                        true,
                        json!({ "exception_type": "TypeError", "exception_message": "no token" }),
                    )
                } else {
                    Reply::Records(vec![json!({ "n": 1 })])
                }
            })
            .await;
            stages_sent(client)
        },
    );

    let file = run_file(&env, "raises");
    let errors: Vec<_> = attempt_errors(&file)
        .into_iter()
        .map(|(attempt, code, details)| {
            (
                attempt,
                code,
                details["stage"].clone(),
                details["hook"].clone(),
            )
        })
        .collect();
    assert_eq!(
        (sent, errors, records(&file).len()),
        (
            vec![hook_call("check", stage, 1)],
            vec![(1, "hook_failed".to_owned(), json!(stage), json!("check"))],
            0,
        )
    );
}

/// Answers a Hook's call with no change and a handler request with its
/// page's Record.
fn answer_hooked(params: &Value) -> Reply {
    if is_hook(params) {
        Reply::Records(Vec::new())
    } else {
        Reply::Records(vec![json!({ "page": page_of(params) })])
    }
}

/// The Error code and details of the refusal `answer`.
fn refusal(answer: &Value) -> (Value, Value) {
    let mut details = answer["error"]["data"].clone();
    let code = details["code"].take();
    if let Some(fields) = details.as_object_mut() {
        fields.retain(|field, _| field != "code" && field != "retryable");
    }
    (code, details)
}

/// `hook_undeclared`'s code and details for the Run `run_id` whose crawl
/// declared the Hook [`stamp`].
fn stamp_undeclared(run_id: &str) -> (Value, Value) {
    (
        json!("hook_undeclared"),
        json!({
            "run_id": run_id,
            "hooks": [{ "name": "stamp", "stage": "before_record_commit" }],
        }),
    )
}

/// The start under the ID `run_id` of the one-seed crawl of the Label
/// `page` with the Hook [`stamp`].
fn stamped_start(run_id: &str) -> Value {
    json!({
        "run_id": run_id,
        "seeds": seeds(1),
        "labels": { "page": {} },
        "hooks": stamp(),
    })
}

/// `run.retry_failed` alone never reopens an ended crawl whose start
/// declared Hooks in code, which would commit Records its Hooks never saw:
/// it is refused with `hook_undeclared`, naming the Hooks, and nothing is
/// re-admitted. Once a start under its ID declares them again, the retry
/// reopens it and its Record goes through them (ADR 0104's note of #2912).
///
/// Covers: error:hook_undeclared, operation:run.retry_failed
#[test]
fn a_retry_of_a_crawl_with_hooks_waits_for_a_start_to_declare_them() {
    let env = TestEnv::new();

    let (refused, before, after, retried, hooked) = crawl(
        || dependencies(&env, RunStateOptions::default()),
        async |client| {
            crawled(
                client,
                "retried",
                (json!({}), json!({}), stamp()),
                |params| {
                    if is_hook(params) {
                        Reply::Records(Vec::new())
                    } else {
                        Reply::Fails("handler_failed", false)
                    }
                },
            )
            .await;
            let retry = json!({ "run_id": "retried" });
            let before = work_items(&run_file(&env, "retried"));
            let refused = client
                .call("run.retry_failed", retry.clone(), &mut answer_hooked)
                .await;
            let after = work_items(&run_file(&env, "retried"));
            client
                .call("crawl.start", stamped_start("retried"), &mut answer_hooked)
                .await;
            let retried = client
                .call("run.retry_failed", retry.clone(), &mut answer_hooked)
                .await;
            client.call("run.finish", retry, &mut answer_hooked).await;
            let hooked: Vec<_> = client
                .hooked
                .iter()
                .map(|params| params["hook"].clone())
                .collect();
            (refused, before, after, retried, hooked)
        },
    );

    assert_eq!(
        (
            refusal(&refused),
            after,
            retried["result"].clone(),
            hooked,
            records(&run_file(&env, "retried")).len(),
        ),
        (
            stamp_undeclared("retried"),
            before,
            json!({ "readmitted": 1 }),
            vec![json!("stamp")],
            1,
        ),
        "{refused}\n{retried}"
    );
}

/// `run.resume` alone never reopens a stopped crawl whose start declared
/// Hooks in code: it is refused with `hook_undeclared`, naming the Hooks,
/// and the Run stays stopped. A start under its ID resumes it, and its
/// Record goes through them (ADR 0104's note of #2912).
///
/// Covers: error:hook_undeclared, operation:run.resume
#[test]
fn a_resume_of_a_crawl_with_hooks_waits_for_a_start_to_declare_them() {
    let env = TestEnv::new();

    let (refused, stopped, finished, hooked) = crawl(
        || dependencies(&env, RunStateOptions::default()),
        async |client| {
            let run = json!({ "run_id": "paused" });
            client
                .call("crawl.start", stamped_start("paused"), &mut |_| {
                    Reply::Silent
                })
                .await;
            client
                .call("run.pause", run.clone(), &mut |_| Reply::Silent)
                .await;
            let refused = client
                .call("run.resume", run.clone(), &mut answer_hooked)
                .await;
            let stopped = client
                .call("run.report", run.clone(), &mut answer_hooked)
                .await;
            client
                .call("crawl.start", stamped_start("paused"), &mut answer_hooked)
                .await;
            let finished = client.call("run.finish", run, &mut answer_hooked).await;
            let hooked: Vec<_> = client
                .hooked
                .iter()
                .map(|params| params["hook"].clone())
                .collect();
            (refused, stopped, finished, hooked)
        },
    );

    assert_eq!(
        (
            refusal(&refused),
            stopped["result"]["lifecycle"].clone(),
            finished["result"]["lifecycle"].clone(),
            hooked,
            records(&run_file(&env, "paused")).len(),
        ),
        (
            stamp_undeclared("paused"),
            json!("stopped"),
            json!("finished"),
            vec![json!("stamp")],
            1,
        ),
        "{refused}\n{stopped}"
    );
}
