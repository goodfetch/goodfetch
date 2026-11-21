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

/// A start declaring a Hook at a stage this engine does not run Hooks in
/// code at yet is refused with `invalid_params`, before the Run begins.
///
/// Covers: story:3-168
#[test]
fn a_start_declaring_a_hook_at_a_stage_not_run_yet_is_refused() {
    let env = TestEnv::new();

    let refused = crawl(
        || dependencies(&env, RunStateOptions::default()),
        async |client| {
            let start = json!({
                "run_id": "early",
                "seeds": seeds(1),
                "labels": { "page": {} },
                "hooks": [{ "name": "sign", "stage": "before_request" }],
            });
            client
                .call("crawl.start", start, &mut |_| Reply::Silent)
                .await
        },
    );

    assert_eq!(
        (
            refused["error"]["data"]["code"].clone(),
            env.home().join("1").join("runs").join("early").exists()
        ),
        (json!("invalid_params"), false)
    );
}
