use std::time::Duration;

use rstest::rstest;

use super::InstanceState::{Quarantined, Ready, Restarting, Starting, Stopped};
use super::RecentCrashes::{Fewer, Third};
use super::{
    CrashDecision, CrashTracker, InstanceEffect, InstanceEvent, InstanceState, QUARANTINE_WINDOW,
    apply,
};
use crate::clock::Clock;
use crate::transition::Refusal;

type Outcome = Result<(InstanceState, InstanceEffect), Refusal>;

const INTERNAL: Outcome = Err(Refusal::Internal);
const PLUGIN_QUARANTINED: Outcome = Err(Refusal::Program("plugin_quarantined"));

/// What `event` does to an instance in `state`, with the refusal alone of
/// a refused pair, which must leave the state as it was.
fn outcome(state: InstanceState, event: InstanceEvent) -> Outcome {
    apply(state, event).map_err(|refused| {
        assert_eq!((refused.state, refused.event), (state, event));
        refused.refusal
    })
}

/// Every pair of the instance's table, its next state and effect, or its
/// code, copied from the spec's table (ADR 0128); a crash here is the first
/// or second within 10 minutes.
#[rstest]
#[case::starting_hello_ok(Starting, InstanceEvent::HelloOk, Ok((Ready, InstanceEffect::Open)))]
#[case::starting_hello_refused(Starting, InstanceEvent::HelloRefused, Ok((Stopped, InstanceEffect::Refuse)))]
#[case::starting_crash(Starting, InstanceEvent::Crash(Fewer), Ok((Restarting, InstanceEffect::Restart)))]
#[case::starting_ping_missed(Starting, InstanceEvent::PingMissed(Fewer), Ok((Starting, InstanceEffect::Nothing)))]
#[case::starting_call(Starting, InstanceEvent::Call, Ok((Starting, InstanceEffect::Queue)))]
#[case::starting_result(Starting, InstanceEvent::Result, INTERNAL)]
#[case::starting_late_result(Starting, InstanceEvent::LateResult, Ok((Starting, InstanceEffect::Nothing)))]
#[case::starting_run_ended(Starting, InstanceEvent::RunEnded, Ok((Stopped, InstanceEffect::Stop)))]
#[case::starting_stop(Starting, InstanceEvent::Stop, Ok((Stopped, InstanceEffect::Stop)))]
#[case::ready_hello_ok(Ready, InstanceEvent::HelloOk, INTERNAL)]
#[case::ready_hello_refused(Ready, InstanceEvent::HelloRefused, INTERNAL)]
#[case::ready_crash(Ready, InstanceEvent::Crash(Fewer), Ok((Restarting, InstanceEffect::Restart)))]
#[case::ready_ping_missed(Ready, InstanceEvent::PingMissed(Fewer), Ok((Restarting, InstanceEffect::Restart)))]
#[case::ready_call(Ready, InstanceEvent::Call, Ok((Ready, InstanceEffect::Send)))]
#[case::ready_result(Ready, InstanceEvent::Result, Ok((Ready, InstanceEffect::Deliver)))]
#[case::ready_late_result(Ready, InstanceEvent::LateResult, Ok((Ready, InstanceEffect::Nothing)))]
#[case::ready_run_ended(Ready, InstanceEvent::RunEnded, Ok((Stopped, InstanceEffect::Stop)))]
#[case::ready_stop(Ready, InstanceEvent::Stop, Ok((Stopped, InstanceEffect::Stop)))]
#[case::restarting_hello_ok(Restarting, InstanceEvent::HelloOk, Ok((Ready, InstanceEffect::Open)))]
#[case::restarting_hello_refused(Restarting, InstanceEvent::HelloRefused, Ok((Stopped, InstanceEffect::Refuse)))]
#[case::restarting_crash(Restarting, InstanceEvent::Crash(Fewer), Ok((Restarting, InstanceEffect::Restart)))]
#[case::restarting_ping_missed(Restarting, InstanceEvent::PingMissed(Fewer), Ok((Restarting, InstanceEffect::Nothing)))]
#[case::restarting_call(Restarting, InstanceEvent::Call, Ok((Restarting, InstanceEffect::Queue)))]
#[case::restarting_result(Restarting, InstanceEvent::Result, INTERNAL)]
#[case::restarting_late_result(Restarting, InstanceEvent::LateResult, Ok((Restarting, InstanceEffect::Nothing)))]
#[case::restarting_run_ended(Restarting, InstanceEvent::RunEnded, Ok((Stopped, InstanceEffect::Stop)))]
#[case::restarting_stop(Restarting, InstanceEvent::Stop, Ok((Stopped, InstanceEffect::Stop)))]
#[case::quarantined_hello_ok(Quarantined, InstanceEvent::HelloOk, INTERNAL)]
#[case::quarantined_hello_refused(Quarantined, InstanceEvent::HelloRefused, INTERNAL)]
#[case::quarantined_crash(Quarantined, InstanceEvent::Crash(Fewer), Ok((Quarantined, InstanceEffect::Nothing)))]
#[case::quarantined_ping_missed(Quarantined, InstanceEvent::PingMissed(Fewer), Ok((Quarantined, InstanceEffect::Nothing)))]
#[case::quarantined_call(Quarantined, InstanceEvent::Call, PLUGIN_QUARANTINED)]
#[case::quarantined_result(Quarantined, InstanceEvent::Result, INTERNAL)]
#[case::quarantined_late_result(Quarantined, InstanceEvent::LateResult, Ok((Quarantined, InstanceEffect::Nothing)))]
#[case::quarantined_run_ended(Quarantined, InstanceEvent::RunEnded, Ok((Stopped, InstanceEffect::Stop)))]
#[case::quarantined_stop(Quarantined, InstanceEvent::Stop, Ok((Stopped, InstanceEffect::Stop)))]
#[case::stopped_hello_ok(Stopped, InstanceEvent::HelloOk, Ok((Stopped, InstanceEffect::Nothing)))]
#[case::stopped_hello_refused(Stopped, InstanceEvent::HelloRefused, Ok((Stopped, InstanceEffect::Nothing)))]
#[case::stopped_crash(Stopped, InstanceEvent::Crash(Fewer), Ok((Stopped, InstanceEffect::Nothing)))]
#[case::stopped_ping_missed(Stopped, InstanceEvent::PingMissed(Fewer), Ok((Stopped, InstanceEffect::Nothing)))]
#[case::stopped_call(Stopped, InstanceEvent::Call, INTERNAL)]
#[case::stopped_result(Stopped, InstanceEvent::Result, INTERNAL)]
#[case::stopped_late_result(Stopped, InstanceEvent::LateResult, Ok((Stopped, InstanceEffect::Nothing)))]
#[case::stopped_run_ended(Stopped, InstanceEvent::RunEnded, Ok((Stopped, InstanceEffect::Nothing)))]
#[case::stopped_stop(Stopped, InstanceEvent::Stop, Ok((Stopped, InstanceEffect::Nothing)))]
fn every_pair_of_the_instance_table_gives_its_next_state_and_effect_or_its_code(
    #[case] state: InstanceState,
    #[case] event: InstanceEvent,
    #[case] expected: Outcome,
) {
    assert_eq!(outcome(state, event), expected);
}

/// The third crash within 10 minutes, or a third missed heartbeat that
/// counts as one, quarantines the instance where a crash would restart it,
/// and changes nothing where a crash changes nothing.
#[rstest]
#[case::starting_crash(Starting, InstanceEvent::Crash(Third), Ok((Quarantined, InstanceEffect::Quarantine)))]
#[case::ready_crash(Ready, InstanceEvent::Crash(Third), Ok((Quarantined, InstanceEffect::Quarantine)))]
#[case::ready_ping_missed(Ready, InstanceEvent::PingMissed(Third), Ok((Quarantined, InstanceEffect::Quarantine)))]
#[case::restarting_crash(Restarting, InstanceEvent::Crash(Third), Ok((Quarantined, InstanceEffect::Quarantine)))]
#[case::restarting_ping_missed(Restarting, InstanceEvent::PingMissed(Third), Ok((Restarting, InstanceEffect::Nothing)))]
#[case::quarantined_crash(Quarantined, InstanceEvent::Crash(Third), Ok((Quarantined, InstanceEffect::Nothing)))]
#[case::stopped_crash(Stopped, InstanceEvent::Crash(Third), Ok((Stopped, InstanceEffect::Nothing)))]
fn the_third_crash_within_ten_minutes_quarantines_the_instance(
    #[case] state: InstanceState,
    #[case] event: InstanceEvent,
    #[case] expected: Outcome,
) {
    assert_eq!(outcome(state, event), expected);
}

#[test]
fn backoff_doubles_from_one_second_and_caps_at_thirty() {
    let waits: Vec<Duration> = (1..=8).map(CrashTracker::backoff).collect();
    assert_eq!(
        waits,
        vec![
            Duration::from_secs(1),
            Duration::from_secs(2),
            Duration::from_secs(4),
            Duration::from_secs(8),
            Duration::from_secs(16),
            Duration::from_secs(30),
            Duration::from_secs(30),
            Duration::from_secs(30),
        ]
    );
}

#[test]
fn crash_tracker_quarantines_on_third_crash_within_ten_minutes() {
    let start = Clock::system().now();
    let mut tracker = CrashTracker::new();

    assert_eq!(
        tracker.record(start),
        CrashDecision::Restart {
            wait: Duration::from_secs(1)
        }
    );
    assert_eq!(
        tracker.record(start + Duration::from_mins(5)),
        CrashDecision::Restart {
            wait: Duration::from_secs(2)
        }
    );
    assert_eq!(
        tracker.record(start + QUARANTINE_WINDOW),
        CrashDecision::Quarantine { crashes: 3 }
    );
}

#[test]
fn crash_tracker_prunes_crashes_older_than_ten_minutes_while_doubling_backoff() {
    let start = Clock::system().now();
    let mut tracker = CrashTracker::new();
    let step = QUARANTINE_WINDOW + Duration::from_secs(1);

    let waits: Vec<Duration> = (0..7)
        .map(|i| match tracker.record(start + step * i) {
            CrashDecision::Restart { wait } => wait,
            CrashDecision::Quarantine { .. } => panic!("spaced crashes must not quarantine"),
        })
        .collect();
    assert_eq!(
        waits,
        vec![
            Duration::from_secs(1),
            Duration::from_secs(2),
            Duration::from_secs(4),
            Duration::from_secs(8),
            Duration::from_secs(16),
            Duration::from_secs(30),
            Duration::from_secs(30),
        ]
    );
}
