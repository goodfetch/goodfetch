use super::*;
use crate::compare::trials::{QUIET_LOAD, STRACE_QUIET_LOAD};

/// The loads `read` gives in turn, and the seconds each pause slept.
fn wait(threshold: f64, loads: &[f64]) -> (Option<f64>, Vec<u64>) {
    let mut loads = loads.iter().copied();
    let mut slept = Vec::new();
    let started = waited(threshold, || loads.next(), |seconds| slept.push(seconds));
    (started, slept)
}

/// A run under strace starts only once the load falls under its own
/// threshold (#2521: one started at 19.3 lost its cell), and the load it
/// starts at is the one read last.
#[test]
fn a_run_starts_once_the_load_falls_under_its_threshold() {
    assert_eq!(
        wait(STRACE_QUIET_LOAD, &[19.3, 9.0, 7.5, 1.0]),
        (Some(7.5), vec![10, 10])
    );
    assert_eq!(wait(QUIET_LOAD, &[7.5, 3.9]), (Some(3.9), vec![10]));
}

/// A machine that stays loaded holds a run back no longer than its wait:
/// it then starts, at the load read last.
#[test]
fn a_run_on_a_machine_that_stays_loaded_starts_once_its_wait_ends() {
    let loads = vec![19.3; 1000];

    let (started, slept) = wait(STRACE_QUIET_LOAD, &loads);

    assert_eq!(
        (started, slept.iter().sum::<u64>()),
        (Some(19.3), QUIET_WAIT_SECONDS)
    );
}

/// Where the OS gives no load, a run starts at once.
#[test]
fn a_run_starts_at_once_where_the_os_gives_no_load() {
    assert_eq!(wait(STRACE_QUIET_LOAD, &[]), (None, vec![]));
}
