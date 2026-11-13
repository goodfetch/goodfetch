use goodfetch_protocol::run::Lifecycle::{self, Failed, Finished, Running, Stopped};
use proptest::prelude::*;
use rstest::rstest;

use super::{Next, RunEvent, apply};
use crate::transition::Refusal;

const RUN_IN_USE: Result<Next, Refusal> = Err(Refusal::Program("run_in_use"));
const UNCHANGED: Result<Next, Refusal> = Ok(Next::Unchanged);

const TO_RUNNING: Result<Next, Refusal> = Ok(Next::To(Running));
const TO_STOPPED: Result<Next, Refusal> = Ok(Next::To(Stopped));
const TO_FINISHED: Result<Next, Refusal> = Ok(Next::To(Finished));
const TO_FAILED: Result<Next, Refusal> = Ok(Next::To(Failed));

/// Every pair of the Run's table, its outcome copied from the table in
/// the module's documentation (ADR 0128).
///
/// Covers: story:3-151
#[rstest]
#[case::running_start(Running, RunEvent::Start, UNCHANGED)]
#[case::running_resume(Running, RunEvent::Resume, TO_RUNNING)]
#[case::running_checkpoint(Running, RunEvent::Checkpoint, UNCHANGED)]
#[case::running_pause(Running, RunEvent::Pause, TO_STOPPED)]
#[case::running_migrate(Running, RunEvent::Migrate, TO_STOPPED)]
#[case::running_abort(Running, RunEvent::Abort, TO_STOPPED)]
#[case::running_timeout(Running, RunEvent::Timeout, TO_STOPPED)]
#[case::running_limit(Running, RunEvent::Limit, TO_STOPPED)]
#[case::running_limit_not_understood(Running, RunEvent::LimitNotUnderstood, TO_STOPPED)]
#[case::running_resources_exhausted(Running, RunEvent::ResourcesExhausted, TO_STOPPED)]
#[case::running_engine_lost(Running, RunEvent::EngineLost, TO_STOPPED)]
#[case::running_engine_panicked(Running, RunEvent::EnginePanicked, TO_STOPPED)]
#[case::running_drain(Running, RunEvent::Drain, TO_STOPPED)]
#[case::running_finish(Running, RunEvent::Finish, TO_FINISHED)]
#[case::running_fail(Running, RunEvent::Fail, TO_FAILED)]
#[case::running_retry_failed(Running, RunEvent::RetryFailed, UNCHANGED)]
#[case::running_writer_found(Running, RunEvent::WriterFound, RUN_IN_USE)]
#[case::stopped_start(Stopped, RunEvent::Start, UNCHANGED)]
#[case::stopped_resume(Stopped, RunEvent::Resume, TO_RUNNING)]
#[case::stopped_checkpoint(Stopped, RunEvent::Checkpoint, UNCHANGED)]
#[case::stopped_pause(Stopped, RunEvent::Pause, UNCHANGED)]
#[case::stopped_migrate(Stopped, RunEvent::Migrate, UNCHANGED)]
#[case::stopped_abort(Stopped, RunEvent::Abort, UNCHANGED)]
#[case::stopped_timeout(Stopped, RunEvent::Timeout, UNCHANGED)]
#[case::stopped_limit(Stopped, RunEvent::Limit, UNCHANGED)]
#[case::stopped_limit_not_understood(Stopped, RunEvent::LimitNotUnderstood, UNCHANGED)]
#[case::stopped_resources_exhausted(Stopped, RunEvent::ResourcesExhausted, UNCHANGED)]
#[case::stopped_engine_lost(Stopped, RunEvent::EngineLost, UNCHANGED)]
#[case::stopped_engine_panicked(Stopped, RunEvent::EnginePanicked, UNCHANGED)]
#[case::stopped_drain(Stopped, RunEvent::Drain, UNCHANGED)]
#[case::stopped_finish(Stopped, RunEvent::Finish, UNCHANGED)]
#[case::stopped_fail(Stopped, RunEvent::Fail, UNCHANGED)]
#[case::stopped_retry_failed(Stopped, RunEvent::RetryFailed, TO_RUNNING)]
#[case::stopped_writer_found(Stopped, RunEvent::WriterFound, RUN_IN_USE)]
#[case::finished_start(Finished, RunEvent::Start, UNCHANGED)]
#[case::finished_resume(Finished, RunEvent::Resume, UNCHANGED)]
#[case::finished_checkpoint(Finished, RunEvent::Checkpoint, UNCHANGED)]
#[case::finished_pause(Finished, RunEvent::Pause, UNCHANGED)]
#[case::finished_migrate(Finished, RunEvent::Migrate, UNCHANGED)]
#[case::finished_abort(Finished, RunEvent::Abort, UNCHANGED)]
#[case::finished_timeout(Finished, RunEvent::Timeout, UNCHANGED)]
#[case::finished_limit(Finished, RunEvent::Limit, UNCHANGED)]
#[case::finished_limit_not_understood(Finished, RunEvent::LimitNotUnderstood, UNCHANGED)]
#[case::finished_resources_exhausted(Finished, RunEvent::ResourcesExhausted, UNCHANGED)]
#[case::finished_engine_lost(Finished, RunEvent::EngineLost, UNCHANGED)]
#[case::finished_engine_panicked(Finished, RunEvent::EnginePanicked, UNCHANGED)]
#[case::finished_drain(Finished, RunEvent::Drain, UNCHANGED)]
#[case::finished_finish(Finished, RunEvent::Finish, UNCHANGED)]
#[case::finished_fail(Finished, RunEvent::Fail, UNCHANGED)]
#[case::finished_retry_failed(Finished, RunEvent::RetryFailed, TO_RUNNING)]
#[case::finished_writer_found(Finished, RunEvent::WriterFound, RUN_IN_USE)]
#[case::failed_start(Failed, RunEvent::Start, UNCHANGED)]
#[case::failed_resume(Failed, RunEvent::Resume, TO_RUNNING)]
#[case::failed_checkpoint(Failed, RunEvent::Checkpoint, UNCHANGED)]
#[case::failed_pause(Failed, RunEvent::Pause, UNCHANGED)]
#[case::failed_migrate(Failed, RunEvent::Migrate, UNCHANGED)]
#[case::failed_abort(Failed, RunEvent::Abort, UNCHANGED)]
#[case::failed_timeout(Failed, RunEvent::Timeout, UNCHANGED)]
#[case::failed_limit(Failed, RunEvent::Limit, UNCHANGED)]
#[case::failed_limit_not_understood(Failed, RunEvent::LimitNotUnderstood, UNCHANGED)]
#[case::failed_resources_exhausted(Failed, RunEvent::ResourcesExhausted, UNCHANGED)]
#[case::failed_engine_lost(Failed, RunEvent::EngineLost, UNCHANGED)]
#[case::failed_engine_panicked(Failed, RunEvent::EnginePanicked, UNCHANGED)]
#[case::failed_drain(Failed, RunEvent::Drain, UNCHANGED)]
#[case::failed_finish(Failed, RunEvent::Finish, UNCHANGED)]
#[case::failed_fail(Failed, RunEvent::Fail, UNCHANGED)]
#[case::failed_retry_failed(Failed, RunEvent::RetryFailed, TO_RUNNING)]
#[case::failed_writer_found(Failed, RunEvent::WriterFound, RUN_IN_USE)]
fn each_pair_of_the_runs_table_gives_its_outcome(
    #[case] state: Lifecycle,
    #[case] event: RunEvent,
    #[case] expected: Result<Next, Refusal>,
) {
    assert_eq!(
        apply(state, event).map_err(|invalid| invalid.refusal),
        expected
    );
}

/// Covers: story:3-151
#[test]
fn a_refused_transition_names_its_state_and_event() {
    let refused = apply(Finished, RunEvent::WriterFound).unwrap_err();

    assert_eq!(
        (refused.state, refused.event, refused.to_string()),
        (
            Finished,
            RunEvent::WriterFound,
            "WriterFound is refused in the state Finished".to_owned()
        )
    );
}

fn lifecycle() -> impl Strategy<Value = Lifecycle> {
    prop::sample::select(vec![Running, Stopped, Finished, Failed])
}

fn event() -> impl Strategy<Value = RunEvent> {
    prop::sample::select(vec![
        RunEvent::Start,
        RunEvent::Resume,
        RunEvent::Checkpoint,
        RunEvent::Pause,
        RunEvent::Migrate,
        RunEvent::Abort,
        RunEvent::Timeout,
        RunEvent::Limit,
        RunEvent::LimitNotUnderstood,
        RunEvent::ResourcesExhausted,
        RunEvent::EngineLost,
        RunEvent::EnginePanicked,
        RunEvent::Drain,
        RunEvent::Finish,
        RunEvent::Fail,
        RunEvent::RetryFailed,
        RunEvent::WriterFound,
    ])
}

/// Whether `code` is an Error code a Client can be answered with: one the
/// catalog defines, or one of the RPC layer's own.
fn is_error_code(code: &str) -> bool {
    goodfetch_protocol::entries::MODULES
        .iter()
        .flat_map(|entries| entries.error_codes)
        .any(|defined| defined.name == code)
}

proptest! {
    /// Whatever events reach it, a Run that left `running` comes back only
    /// through a resume or `retryFailed`, an event in an ended state never
    /// ends it otherwise, and every refusal a program causes answers with an
    /// Error code the catalog defines.
    ///
    /// Covers: story:3-151
    #[test]
    fn an_ended_run_returns_to_running_only_by_a_resume_or_a_retry_of_its_failures(
        start in lifecycle(),
        events in proptest::collection::vec(event(), 1..40),
    ) {
        let mut state = start;
        for event in events {
            match apply(state, event) {
                Ok(Next::To(next)) => {
                    if state != Running {
                        prop_assert_eq!(next, Running);
                        prop_assert!(
                            event == RunEvent::Resume || event == RunEvent::RetryFailed,
                            "{event:?} took an ended Run back to running"
                        );
                    }
                    state = next;
                }
                Ok(Next::Unchanged) => {}
                Err(refused) => {
                    prop_assert_eq!((refused.state, refused.event), (state, event));
                    match refused.refusal {
                        Refusal::Program(code) => prop_assert!(is_error_code(code), "{code}"),
                        Refusal::Internal => {}
                    }
                }
            }
        }
    }
}
