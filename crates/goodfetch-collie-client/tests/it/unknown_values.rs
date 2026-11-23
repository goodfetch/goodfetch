//! Unknown values from a newer engine (ADR 0079) through the client crate:
//! the corpus's script of the `unknown-values-are-kept` client conformance
//! case, which the TypeScript and Python SDKs play too, with the Scripted
//! engine as the Daemon. Each value the crate reads that no catalog of its
//! package has is kept as received and fails nothing of its own: a code no
//! catalog has keeps its name, its group and its details; `call_cancelled`
//! keeps a canceller and a closed value it does not know;
//! `file_write_failed` keeps a reason no list has; and the Run's report
//! keeps its Verdict, never read as `verified`, its Stop reason as the
//! union's unknown member with the tag it was sent in, Foxhound's Usage
//! kind and Limit scope, values of types both Products share, and a
//! warning code no table has. The Scripted engine fails on a message it
//! does not expect and on a step left unused, so its clean exit says the
//! crate sent exactly what the script holds. This is Rust's `daemon` mode
//! play; `goodfetch-collie`'s `unknown_values` is its `linked` one.

use goodfetch_collie_client::transport::{Endpoint, Network};
use goodfetch_collie_client::{ClientConnection, ConnectionRuntime, Error, ErrorCode};
use goodfetch_conformance::{ClientCaseRun, RustMode};
use goodfetch_protocol::catalog::ErrorGroup;
use goodfetch_protocol::checkpoint::{FileWriteFailedDetails, WriteSide};
use goodfetch_protocol::connection::{
    CallCancelledDetails, ClosedValue, DaemonEndpoint, HelloParams,
};
use goodfetch_protocol::error_code::ErrorDetails;
use goodfetch_protocol::run::{
    AppliedLimit, KindUsage, Lifecycle, LimitScope, LimitSetter, RunReport, StopReason, Verdict,
};
use goodfetch_test_support::{TestEnv, case_logs};
use serde::Deserialize;
use serde_json::json;

use crate::scripted_daemon::{daemon_hello, scripted_daemon_hearing};
use crate::start::{code_group_retryable, failure};

/// The Run the script starts.
const RUN_ID: &str = "01ke6p4ym0j8x3v5wq2rtbn7hc";

/// The details of `frame_frozen`, a page code no catalog of this package
/// has, as a program written against the newer engine's catalog reads
/// them.
#[derive(Debug, Deserialize, PartialEq)]
struct FrameFrozenDetails {
    /// How long the frame stayed frozen, in milliseconds.
    frozen_for: u64,
}

impl ErrorDetails for FrameFrozenDetails {
    const CODES: &'static [ErrorCode] = &[ErrorCode::from_static("frame_frozen")];
}

/// What the script's calls answered.
struct Played {
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

/// Plays the script's calls on a Connection to the Daemon at `endpoint`.
async fn played(endpoint: DaemonEndpoint, hello: HelloParams) -> Played {
    let (connection, _said) = ClientConnection::open(
        ConnectionRuntime::driven(tokio::runtime::Handle::current()),
        &Network::Os,
        &Endpoint::Daemon(endpoint),
        hello,
    )
    .await
    .unwrap();
    let run = connection.start().await.unwrap();
    let page = run.new_page().await.unwrap();
    let unknown_code = failure(page.goto("https://shop.test/").await);
    let cancelled = failure(page.locator("#next").click().await);
    let write_failed = failure(page.locator("#save").click().await);
    let report = run.finish().await.unwrap().into_inner();
    connection.close().await.unwrap();
    Played {
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
            unknown_code.retryable()
        ),
        ("frame_frozen", ErrorGroup::Page, false),
        "a code no catalog has keeps its name and its group"
    );
    assert_eq!(
        (
            unknown_code.operation(),
            unknown_code.call_id(),
            unknown_code.run_id(),
            unknown_code.url()
        ),
        (
            Some("page.goto"),
            Some("call-1"),
            Some(RUN_ID),
            Some("https://shop.test/")
        )
    );
    assert_eq!(
        unknown_code.details::<FrameFrozenDetails>(),
        Some(FrameFrozenDetails { frozen_for: 1200 })
    );

    let cancelled = &played.cancelled;
    assert_eq!(
        code_group_retryable(cancelled),
        (ErrorCode::CALL_CANCELLED, ErrorGroup::Unavailable, false)
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
        code_group_retryable(write_failed),
        (ErrorCode::FILE_WRITE_FAILED, ErrorGroup::Unavailable, false)
    );
    assert_eq!(
        write_failed.details::<FileWriteFailedDetails>(),
        Some(FileWriteFailedDetails {
            reason: "crosses_devices".to_owned(),
            os_code: Some("EXDEV".to_owned()),
            path: Some(format!("/srv/goodfetch/runs/{RUN_ID}")),
            side: WriteSide::Engine,
        }),
        "a reason no list has is kept"
    );
}

/// The report keeps its Verdict, never `verified`, its Stop reason with
/// the tag it was sent in, Foxhound's Usage kind and Limit scope, and a
/// warning code no table has.
fn assert_report_kept(report: &RunReport) {
    assert_eq!(report.lifecycle, Lifecycle::Stopped);
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

/// The `unknown-values-are-kept` client conformance case, played by the
/// Scripted engine's Daemon: each value no catalog has is kept, and only
/// the calls the engine failed fail.
///
/// Covers: story:11-33, story:11-188
#[test]
fn each_value_a_newer_engine_sends_is_kept_and_fails_nothing_of_its_own() {
    let _case = ClientCaseRun::start(
        case_logs().as_deref(),
        "unknown-values-are-kept",
        RustMode::Daemon,
    )
    .unwrap();
    let env = TestEnv::new();
    let runtime = goodfetch_test_support::real_time_runtime();
    let hello = daemon_hello();
    let (process, endpoint) =
        scripted_daemon_hearing(&env, "sdk-rust/run/unknown-values-are-kept.json", &hello);
    let played = runtime.block_on(played(endpoint, hello));
    let status = process.child().wait().unwrap();

    assert_errors_kept(&played);
    assert_report_kept(&played.report);
    assert!(
        status.success(),
        "the Scripted engine played its script whole"
    );
}
