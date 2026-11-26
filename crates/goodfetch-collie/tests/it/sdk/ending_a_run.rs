//! How a Run ends and what it does meanwhile through the Rust SDK (ADRs
//! 0029 and 0102), the `finish-stop-and-progress` client conformance case
//! in `linked` mode, as far as the linked engine goes: `progress` answers
//! at once, and the next value, read after two messages were set, is the
//! latest, the one between coalesced; `finish` answers the Run report with
//! Rust's points in time and durations; `stop` answers the Run stopped by
//! its program, with its Checkpoint. A Human pause, a timeout that ends a
//! finish's wait and a failed Run play on the Scripted engine, in the
//! client crate's `ending_a_run` as the Daemon.

use std::time::Duration;

use goodfetch_collie::{Collie, RunId, StopReason};
use goodfetch_conformance::{ClientCaseRun, RustMode};
use goodfetch_protocol::connection::StoppedBy;
use goodfetch_protocol::run::{Completeness, Lifecycle, ProgressMessage};
use goodfetch_test_support::{TestEnv, case_logs, real_time_runtime};

use super::options;

/// Covers: story:11-128, story:11-129
#[test]
fn finish_and_stop_answer_the_report_and_progress_coalesces_on_the_linked_engine() {
    let mut case = ClientCaseRun::start(
        case_logs().as_deref(),
        "finish-stop-and-progress",
        RustMode::Linked,
    )
    .unwrap();
    let env = TestEnv::new();
    let earlier = ProgressMessage::parse("four pages left").unwrap();
    let latest = ProgressMessage::parse("three pages left").unwrap();
    let weekly = RunId::parse("weekly").unwrap();
    let (first, set, next, finished, stopped) = real_time_runtime().block_on(async {
        let collie = Collie::connect(options(&env)).await.unwrap();
        let nightly = collie.start().await.unwrap();
        let mut following = nightly.progress();
        let first = following.next().await.unwrap().unwrap().into_inner();
        let mut set = Vec::new();
        for message in [&earlier, &latest] {
            let value = nightly.set_progress_message(message.clone()).await.unwrap();
            set.push(value.position);
        }
        let next = following.next().await.unwrap().unwrap().into_inner();
        drop(following);
        let finished = nightly.finish().await.unwrap().into_inner();
        let stopped = collie
            .start()
            .run_id(&weekly)
            .await
            .unwrap()
            .stop()
            .await
            .unwrap()
            .into_inner();
        collie.close().await.unwrap();
        (first, set, next, finished, stopped)
    });

    case.check(
        "the progress answers at once, then the latest value after it, the ones between coalesced, carrying the program's message",
        || {
            assert_eq!(
                (first.lifecycle, first.message, first.pauses),
                (Lifecycle::Running, None, vec![])
            );
            assert_eq!(set, [first.position + 1, first.position + 2]);
            assert_eq!(
                (next.position, next.message),
                (first.position + 2, Some(latest.clone()))
            );
        },
    );
    case.check("finish answers the whole Run report", || {
        assert_eq!(
            (finished.lifecycle, finished.completeness),
            (Lifecycle::Finished, Completeness::Complete)
        );
        assert!(
            finished
                .ended_at
                .duration_since(finished.started_at)
                .is_ok(),
            "the report's points in time are Rust's, its end after its start: {finished:?}"
        );
        assert_eq!(finished.usage.browser_time, Duration::ZERO);
    });
    case.check(
        "stop answers the Run stopped by its program, with its Checkpoint",
        || {
            assert_eq!(
                (stopped.lifecycle, stopped.completeness),
                (Lifecycle::Stopped, Completeness::Incomplete)
            );
            assert!(
                matches!(
                    &stopped.stop_reason,
                    Some(StopReason::Abort {
                        by: StoppedBy::Program,
                        person: None
                    })
                ),
                "{:?}",
                stopped.stop_reason
            );
            assert_eq!(
                stopped
                    .checkpoint
                    .as_ref()
                    .map(|checkpoint| &checkpoint.run_id),
                Some(&weekly)
            );
        },
    );
}
