//! Unknown values (ADR 0079), the `unknown-values-are-kept` client
//! conformance case in `linked` mode: Rust's linked engine is the real one,
//! in the program's process, whose composition here is the engine's own
//! contribution and a test contribution answering the page calls as the
//! corpus's `run/unknown-values-are-kept.json` does, as a newer
//! contribution would, with a guard on the engine's own calls answering
//! `run.finish` with that script's report. Each value crossing the linked
//! engine's protocol server and the client crate's reading of it that no
//! catalog of the package has is kept as received and fails nothing of its
//! own: a code no catalog has keeps its name, its group and its details;
//! `call_cancelled` keeps a canceller and a closed value it does not know;
//! `file_write_failed` keeps a reason no list has; and the Run's report
//! keeps its Verdict, never read as `verified`, its Stop reason as the
//! union's unknown member with the tag it was sent in, Foxhound's Usage
//! kind and Limit scope, and a warning code no table has. The case's
//! Daemon part is `goodfetch-collie-client`'s `unknown_values`, whose
//! checks these repeat; TypeScript's `unknown-values.test.ts` and Python's
//! `test_unknown_values.py` are its twins.

use std::future::ready;
use std::sync::Arc;

use goodfetch_collie::linked::{Composed, ProductConnection};
use goodfetch_collie::{EngineChoice, Error, ErrorCode};
use goodfetch_conformance::{ClientCaseRun, RustMode};
use goodfetch_engine::{
    Answered, CallContext, CallError, Composition, Contribution, Guard, GuardAnswer, Handler,
};
use goodfetch_protocol::catalog::{ErrorCodeDefinition, ErrorGroup, PROTOCOL_VERSION, Product};
use goodfetch_protocol::checkpoint::{FILE_WRITE_FAILED, FileWriteFailedDetails, WriteSide};
use goodfetch_protocol::connection::{CALL_CANCELLED, CallCancelledDetails, ClosedValue};
use goodfetch_protocol::error_code::ErrorDetails;
use goodfetch_protocol::page::{LOCATOR_CLICK, PAGE_GOTO, RUN_NEW_PAGE};
use goodfetch_protocol::run::{
    AppliedLimit, KindUsage, Lifecycle, LimitScope, LimitSetter, RUN_FINISH, RunReport, StopReason,
    Verdict,
};
use goodfetch_test_support::{TestEnv, case_logs, real_time_runtime};
use serde::Deserialize;
use serde_json::{Value, json};

use super::options;

/// A page code no catalog of this package has, which the engine answers
/// as a newer contribution would.
static FRAME_FROZEN: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "frame_frozen",
    description: "The page's frame froze before the call could start.",
    group: ErrorGroup::Page,
    retryable: false,
    exit_code: None,
    arrived_in: PROTOCOL_VERSION,
};

/// The details of [`FRAME_FROZEN`], as a program written against the
/// newer catalog reads them.
#[derive(Debug, Deserialize, PartialEq)]
struct FrameFrozenDetails {
    /// How long the frame stayed frozen, in milliseconds.
    frozen_for: u64,
}

impl ErrorDetails for FrameFrozenDetails {
    const CODES: &'static [ErrorCode] = &[ErrorCode::from_static("frame_frozen")];
}

/// What the click on `selector` fails with, the script's failure for it.
fn click_failing(selector: &str) -> CallError {
    match selector {
        "#next" => CallError::new(
            &CALL_CANCELLED,
            "`locator.click` was cancelled: its tab group closed under it",
            &json!({
                "operation": "locator.click",
                "call_id": "call-2",
                "target": "#next",
                "by": "satellite",
                "closed": "tab_group",
            }),
        ),
        "#save" => CallError::new(
            &FILE_WRITE_FAILED,
            "the engine could not write the Run's state: crosses devices",
            &json!({
                "operation": "locator.click",
                "call_id": "call-3",
                "target": "#save",
                "reason": "crosses_devices",
                "os_code": "EXDEV",
                "path": "/srv/goodfetch/runs/the-run",
                "side": "engine",
            }),
        ),
        _ => panic!("no click of the case targets {selector}"),
    }
}

/// The script's stopped report of the Run `run_id`, with the values no
/// catalog of this package has.
fn stopped_report(run_id: &str) -> Value {
    json!({
        "run_id": run_id,
        "lifecycle": "stopped",
        "verdict": "verified_by_quorum",
        "verdict_reasons": [
            {"code": "quorum_pending", "message": "Two of three reviewers agreed.", "count": 1},
        ],
        "completeness": "complete",
        "started_at": "2026-01-01T00:00:00Z",
        "ended_at": "2026-01-01T00:01:00Z",
        "stop_reason": {"kind": "budget_window", "window": "night"},
        "usage": {
            "model": {
                "calls": 0,
                "tokens": 0,
                "input_tokens": 0,
                "cache_read_tokens": 0,
                "cache_write_tokens": 0,
                "output_tokens": 0,
                "reasoning_tokens": 0,
                "price": null,
            },
            "browser_time": 0,
            "proxy_bytes": 0,
            "requests": 3,
            "other": [{"usage_kind": "paid_fetch", "amount": 3, "currency": null}],
            "limits": [{
                "scope": {"kind": "partitioned_crawl", "id": "books"},
                "usage_kind": "paid_fetch",
                "amount": 10,
                "left": 7,
                "set_by": "person",
            }],
        },
        "warnings": [{"code": "quorum_slow", "message": "A reviewer took over a minute."}],
    })
}

/// The guard on the engine's own calls answering `run.finish` with the
/// script's report in its handler's place, as a newer engine's would.
fn finishing_as_the_script() -> Guard {
    Arc::new(|operation, params, _read, context| {
        (operation.name == RUN_FINISH.name).then(|| {
            let run_id = params
                .read::<Value>()
                .map(|params| params["run_id"].clone());
            let answer = run_id.and_then(|run_id| {
                Answered::encode(
                    context,
                    &stopped_report(run_id.as_str().unwrap_or_default()),
                )
            });
            Box::pin(ready(answer)) as GuardAnswer
        })
    })
}

/// The engine's own contribution, its `run.finish` answered as the script
/// answers it, and the case's page calls answered as the script answers
/// them.
fn answering_as_the_script() -> Composition {
    Composition::default()
        .with(goodfetch_engine::contribution().guarded(&finishing_as_the_script()))
        .with(
            Contribution::new("test_unknown_values")
                .with_handler(Handler::new(&RUN_NEW_PAGE, |_: Value, _: CallContext| {
                    ready(Ok::<_, CallError>(
                        json!({"page_id": "page-1", "session_id": "session-1"}),
                    ))
                }))
                .with_handler(Handler::new(&PAGE_GOTO, |_: Value, _: CallContext| {
                    ready(Err::<Value, _>(CallError::new(
                        &FRAME_FROZEN,
                        "the page's frame froze before the navigation could start",
                        &json!({
                            "operation": "page.goto",
                            "call_id": "call-1",
                            "url": "https://shop.test/",
                            "frozen_for": 1200,
                        }),
                    )))
                }))
                .with_handler(Handler::new(
                    &LOCATOR_CLICK,
                    |params: Value, _: CallContext| {
                        let selector = params["locator"]["selector"].as_str().unwrap_or_default();
                        ready(Err::<Value, _>(click_failing(selector)))
                    },
                )),
        )
}

/// What the case's calls answered.
struct Played {
    /// The Run the case started.
    run_id: String,
    /// The navigation's failure with a code no catalog has.
    unknown_code: Error,
    /// The click cancelled by a canceller, under a closed value, no
    /// catalog has.
    cancelled: Error,
    /// The click whose write failed for a reason no list has.
    write_failed: Error,
    /// The Run's stopped report.
    report: RunReport,
}

/// The error `answer` holds.
fn failure<T: std::fmt::Debug>(answer: Result<T, Error>) -> Error {
    let Err(error) = answer else {
        panic!("{answer:?}");
    };
    error
}

/// Plays the case's calls on the linked engine of `env`'s home.
async fn played(env: &TestEnv) -> Played {
    let engine = Composed {
        product: Product::Collie,
        composition: answering_as_the_script,
    };
    let collie = ProductConnection::connect(engine, options(env).engine(EngineChoice::Linked))
        .await
        .unwrap();
    let connection = collie.connection();
    let run = connection.start().await.unwrap();
    let page = run.new_page().await.unwrap();
    let unknown_code = failure(page.goto("https://shop.test/").await);
    let cancelled = failure(page.locator("#next").click().await);
    let write_failed = failure(page.locator("#save").click().await);
    let report = run.finish().await.unwrap().into_inner();
    collie.close().await.unwrap();
    Played {
        run_id: run.id().to_string(),
        unknown_code,
        cancelled,
        write_failed,
        report,
    }
}

/// A code no catalog has keeps its name, its group and its details, and
/// the two known codes keep the values no catalog has in their details.
fn assert_errors_kept(played: &Played) {
    let unknown_code = &played.unknown_code;
    assert_eq!(
        (
            unknown_code.code().as_str(),
            unknown_code.group(),
            unknown_code.retryable(),
            unknown_code.operation(),
            unknown_code.url()
        ),
        (
            "frame_frozen",
            ErrorGroup::Page,
            false,
            Some("page.goto"),
            Some("https://shop.test/")
        ),
        "a code no catalog has keeps its name and its group"
    );
    assert_eq!(
        unknown_code.details::<FrameFrozenDetails>(),
        Some(FrameFrozenDetails { frozen_for: 1200 })
    );

    let cancelled = &played.cancelled;
    assert_eq!(
        (cancelled.code(), cancelled.group()),
        (ErrorCode::CALL_CANCELLED, ErrorGroup::Unavailable)
    );
    let details = cancelled.details::<CallCancelledDetails>().unwrap();
    assert_eq!(
        (
            details.by.as_str(),
            details.closed.as_ref().map(ClosedValue::as_str)
        ),
        ("satellite", Some("tab_group")),
        "a canceller and a closed value no catalog has are kept"
    );

    let write_failed = &played.write_failed;
    assert_eq!(
        (write_failed.code(), write_failed.group()),
        (ErrorCode::FILE_WRITE_FAILED, ErrorGroup::Unavailable)
    );
    assert_eq!(
        write_failed.details::<FileWriteFailedDetails>(),
        Some(FileWriteFailedDetails {
            reason: "crosses_devices".to_owned(),
            os_code: Some("EXDEV".to_owned()),
            path: Some("/srv/goodfetch/runs/the-run".to_owned()),
            side: WriteSide::Engine,
        }),
        "a reason no list has is kept"
    );
}

/// The report keeps its Verdict, never `verified`, its Stop reason with
/// the tag it was sent in, Foxhound's Usage kind and Limit scope, and a
/// warning code no table has.
fn assert_report_kept(played: &Played) {
    let report = &played.report;
    assert_eq!(
        (report.run_id.to_string(), report.lifecycle),
        (played.run_id.clone(), Lifecycle::Stopped)
    );
    assert!(
        matches!(report.verdict, Verdict::Unknown(_)),
        "a Verdict no catalog has is never read as a known one: {:?}",
        report.verdict
    );
    assert_eq!(report.verdict.as_str(), "verified_by_quorum");
    let Some(StopReason::Unknown(reason)) = &report.stop_reason else {
        panic!(
            "expected the unknown Stop reason, got {:?}",
            report.stop_reason
        );
    };
    assert_eq!(
        (reason.tag(), serde_json::to_value(reason).unwrap()),
        (
            "budget_window",
            json!({"kind": "budget_window", "window": "night"})
        ),
        "a Stop reason no catalog has keeps its tag and its fields"
    );
    assert_eq!(
        report.usage.other,
        [KindUsage {
            usage_kind: "paid_fetch".to_owned(),
            currency: None,
            amount: 3,
        }]
    );
    assert_eq!(
        report.usage.limits,
        [AppliedLimit {
            scope: LimitScope {
                kind: "partitioned_crawl".to_owned(),
                id: "books".to_owned(),
            },
            usage_kind: "paid_fetch".to_owned(),
            currency: None,
            period: None,
            resets_at: None,
            amount: 10,
            left: 7,
            set_by: LimitSetter::Person,
        }],
        "Foxhound's Usage kind and Limit scope are kept as strings"
    );
    assert_eq!(
        report
            .warnings
            .iter()
            .map(|warning| warning.code.as_str())
            .collect::<Vec<_>>(),
        ["quorum_slow"]
    );
}

/// The `unknown-values-are-kept` client conformance case, linked: each
/// value no catalog has is kept, and only the calls the engine failed
/// fail.
///
/// Covers: story:11-33, story:11-188
#[test]
fn each_value_no_catalog_has_is_kept_on_the_linked_engine_and_fails_nothing_of_its_own() {
    let _case = ClientCaseRun::start(
        case_logs().as_deref(),
        "unknown-values-are-kept",
        RustMode::Linked,
    )
    .unwrap();
    let env = TestEnv::new();

    let played = real_time_runtime().block_on(played(&env));

    assert_errors_kept(&played);
    assert_report_kept(&played);
}
