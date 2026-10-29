//! A Run's report as the engine assembles it, through its public interface
//! over the in-memory network (spec #213's Seam 3, ADR 0102): `run.finish`
//! and `run.stop` answering it, `run.stop`'s wait bounded by
//! `timeout.close`, a timeout on a wait ending only the wait, a `failed`
//! Run raising its error, each stop reason with its details, every part an
//! area supplies through its composition value and none it did not, the
//! warnings handed over, `nothingDone` and `run.usage`. A test's own
//! operations act on a live Run as an area does: `test.hand_over` charges
//! Usage, hands over warnings and `nothingDone`, asks a stop or fails the
//! Run; `test.drive` starts a Run its driver ends only once stopped, after
//! a minute of its clock when `slow`; `test.cancelled_stop` drops a
//! `run.stop` at its first await, as a Client's cancel does.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use futures_util::FutureExt;
use goodfetch_protocol::catalog::{
    Operation, OperationClass, OperationKind, Product, Reach, Stability, schema_of,
};
use goodfetch_protocol::run::{
    AxisReason, EndingError, NothingDone, PlanReport, PlanStatus, PluginRun, RUN_STOP,
    RecordingReport, RunId, RunKind, RunReport, RunWarning, StopReason, Tags, UncertainEffect,
    Verdict,
};
use goodfetch_test_support::TestEnv;
use rstest::rstest;
use rusqlite::Connection;
use serde_json::{Value, json};

use super::run::{call, run_folder};
use super::{Client, attached_on, engine_with, hello};
use crate::composition::{CallContext, CallError, Contribution, Handler};
use crate::runs::{EndedBy, Ending, ReportPart, Start, Started};

/// Acts on a live Run as an area does.
const HAND_OVER: Operation = Operation {
    name: "test.hand_over",
    description: "Charges Usage, hands over warnings and `nothingDone`, stops or fails a live Run.",
    class: OperationClass::Effect,
    kind: OperationKind::RoundTrip,
    reach: Reach::AllSurfaces,
    params: schema_of::<Value>,
    result: schema_of::<Value>,
    events: &[],
    cancellable: false,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: goodfetch_protocol::catalog::PROTOCOL_VERSION,
};

/// Starts a Run its driver ends only once something stops it.
const DRIVE: Operation = Operation {
    name: "test.drive",
    description: "Starts a Run its driver ends once stopped.",
    ..HAND_OVER
};

/// Starts a `run.stop` and drops it at its first await.
const CANCELLED_STOP: Operation = Operation {
    name: "test.cancelled_stop",
    description: "Drops a `run.stop` at its first await.",
    ..HAND_OVER
};

async fn hand_over(params: Value, context: CallContext) -> Result<Value, CallError> {
    let run_id = RunId::parse(params["run_id"].as_str().unwrap()).unwrap();
    let run = context.runs().live(run_id.clone()).await.unwrap();
    for charge in params["charges"].as_array().into_iter().flatten() {
        let (usage_kind, amount) = (
            charge["usage_kind"].as_str().unwrap(),
            charge["amount"].as_u64().unwrap(),
        );
        let price = charge["price"]["currency"]
            .as_str()
            .map(|currency| (currency, charge["price"]["amount"].as_u64().unwrap()));
        match charge["part"].as_str() {
            Some(part) => run.charge_part(usage_kind, part, amount, price).await.ok(),
            None => run.charge(usage_kind, amount).await.ok(),
        };
    }
    if let Some(reserve) = params.get("reserve") {
        let reservation = run
            .reserve(
                reserve["usage_kind"].as_str().unwrap(),
                reserve["currency"].as_str(),
                reserve["worst"].as_u64().unwrap(),
            )
            .await
            .unwrap();
        run.settle(reservation, reserve["cost"].as_u64().unwrap())
            .await
            .unwrap();
    }
    for warning in params["warnings"].as_array().into_iter().flatten() {
        let warning: RunWarning = serde_json::from_value(warning.clone()).unwrap();
        run.warn(warning).await.unwrap();
    }
    if let Some(nothing_done) = params.get("nothing_done") {
        let nothing_done: NothingDone = serde_json::from_value(nothing_done.clone()).unwrap();
        run.nothing_done(nothing_done).await.unwrap();
    }
    let ends = if let Some(error) = params.get("fail") {
        let error: EndingError = serde_json::from_value(error.clone()).unwrap();
        run.end(Ending::Failed(error)).await;
        true
    } else if let Some(reason) = params.get("stop") {
        let reason: StopReason = serde_json::from_value(reason.clone()).unwrap();
        run.stop(reason);
        true
    } else {
        false
    };
    // The Run's end waits for every holder of its writer, as a driver lets
    // its Run go once it ends.
    drop(run);
    if ends {
        context.runs().ended(run_id).await.ok();
    }
    Ok(json!({}))
}

async fn drive(params: Value, context: CallContext) -> Result<Value, CallError> {
    let started = context
        .runs()
        .start(Start {
            run_id: Some(RunId::parse(params["run_id"].as_str().unwrap()).unwrap()),
            tags: Tags::default(),
            kind: RunKind::Collie,
            needs: Product::Collie,
            inputs: BTreeMap::new(),
            ended_by: EndedBy::Driver,
            initialize: None,
            policy_version: None,
            weakened: Vec::new(),
            binding: None,
            clean_up_digest: None,
            checkpoint_interval: None,
            timeout: None,
            settings: None,
        })
        .await
        .map_err(crate::run::start_error)?;
    let Started::New(run) = started else {
        panic!("the Run began before");
    };
    let slow = params["slow"].as_bool() == Some(true);
    let driver = run.clone();
    run.spawn(async move {
        let stopping = async {
            let reason = driver.stopping().await;
            if slow {
                driver.clock().sleep(Duration::from_mins(1)).await;
            }
            driver.end(Ending::Stopped(reason)).await;
        };
        tokio::select! {
            () = driver.cancellation().cancelled() => {}
            () = stopping => {}
        }
    });
    Ok(json!({ "run_id": run.id() }))
}

/// Polls the engine's own `run.stop` of `params` once, as its call's task
/// does before it sees its Client's cancel, then drops it; answers whether
/// it answered at once.
async fn cancelled_stop(params: Value, context: CallContext) -> Result<Value, CallError> {
    let stop = crate::run::handlers()
        .into_iter()
        .find(|handler| handler.operation().name == RUN_STOP.name)
        .expect("invariant: the engine handles `run.stop`");
    let answered = stop.call(params, context).now_or_never();
    Ok(json!({ "answered": answered.is_some() }))
}

/// What areas not built yet supply, for a Run whose ID begins `seeded`:
/// the Plan's report, the Plugins, the recording, the uncertain Effects
/// that make it `needs_review`, and a kind's part; for `unwritable`, a
/// Plan file whose write at the Run's end failed (ADR 0116).
struct Seeded;

impl ReportPart for Seeded {
    fn fill(&self, _: &Connection, report: &mut RunReport) -> rusqlite::Result<()> {
        if report.run_id.as_str() == "unwritable" {
            report.warnings.push(RunWarning {
                code: "path_not_writable".to_owned(),
                message: "The Plan file `plans/shop.plan` could not be written.".to_owned(),
                sessions: Vec::new(),
                folder: Some("plans".to_owned()),
            });
            report.error = Some(EndingError {
                code: "path_not_writable".to_owned(),
                message: "The Plan file `plans/shop.plan` could not be written.".to_owned(),
                details: serde_json::Map::new(),
            });
        }
        if !report.run_id.as_str().starts_with("seeded") {
            return Ok(());
        }
        report.plan = Some(PlanReport {
            file: Some("plans/shop.plan".to_owned()),
            version: Some(3),
            status: PlanStatus::PendingReview,
            steps_changed: vec!["checkout".to_owned()],
        });
        report.plugins = Some(vec![PluginRun {
            name: "sink-s3".to_owned(),
            network_level: "egress".to_owned(),
            confinement_missing: vec!["seccomp".to_owned()],
        }]);
        report.recording = Some(RecordingReport {
            mode: "replay".to_owned(),
            file: "shop.recording".to_owned(),
            responses_used: 4,
            recorded_from: None,
            recorded_to: None,
            written_by: None,
            misses: Vec::new(),
        });
        report.uncertain_effects = Some(vec![UncertainEffect {
            entry: "e1".to_owned(),
            key: Some("order-1".to_owned()),
            instruction: None,
            crawl_request: None,
            reason: "sent".to_owned(),
            first_command: Some("Input.dispatchMouseEvent".to_owned()),
            since: report.started_at.clone(),
        }]);
        report.verdict = Verdict::NeedsReview;
        report.verdict_reasons.push(AxisReason {
            code: "uncertain_effects".to_owned(),
            message: "An Effect is uncertain.".to_owned(),
            count: None,
            call_id: None,
        });
        report
            .kind_parts
            .insert("requests".to_owned(), json!({ "succeeded": 1 }));
        Ok(())
    }
}

fn reporting() -> Contribution {
    Contribution::new("test-report")
        .with_handler(Handler::new(&HAND_OVER, hand_over))
        .with_handler(Handler::new(&DRIVE, drive))
        .with_handler(Handler::new(&CANCELLED_STOP, cancelled_stop))
        .with_report_part(Arc::new(Seeded))
}

/// Writes `text` as `file` of the test home's per-major folder.
#[expect(
    clippy::disallowed_methods,
    reason = "the test writes the Goodfetch home's settings file itself"
)]
fn write(env: &TestEnv, file: &str, text: &str) {
    let folder = env.home().join("1");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join(file), text).unwrap();
}

/// Runs a Collie engine with the test's operations in `env` while the
/// Client makes `calls`, after its `hello`, and answers each reply.
fn replies(env: &TestEnv, calls: &[(&str, Value)]) -> Vec<Value> {
    let engine = engine_with(Product::Collie, Some(reporting()));
    let calls: Vec<(String, Value)> = calls
        .iter()
        .map(|(method, params)| ((*method).to_owned(), params.clone()))
        .collect();
    attached_on(&engine, env, |mut client: Client| async move {
        client.send(&[hello("collie", "0.1")]).await;
        client.next().await;
        let mut replies = Vec::new();
        for (id, (method, params)) in (1..).zip(calls) {
            replies.push(call(&mut client, id, &method, params).await);
        }
        replies
    })
}

/// The keys of a report, or of one of its parts.
fn keys(value: &Value) -> Vec<&str> {
    let mut keys: Vec<&str> = value
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    keys
}

/// Covers: story:3-153, operation:run.finish
#[test]
fn finish_ends_a_collie_run_at_once_finished_and_complete() {
    let env = TestEnv::new();

    let replies = replies(
        &env,
        &[
            ("run.start", json!({ "run_id": "done" })),
            ("run.finish", json!({ "run_id": "done" })),
        ],
    );

    let report = &replies[1]["result"];
    assert_eq!(
        (
            &report["lifecycle"],
            &report["completeness"],
            &report["stop_reason"]
        ),
        (&json!("finished"), &json!("complete"), &Value::Null)
    );
}

/// Covers: story:3-153, operation:run.stop
#[test]
fn stop_stops_a_run_with_a_checkpoint_as_an_abort_by_its_program() {
    let env = TestEnv::new();

    let replies = replies(
        &env,
        &[
            ("run.start", json!({ "run_id": "stopped" })),
            ("run.stop", json!({ "run_id": "stopped" })),
        ],
    );

    let report = &replies[1]["result"];
    assert_eq!(
        (
            &report["lifecycle"],
            &report["completeness"],
            &report["stop_reason"],
            &report["checkpoint"]["run_epoch"],
            run_folder(&env, "stopped")
                .join("checkpoint.sqlite")
                .exists(),
        ),
        (
            &json!("stopped"),
            &json!("incomplete"),
            &json!({ "kind": "abort", "by": "program" }),
            &json!(1),
            true,
        )
    );
}

/// A stop whose Run does not reach its safe point within its wait fails
/// the wait with `deadline_exceeded`, naming `timeout.close` as the default
/// or the call's own `timeout` set it; the Run still stops there.
/// Covers: story:3-153, setting:timeout.close, error:deadline_exceeded
#[rstest]
#[case::by_default(json!({ "run_id": "slow" }), json!({ "key": "timeout.close", "value": 30_000, "layer": "default" }))]
#[case::by_the_calls_timeout(json!({ "run_id": "slow", "timeout": 1_000 }), json!({ "key": "timeout.close", "value": 1_000, "layer": "run" }))]
fn a_stop_past_its_wait_fails_the_wait_and_the_run_still_stops(
    #[case] stop: Value,
    #[case] limit: Value,
) {
    let env = TestEnv::new();

    let replies = replies(
        &env,
        &[
            ("test.drive", json!({ "run_id": "slow", "slow": true })),
            ("run.stop", stop),
            ("test.sleep", json!({})),
            ("run.stop", json!({ "run_id": "slow" })),
        ],
    );

    assert_eq!(
        (
            &replies[1]["error"]["data"]["code"],
            &replies[1]["error"]["data"]["limit"],
            &replies[3]["result"]["stop_reason"],
        ),
        (
            &json!("deadline_exceeded"),
            &limit,
            &json!({ "kind": "abort", "by": "program" }),
        )
    );
}

/// A `run.stop` its Client cancels at its first await, before the Run's
/// `timeout.close` is read, still stops the Run: only its wait ends with the
/// call (#1743). The test calls `run.stop`'s handler itself, so no
/// dispatch records the operation.
/// Covers: story:3-153
#[test]
fn a_stop_cancelled_at_its_first_await_still_stops_the_run() {
    let env = TestEnv::new();

    let replies = replies(
        &env,
        &[
            ("test.drive", json!({ "run_id": "cancelled" })),
            ("test.cancelled_stop", json!({ "run_id": "cancelled" })),
            (
                "run.finish",
                json!({ "run_id": "cancelled", "timeout": 60_000 }),
            ),
        ],
    );

    assert_eq!(
        (
            &replies[1]["result"]["answered"],
            &replies[2]["result"]["stop_reason"],
        ),
        (&json!(false), &json!({ "kind": "abort", "by": "program" }))
    );
}

/// Covers: story:3-153, error:deadline_exceeded
#[test]
fn a_timeout_on_finish_ends_only_the_wait() {
    let env = TestEnv::new();

    let replies = replies(
        &env,
        &[
            ("test.drive", json!({ "run_id": "going" })),
            ("run.finish", json!({ "run_id": "going", "timeout": 1_000 })),
            ("run.abort", json!({ "run_id": "going" })),
        ],
    );

    assert_eq!(
        (
            &replies[1]["error"]["data"]["code"],
            &replies[2]["result"]["stop_reason"],
        ),
        (
            &json!("deadline_exceeded"),
            &json!({ "kind": "abort", "by": "client" }),
        )
    );
}

/// Covers: story:3-153, error:run_state_unavailable
#[rstest]
#[case::finish("run.finish")]
#[case::stop("run.stop")]
fn a_failed_run_makes_its_ending_call_raise_its_error(#[case] ending: &str) {
    let env = TestEnv::new();
    let error = json!({
        "code": "run_state_unavailable",
        "message": "The crawl's work items could not be read.",
        "details": { "run_id": "broken" },
    });

    let replies = replies(
        &env,
        &[
            ("test.drive", json!({ "run_id": "broken" })),
            (
                "test.hand_over",
                json!({ "run_id": "broken", "fail": error }),
            ),
            (ending, json!({ "run_id": "broken" })),
            ("run.abort", json!({ "run_id": "broken" })),
        ],
    );

    assert_eq!(
        (
            &replies[2]["error"]["data"]["code"],
            &replies[2]["error"]["message"],
            &replies[3]["result"]["lifecycle"],
        ),
        (
            &json!("run_state_unavailable"),
            &json!("The crawl's work items could not be read."),
            &json!("failed"),
        )
    );
}

/// Covers: story:3-153, story:3-155, error:path_not_writable
#[test]
fn a_write_that_failed_at_the_runs_end_raises_its_code_once_the_run_ended() {
    let env = TestEnv::new();

    let replies = replies(
        &env,
        &[
            ("run.start", json!({ "run_id": "unwritable" })),
            ("run.finish", json!({ "run_id": "unwritable" })),
            ("run.abort", json!({ "run_id": "unwritable" })),
        ],
    );

    assert_eq!(
        (
            &replies[1]["error"]["data"]["code"],
            &replies[2]["result"]["lifecycle"],
            &replies[2]["result"]["warnings"][0]["code"],
        ),
        (
            &json!("path_not_writable"),
            &json!("finished"),
            &json!("path_not_writable"),
        )
    );
}

/// Covers: story:3-154
#[rstest]
#[case::a_limit(
    vec![
        ("usage.set", json!({ "scope": "install", "usage_kind": "requests", "period": { "every": "month", "time_zone": "UTC" }, "amount": 1 })),
        ("run.start", json!({ "run_id": "stops" })),
        ("test.hand_over", json!({ "run_id": "stops", "charges": [{ "usage_kind": "requests", "amount": 1 }, { "usage_kind": "requests", "amount": 1 }] })),
    ],
    json!({ "kind": "limit", "scope": { "kind": "install", "id": "" }, "usage_kind": "requests", "period": { "every": "month", "time_zone": "UTC" }, "resets_at": "2026-02-01T00:00:00Z" }),
)]
#[case::its_timeout(
    vec![
        ("run.start", json!({ "run_id": "stops", "settings": { "timeout": { "run": 1_000 } } })),
        ("test.sleep", json!({})),
    ],
    json!({ "kind": "timeout" }),
)]
#[case::an_abort(
    vec![
        ("run.start", json!({ "run_id": "stops" })),
        ("run.abort", json!({ "run_id": "stops" })),
    ],
    json!({ "kind": "abort", "by": "client" }),
)]
#[case::the_engines_loss(
    vec![
        ("run.start", json!({ "run_id": "stops" })),
        ("test.hand_over", json!({ "run_id": "stops", "stop": { "kind": "engine_lost" } })),
    ],
    json!({ "kind": "engine_lost" }),
)]
#[case::storage(
    vec![
        ("run.start", json!({ "run_id": "stops" })),
        ("test.hand_over", json!({ "run_id": "stops", "stop": { "kind": "resources", "resource": "storage", "folder": "/data/runs" } })),
    ],
    json!({ "kind": "resources", "resource": "storage", "folder": "/data/runs" }),
)]
#[case::memory(
    vec![
        ("run.start", json!({ "run_id": "stops" })),
        ("test.hand_over", json!({ "run_id": "stops", "stop": { "kind": "resources", "resource": "memory" } })),
    ],
    json!({ "kind": "resources", "resource": "memory" }),
)]
fn each_stop_gives_its_stop_reason_with_its_details(
    #[case] mut calls: Vec<(&str, Value)>,
    #[case] stop_reason: Value,
) {
    let env = TestEnv::new();
    calls.push(("run.finish", json!({ "run_id": "stops" })));

    let replies = replies(&env, &calls);

    let report = &replies.last().unwrap()["result"];
    assert_eq!(
        (
            &report["lifecycle"],
            &report["stop_reason"],
            &report["completeness"],
            &report["completeness_reasons"][0]["code"],
        ),
        (
            &json!("stopped"),
            &stop_reason,
            &json!("incomplete"),
            &json!("stopped"),
        )
    );
}

/// Covers: story:3-154, story:3-156, operation:run.checkpoint
#[test]
fn a_seeded_runs_report_holds_every_part() {
    let env = TestEnv::new();
    write(&env, "operator-policy.toml", "version = \"0.1\"\n");

    let replies = replies(
        &env,
        &[
            (
                "run.start",
                json!({ "run_id": "seeded", "settings": { "tls": { "accept_invalid_certs": { "intranet.example": null } } } }),
            ),
            (
                "test.hand_over",
                json!({
                    "run_id": "seeded",
                    "charges": [{ "usage_kind": "requests", "amount": 2 }],
                    "warnings": [{ "code": "cpu_throttled", "message": "The host throttled the CPU." }],
                    "nothing_done": { "reason": "nothing_pending" },
                }),
            ),
            ("run.checkpoint", json!({ "run_id": "seeded" })),
            ("run.finish", json!({ "run_id": "seeded" })),
        ],
    );

    let report = &replies[3]["result"];
    assert_eq!(
        keys(report),
        [
            "checkpoint",
            "completeness",
            "ended_at",
            "lifecycle",
            "nothing_done",
            "plan",
            "plugins",
            "policy_version",
            "recording",
            "requests",
            "run_id",
            "settings",
            "started_at",
            "uncertain_effects",
            "usage",
            "verdict",
            "verdict_reasons",
            "warnings",
            "weakened",
        ]
    );
}

/// A Run that replayed a Response recording is marked so in its Run index
/// row once it ended, and a Run that did not is not (ADR 0105).
///
/// Covers: story:12-15, operation:run.list
#[test]
fn the_row_of_a_run_that_replayed_a_response_recording_is_marked_replayed() {
    let env = TestEnv::new();

    let replies = replies(
        &env,
        &[
            ("run.start", json!({ "run_id": "seeded-replay" })),
            ("run.finish", json!({ "run_id": "seeded-replay" })),
            ("run.start", json!({ "run_id": "live" })),
            ("run.finish", json!({ "run_id": "live" })),
            ("run.list", json!({})),
        ],
    );

    let marks: BTreeMap<&str, &Value> = replies[4]["result"]["runs"]
        .as_array()
        .unwrap_or_else(|| panic!("{}", replies[4]))
        .iter()
        .map(|row| (row["run_id"].as_str().unwrap(), &row["replayed"]))
        .collect();
    assert_eq!(
        marks,
        BTreeMap::from([("live", &json!(false)), ("seeded-replay", &json!(true))])
    );
}

/// Covers: story:3-154
#[test]
fn a_part_no_area_supplied_is_absent_never_zero() {
    let env = TestEnv::new();

    let replies = replies(
        &env,
        &[
            ("run.start", json!({ "run_id": "plain" })),
            ("run.finish", json!({ "run_id": "plain" })),
        ],
    );

    let report = &replies[1]["result"];
    assert_eq!(
        (keys(report), keys(&report["usage"])),
        (
            vec![
                "completeness",
                "ended_at",
                "lifecycle",
                "run_id",
                "settings",
                "started_at",
                "usage",
                "verdict",
            ],
            vec!["browser_time", "model", "proxy_bytes", "requests"],
        )
    );
}

/// Covers: story:3-155
#[rstest]
fn a_warning_handed_over_appears_with_what_it_concerns(
    #[values(
        "fingerprint_none",
        "fingerprint_js_injection",
        "fingerprint_basic",
        "fingerprint_basic_fallback",
        "browser_build_outdated",
        "browser_build_third_party",
        "browser_build_firefox",
        "browser_build_plugin",
        "human_input_fallback",
        "proxy_exit_may_change",
        "filter_vendor_flagged",
        "trap_links_off",
        "cpu_throttled",
        "storage_low",
        "politeness_store_degraded",
        "clean_up_rules_stale",
        "path_not_writable",
        "file_write_failed",
        "baseline_unavailable",
        "plugin_quantity_above_maximum",
        "plugin_figures",
        "a_code_from_a_newer_engine"
    )]
    code: &str,
) {
    let env = TestEnv::new();
    let warning = json!({
        "code": code,
        "message": "What the area says.",
        "sessions": ["session-1"],
        "folder": "/data/runs/warned",
    });

    let replies = replies(
        &env,
        &[
            ("run.start", json!({ "run_id": "warned" })),
            (
                "test.hand_over",
                json!({ "run_id": "warned", "warnings": [warning] }),
            ),
            ("run.finish", json!({ "run_id": "warned" })),
        ],
    );

    assert_eq!(replies[2]["result"]["warnings"], json!([warning]));
}

/// A periodic Checkpoint whose file cannot be written, its destination a
/// folder, warns `file_write_failed` naming the Run's folder (ADR 0122).
/// Covers: story:3-155, setting:checkpoint.interval
#[test]
#[expect(
    clippy::disallowed_methods,
    reason = "the test puts a folder where the Checkpoint's file goes"
)]
fn a_periodic_checkpoint_that_fails_warns_naming_its_folder() {
    let env = TestEnv::new();
    let folder = run_folder(&env, "periodic");
    let blocking = folder.join("checkpoint.sqlite");
    let engine = engine_with(Product::Collie, Some(reporting()));

    let finished = attached_on(&engine, &env, |mut client: Client| async move {
        client.send(&[hello("collie", "0.1")]).await;
        client.next().await;
        call(
            &mut client,
            1,
            "run.start",
            json!({ "run_id": "periodic", "settings": { "checkpoint": { "interval": 1_000 } } }),
        )
        .await;
        std::fs::create_dir_all(blocking.join("in the way")).unwrap();
        call(&mut client, 2, "test.sleep", json!({})).await;
        call(
            &mut client,
            3,
            "run.finish",
            json!({ "run_id": "periodic" }),
        )
        .await
    });

    let warning = &finished["result"]["warnings"][0];
    assert_eq!(
        (&warning["code"], &warning["folder"]),
        (
            &json!("file_write_failed"),
            &json!(folder.display().to_string())
        )
    );
}

/// Covers: story:3-156
#[test]
fn a_finish_of_a_run_its_start_found_finished_did_nothing() {
    let env = TestEnv::new();

    let replies = replies(
        &env,
        &[
            ("run.start", json!({ "run_id": "twice" })),
            ("run.finish", json!({ "run_id": "twice" })),
            ("run.start", json!({ "run_id": "twice" })),
            ("run.finish", json!({ "run_id": "twice" })),
        ],
    );

    assert_eq!(
        (
            &replies[1]["result"]["nothing_done"],
            &replies[2]["result"]["started_as"],
            &replies[3]["result"]["nothing_done"],
        ),
        (
            &Value::Null,
            &json!("existing"),
            &json!({ "reason": "already_ended", "ended_at": replies[1]["result"]["ended_at"] }),
        )
    );
}

/// Covers: story:3-157, operation:run.usage
#[test]
fn run_usage_gives_what_the_run_used_and_the_limits_that_apply() {
    let env = TestEnv::new();

    let replies = replies(
        &env,
        &[
            (
                "usage.set",
                json!({ "scope": "install", "usage_kind": "requests", "period": { "every": "month", "time_zone": "UTC" }, "amount": 100 }),
            ),
            ("run.start", json!({ "run_id": "spends" })),
            (
                "test.hand_over",
                json!({
                    "run_id": "spends",
                    "charges": [
                        { "usage_kind": "model_calls", "amount": 2 },
                        { "usage_kind": "model_tokens", "amount": 300 },
                        { "usage_kind": "browser_time", "amount": 1_500 },
                        { "usage_kind": "proxy_bytes", "amount": 4_096 },
                        { "usage_kind": "requests", "amount": 3 },
                    ],
                    "reserve": { "usage_kind": "paid_solves", "currency": "USD", "worst": 50_000, "cost": 20_000 },
                }),
            ),
            ("run.usage", json!({ "run_id": "spends" })),
            ("run.finish", json!({ "run_id": "spends" })),
            ("run.usage", json!({ "run_id": "spends" })),
        ],
    );

    let limit = |left: u64| {
        json!({
            "scope": { "kind": "install", "id": "" },
            "usage_kind": "requests",
            "period": { "every": "month", "time_zone": "UTC" },
            "resets_at": "2026-02-01T00:00:00Z",
            "amount": 100,
            "left": left,
            "set_by": "client",
        })
    };
    let used = |left: u64| {
        json!({
            "model": { "calls": 2, "tokens": 300 },
            "browser_time": 1_500,
            "proxy_bytes": 4_096,
            "requests": 3,
            "paid_solves": [{ "currency": "USD", "amount": 20_000 }],
            "limits": [limit(left)],
            "uncapped": true,
        })
    };
    let live_left = replies[3]["result"]["limits"][0]["left"].as_u64().unwrap();
    assert_eq!(
        (
            &replies[3]["result"],
            &replies[4]["result"]["usage"],
            &replies[5]["result"],
        ),
        (&used(live_left), &used(97), &used(97))
    );
}

/// Covers: story:8-92, operation:run.usage
#[test]
fn run_usage_counts_proxy_bytes_by_proxy_kind_and_their_price_by_currency() {
    let env = TestEnv::new();

    let replies = replies(
        &env,
        &[
            ("run.start", json!({ "run_id": "proxied" })),
            (
                "test.hand_over",
                json!({
                    "run_id": "proxied",
                    "charges": [
                        { "usage_kind": "proxy_bytes", "part": "residential", "amount": 1_000 },
                        { "usage_kind": "proxy_bytes", "part": "datacenter", "amount": 300 },
                        { "usage_kind": "proxy_bytes", "part": "residential", "amount": 24 },
                    ],
                    "reserve": { "usage_kind": "proxy_bytes", "currency": "USD", "worst": 7, "cost": 7 },
                }),
            ),
            ("run.finish", json!({ "run_id": "proxied" })),
            ("run.usage", json!({ "run_id": "proxied" })),
        ],
    );

    let usage = &replies[3]["result"];
    assert_eq!(
        (
            &usage["proxy_bytes"],
            &usage["proxy_bytes_by_kind"],
            &usage["proxy_price"],
            &replies[2]["result"]["usage"]["proxy_bytes_by_kind"],
        ),
        (
            &json!(1_324),
            &json!([
                { "kind": "datacenter", "bytes": 300 },
                { "kind": "residential", "bytes": 1_024 },
            ]),
            &json!([{ "currency": "USD", "amount": 7 }]),
            &json!([
                { "kind": "datacenter", "bytes": 300 },
                { "kind": "residential", "bytes": 1_024 },
            ]),
        )
    );
}

/// Covers: story:8-92, operation:run.usage
#[test]
fn a_priced_part_charges_its_amount_and_its_price_together_or_neither() {
    let env = TestEnv::new();

    let replies = replies(
        &env,
        &[
            (
                "usage.set",
                json!({ "scope": "install", "usage_kind": "proxy_bytes", "amount": 1_500 }),
            ),
            ("run.start", json!({ "run_id": "priced" })),
            (
                "test.hand_over",
                json!({
                    "run_id": "priced",
                    "charges": [
                        { "usage_kind": "proxy_bytes", "part": "mobile", "amount": 1_000, "price": { "currency": "USD", "amount": 40 } },
                        { "usage_kind": "proxy_bytes", "part": "mobile", "amount": 1_000, "price": { "currency": "USD", "amount": 40 } },
                    ],
                }),
            ),
            ("run.finish", json!({ "run_id": "priced" })),
            ("run.usage", json!({ "run_id": "priced" })),
        ],
    );

    let usage = &replies[4]["result"];
    assert_eq!(
        (
            &usage["proxy_bytes"],
            &usage["proxy_bytes_by_kind"],
            &usage["proxy_price"],
        ),
        (
            &json!(1_000),
            &json!([{ "kind": "mobile", "bytes": 1_000 }]),
            &json!([{ "currency": "USD", "amount": 40 }]),
        )
    );
}
