//! A start's options through the Rust SDK (ADRs 0033, 0080, 0091, 0102
//! and 0105), the `start-takes-every-option` client conformance case in
//! `linked` mode: a start's own `timeout` stops the Run with a Checkpoint,
//! `stopped` and `incomplete`, and a start of its Run ID resumes it; a
//! zero `timeout` is refused before anything is sent; and a start that
//! names a Run again with another Plan file or Response recording fails
//! with `run_conflict` naming each. What each option sends is
//! `goodfetch-collie-client`'s
//! `a_start_sends_every_option_refuses_a_zero_timeout_and_cancels_when_dropped`.

use std::time::Duration;

use goodfetch_collie::{Collie, ErrorCode, Recording, RecordingMode, RunId, StartedAs, StopReason};
use goodfetch_conformance::{ClientCaseRun, RustMode};
use goodfetch_protocol::run::{Completeness, Lifecycle, RunConflictDetails, StartInput};
use goodfetch_test_support::{TestEnv, case_logs, real_time_runtime};
use serde_json::json;

use super::options;

/// Covers: story:11-50, story:11-132
#[test]
fn a_starts_timeout_stops_the_run_resumably_and_its_plan_and_recording_must_match() {
    let mut case = ClientCaseRun::start(
        case_logs().as_deref(),
        "start-takes-every-option",
        RustMode::Linked,
    )
    .unwrap();
    let env = TestEnv::new();
    let timed = RunId::parse("timed").unwrap();
    let planned = RunId::parse("planned").unwrap();
    let recording = |path: &str| Recording {
        mode: RecordingMode::Record,
        path: path.to_owned(),
    };
    real_time_runtime().block_on(async {
        let collie = Collie::connect(options(&env)).await.unwrap();

        let run = collie
            .start()
            .run_id(&timed)
            .timeout(Duration::from_millis(1))
            .await
            .unwrap();
        let mut progress = run.progress();
        while let Some(value) = progress.next().await {
            if value.unwrap().last {
                break;
            }
        }
        let timed_out = run.report().await.unwrap();
        let resumed = collie
            .start()
            .run_id(&timed)
            .timeout(Duration::from_secs(3600))
            .await
            .unwrap();
        resumed.finish().await.unwrap();
        let zero = collie.start().timeout(Duration::ZERO).await;

        let first = collie
            .start()
            .run_id(&planned)
            .plan("plans/a.json")
            .recording(recording("recordings/a.warc"))
            .accept_plugin_change("shop-prices")
            .no_timeout()
            .await
            .unwrap();
        first.close().await.unwrap();
        let conflict = collie
            .start()
            .run_id(&planned)
            .plan("plans/b.json")
            .await;
        collie.close().await.unwrap();

        case.check("a start's own timeout stops the Run with a Checkpoint", || {
            assert_eq!(
                (
                    timed_out.lifecycle,
                    timed_out.completeness,
                    timed_out.stop_reason.clone(),
                    timed_out.checkpoint.is_some(),
                ),
                (
                    Lifecycle::Stopped,
                    Completeness::Incomplete,
                    Some(StopReason::Timeout),
                    true,
                )
            );
        });
        case.check("a start of its Run ID resumes it", || {
            assert_eq!(resumed.started_as(), StartedAs::Resumed);
        });
        case.check(
            "a zero timeout is setting_invalid from the argument, naming the null form",
            || {
                let Err(zero) = &zero else {
                    panic!("{zero:?}");
                };
                assert_eq!(
                    (
                        zero.code(),
                        serde_json::to_value(zero.limit()).unwrap(),
                        zero.fix().is_some_and(|fix| fix.contains("`.no_timeout()`")),
                    ),
                    (
                        ErrorCode::SETTING_INVALID,
                        json!({ "key": "timeout.run", "value": 0, "layer": "run", "origin": "argument" }),
                        true,
                    )
                );
            },
        );
        case.check(
            "another Plan file and no recording are run_conflict naming each",
            || {
                let Err(conflict) = &conflict else {
                    panic!("{conflict:?}");
                };
                let details: RunConflictDetails = conflict.details().unwrap();
                assert_eq!(
                    (conflict.code(), details.differences),
                    (
                        ErrorCode::RUN_CONFLICT,
                        vec![StartInput::Plan, StartInput::Recording]
                    )
                );
            },
        );
    });
}
