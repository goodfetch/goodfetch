//! Seeded kills of real processes (ADR 0060): a child is killed at the
//! protocol event its test's seed draws, and the kill is in the Observation
//! log.

use std::time::Duration;

use goodfetch_test_support::{
    Fault, KillAtError, Moment, ObservationLog, Operation, Process, Record, TOOLS_VARIABLE,
    TestEnv, kill_at, kill_at_within,
};

use crate::{TEST_ENV, manifest, probe};

/// The events the probe sends, each three times.
const EVENTS: [&str; 2] = ["delivery.begin", "delivery.commit"];

/// Starts the probe that sends protocol events, then hangs.
fn engine(env: &TestEnv) -> Process {
    env.spawn(&mut probe(env, "probe::sends_protocol_events_then_hangs"))
        .unwrap()
}

#[test]
fn a_process_is_killed_at_the_seeded_moment() {
    let env = TestEnv::new();
    let moment = Moment::drawn(env.seed(), &EVENTS, 3);
    let process = engine(&env);
    let mut log = ObservationLog::<()>::for_test(&env);

    let killed = kill_at(&process, &moment, &mut log).unwrap();

    assert_eq!(killed.message["method"], moment.event.as_str());
    assert_eq!(killed.message["params"]["n"], moment.occurrence);
    assert!(!killed.status.success());
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt as _;
        assert_eq!(killed.status.signal(), Some(9));
    }
    assert_eq!(
        log.records(),
        [Record::new(
            process.program(),
            Operation::Fault(Fault::Kill {
                event: moment.event.clone(),
                occurrence: moment.occurrence,
            }),
        )]
    );
}

/// The kill lands at the moment: the process sends no event after it, so
/// an implementation that read the whole output before killing would show
/// the later events in the probe's own record.
#[test]
#[expect(
    clippy::disallowed_methods,
    reason = "the test reads the probe's record of what it sent"
)]
fn no_event_after_the_moment_is_sent() {
    let env = TestEnv::new();
    let process = env
        .spawn(&mut probe(
            &env,
            "probe::sends_protocol_events_slowly_then_hangs",
        ))
        .unwrap();
    let mut log = ObservationLog::<()>::new();

    kill_at(&process, &Moment::new("delivery.begin", 2), &mut log).unwrap();

    let sent = std::fs::read_to_string(env.folder().join(crate::probe::SENT)).unwrap();
    assert_eq!(
        sent.lines().collect::<Vec<_>>(),
        ["delivery.begin 1", "delivery.commit 1", "delivery.begin 2"]
    );
}

#[test]
fn the_same_seed_kills_at_the_same_moment() {
    let env = TestEnv::new();
    let first = engine(&env);
    let second = engine(&env);
    let mut first_log = ObservationLog::<()>::for_test(&env);
    let mut second_log = ObservationLog::<()>::new();

    let first_kill = kill_at(
        &first,
        &Moment::drawn(env.seed(), &EVENTS, 3),
        &mut first_log,
    );
    let second_kill = kill_at(
        &second,
        &Moment::drawn(env.seed(), &EVENTS, 3),
        &mut second_log,
    );

    assert_eq!(first_kill.unwrap().message, second_kill.unwrap().message);
    assert_eq!(first_log.records(), second_log.records());
}

#[test]
fn a_moment_that_never_comes_fails_at_the_deadline() {
    let env = TestEnv::new();
    let process = engine(&env);
    let mut log = ObservationLog::<()>::for_test(&env);

    let kill = kill_at_within(
        &process,
        &Moment::new("delivery.commit", 4),
        &mut log,
        Duration::from_millis(500),
    );

    process.stop().unwrap();
    assert!(matches!(kill, Err(KillAtError::Deadline(_))), "{kill:?}");
    assert_eq!(log.records(), []);
}

#[test]
fn a_process_that_ends_before_the_moment_is_not_killed() {
    let env = TestEnv::new();
    let mut command = env.process_command(TEST_ENV);
    command.env(TOOLS_VARIABLE, manifest(&env));
    let process = env.spawn(&mut command).unwrap();
    drop(process.child().stdin.take());
    let mut log = ObservationLog::<()>::for_test(&env);

    let kill = kill_at(&process, &Moment::new("delivery.begin", 1), &mut log);

    let status = process.child().wait().unwrap();
    assert!(matches!(kill, Err(KillAtError::Ended)), "{kill:?}");
    assert!(status.success());
    assert_eq!(log.records(), []);
}
