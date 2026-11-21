//! `Collie` over the in-memory pair on the test helper's paused
//! current-thread runtime, which drives the linked engine's tasks with the
//! test's (ADR 0081): the connection module's own interface takes the
//! runtime and the engine's dependencies as values.

use std::collections::BTreeMap;

use goodfetch_conformance::{ClientCaseRun, RustMode};
use goodfetch_engine::Dependencies;
use goodfetch_engine::clock::Clock;
use goodfetch_engine::entropy::Entropy;
use goodfetch_engine::home::{Home, RunStateLocation};
use goodfetch_engine::politeness::PolitenessOptions;
use goodfetch_engine::run_state::RunStateOptions;
use goodfetch_engine::secrets::Keychain;
use goodfetch_engine::tasks::BlockingPolicy;
use goodfetch_protocol::run::{Lifecycle, RUN_REPORT, RunId, RunParams, RunReport, StartedAs};
use goodfetch_test_support::case_logs;
use tempfile::TempDir;
use tokio::runtime::Handle;

use super::Collie;
use crate::ErrorCode;

/// What the linked engine runs on, its clock paused at the epoch, on a
/// Goodfetch home in `home`.
fn dependencies(home: &TempDir) -> Dependencies {
    #[expect(
        clippy::disallowed_methods,
        reason = "the Clock's and the Entropy's test versions, in a test (ADR 0059)"
    )]
    let (clock, entropy) = (Clock::paused(std::time::UNIX_EPOCH), Entropy::seeded(0));
    Dependencies {
        environment: BTreeMap::new(),
        keychain: Keychain::absent(),
        home: Home::new(home.path().to_owned()),
        clock,
        entropy,
        blocking: BlockingPolicy::Inline,
        run_state_location: RunStateLocation::PerMajor,
        run_state: RunStateOptions::default(),
        politeness: PolitenessOptions::default(),
    }
}

/// A Collie whose linked engine runs on the current runtime.
async fn open(home: &TempDir) -> Collie {
    Collie::open(Handle::current(), dependencies(home))
        .await
        .unwrap()
}

/// Covers: story:11-3
#[test]
fn a_collie_starts_a_run_finishes_it_and_closes_on_the_paused_runtime() {
    let home = tempfile::tempdir().unwrap();
    let runtime = goodfetch_test_support::runtime();
    runtime.block_on(async {
        let collie = open(&home).await;
        let first = RunId::parse("first").unwrap();

        let run = collie.start().run_id(&first).await.unwrap();
        let report = run.finish().await.unwrap();
        collie.close().await.unwrap();

        assert_eq!(run.id(), &first);
        assert_eq!(run.started_as(), StartedAs::New);
        assert_eq!(report.lifecycle, Lifecycle::Finished);
        let after = run.finish().await;
        assert!(
            matches!(&after, Err(error) if error.code() == ErrorCode::CLOSED),
            "{after:?}"
        );
    });
}

/// Covers: story:11-59
#[test]
fn closing_a_collie_from_a_clone_stops_its_running_run_and_closes_every_clone() {
    let home = tempfile::tempdir().unwrap();
    let runtime = goodfetch_test_support::runtime();
    let left = RunId::parse("left").unwrap();
    runtime.block_on(async {
        let collie = open(&home).await;
        let run = collie.start().run_id(&left).await.unwrap();

        collie.clone().close().await.unwrap();

        let after = collie.start().await;
        assert!(
            matches!(&after, Err(error) if error.code() == ErrorCode::CLOSED),
            "{after:?}"
        );
        let finished = run.finish().await;
        assert!(
            matches!(&finished, Err(error) if error.code() == ErrorCode::CLOSED),
            "{finished:?}"
        );
    });
    runtime.block_on(async {
        let again = open(&home).await;
        let report: RunReport = again
            .as_ref()
            .call(RUN_REPORT.name, &RunParams { run_id: left })
            .await
            .unwrap();
        again.close().await.unwrap();

        assert_eq!(report.lifecycle, Lifecycle::Stopped);
    });
}

/// The `two-connections-at-once` client conformance case, linked: two
/// Collies open at once in one process, each with its own linked engine,
/// and each one's Run finishes.
///
/// Covers: story:11-15
#[test]
fn two_collies_run_at_once_in_one_process() {
    let _case = ClientCaseRun::start(
        case_logs().as_deref(),
        "two-connections-at-once",
        RustMode::Linked,
    )
    .unwrap();
    let (first_home, second_home) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let runtime = goodfetch_test_support::runtime();
    runtime.block_on(async {
        let (first, second) = tokio::join!(open(&first_home), open(&second_home));

        let (first_report, second_report) = tokio::join!(
            async { first.start().await.unwrap().finish().await.unwrap() },
            async { second.start().await.unwrap().finish().await.unwrap() },
        );
        first.close().await.unwrap();
        second.close().await.unwrap();

        assert_eq!(
            [first_report.lifecycle, second_report.lifecycle],
            [Lifecycle::Finished, Lifecycle::Finished]
        );
    });
}

/// The `connect-after-close` client conformance case, linked: a Collie
/// opens again on the same home once one has closed, and its Run
/// finishes.
///
/// Covers: story:11-15
#[test]
fn a_collie_connects_again_after_a_close() {
    let _case = ClientCaseRun::start(
        case_logs().as_deref(),
        "connect-after-close",
        RustMode::Linked,
    )
    .unwrap();
    let home = tempfile::tempdir().unwrap();
    let runtime = goodfetch_test_support::runtime();
    runtime.block_on(async {
        open(&home).await.close().await.unwrap();

        let again = open(&home).await;
        let report = again.start().await.unwrap().finish().await.unwrap();
        again.close().await.unwrap();

        assert_eq!(report.lifecycle, Lifecycle::Finished);
    });
}
