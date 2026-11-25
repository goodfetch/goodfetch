use std::cell::Cell;
use std::collections::BTreeSet;

use super::*;
use crate::compare::trials::{QUIET_LOAD, QUIET_WAIT_SECONDS, STRACE_QUIET_LOAD};

/// A reading of a machine whose run's cores are free.
const FREE: Cores = Cores {
    least_idle: 0.99,
    cpu_pressure: 0.5,
    memory_pressure: 0.0,
    leftovers: 0,
};

/// A reading of the gate host while six slots build (2026-11-22: CPU
/// pressure at 60 %).
const BUILDING: Cores = Cores {
    least_idle: 0.12,
    cpu_pressure: 60.0,
    memory_pressure: 0.0,
    leftovers: 0,
};

/// What [`waited`] did given `readings` in turn under `limit`: the reading
/// the run started at, or the error it stopped with, and the seconds each
/// pause slept. Each reading of the cores takes the check's sample, as on
/// the machine.
fn wait(limit: Limit, readings: &[QuietReading]) -> (Result<Option<QuietReading>>, Vec<u64>) {
    let mut readings = readings.iter().copied();
    let clock = Cell::new(0);
    let mut slept = Vec::new();
    let outcome = waited(
        STRACE_QUIET_LOAD,
        limit,
        || {
            let reading = readings.next();
            if let Some(QuietReading::Cores(_)) = reading {
                clock.set(clock.get() + QUIET_SAMPLE_SECONDS);
            }
            reading
        },
        |seconds| {
            slept.push(seconds);
            clock.set(clock.get() + seconds);
        },
        || clock.get(),
    );
    (outcome, slept)
}

/// `Cpus_allowed_list`, as `/proc/self/status` writes it.
#[test]
fn the_cores_a_process_may_use_are_read_from_its_status() {
    let status = "Name:\txtask\nCpus_allowed:\tf0f\nCpus_allowed_list:\t0-3,8,10-11\nVoluntary_ctxt_switches:\t3\n";

    assert_eq!(
        allowed_cores(status),
        Some(BTreeSet::from([0, 1, 2, 3, 8, 10, 11]))
    );
    assert_eq!(allowed_cores("Name:\txtask\n"), None);
}

/// The busiest core of those the run may use sets the reading; a core it
/// may not use is not read.
#[test]
fn the_least_idle_core_of_the_runs_sets_the_reading() {
    let before = "cpu  0 0 0 0 0 0 0 0 0 0\n\
                  cpu0 100 0 0 1000 0 0 0 0 0 0\n\
                  cpu1 100 0 0 1000 0 0 0 0 0 0\n\
                  cpu2 100 0 0 1000 0 0 0 0 0 0\n";
    // In the sample: cpu0 idle 97 of 100 ticks, cpu1 90 of 100, cpu2 0.
    let after = "cpu  0 0 0 0 0 0 0 0 0 0\n\
                 cpu0 102 0 1 1097 0 0 0 0 0 0\n\
                 cpu1 105 0 3 1090 2 0 0 0 0 0\n\
                 cpu2 200 0 0 1000 0 0 0 0 0 0\n";

    let least = least_idle(before, after, &BTreeSet::from([0, 1]));

    assert_eq!(least, Some(0.9));
    assert_eq!(least_idle(before, after, &BTreeSet::from([0])), Some(0.97));
    assert_eq!(least_idle(before, after, &BTreeSet::from([5])), None);
}

/// `avg10` of a pressure file's `some` or `full` line, in percent.
#[test]
fn a_pressure_files_ten_second_average_is_read() {
    let text = "some avg10=1.25 avg60=3.00 avg300=4.00 total=123\n\
                full avg10=0.40 avg60=0.00 avg300=0.00 total=0\n";

    assert_eq!(pressure(text, "some"), Some(1.25));
    assert_eq!(pressure(text, "full"), Some(0.4));
    assert_eq!(pressure("", "some"), None);
}

/// What the previous trial left running: the descendants of the run's own
/// process, at any depth, and not an exited one the OS still lists.
#[test]
fn the_runs_live_descendants_are_its_leftovers() {
    let processes = [
        (10, 1, 'S'),
        (11, 10, 'S'),
        (12, 11, 'R'),
        (13, 11, 'Z'),
        (14, 1, 'R'),
        (15, 14, 'S'),
    ];

    assert_eq!(descendants(&processes, 10), 2);
    assert_eq!(descendants(&processes, 12), 0);
}

/// A machine fails the check on each reading over its threshold, and the
/// failure names every one.
#[test]
fn a_failed_check_names_each_reading_that_failed_it() {
    assert_eq!(QuietReading::Cores(FREE).failed(QUIET_LOAD), None);
    assert_eq!(
        QuietReading::Cores(Cores {
            leftovers: 2,
            ..BUILDING
        })
        .failed(QUIET_LOAD)
        .as_deref(),
        Some(
            "a core of the run's was idle 12.0 % of 3 s, under 97 %; \
             CPU pressure (some avg10) 60.00 %, at or over 2 %; \
             2 processes of the previous trial still run"
        )
    );
    assert_eq!(
        QuietReading::Cores(Cores {
            memory_pressure: 0.5,
            ..FREE
        })
        .failed(QUIET_LOAD)
        .as_deref(),
        Some("memory pressure (full avg10) 0.50 %, at or over 0.5 %")
    );
    assert_eq!(
        QuietReading::Load(4.0).failed(QUIET_LOAD).as_deref(),
        Some("1-minute load 4.00, at or over 4")
    );
    assert_eq!(QuietReading::Load(3.9).failed(QUIET_LOAD), None);
}

/// A run starts once the check passes, after the pauses it took.
#[test]
fn a_run_starts_once_its_cores_are_free() {
    let (outcome, slept) = wait(
        Limit::Within(QUIET_WAIT_SECONDS),
        &[
            QuietReading::Cores(BUILDING),
            QuietReading::Cores(BUILDING),
            QuietReading::Cores(FREE),
        ],
    );

    assert_eq!(outcome.unwrap(), Some(QuietReading::Cores(FREE)));
    assert_eq!(slept, vec![POLL_SECONDS, POLL_SECONDS]);
}

/// Where the OS gives no pressure files, the load decides: a run under
/// strace starts only once it falls under its own threshold (#2521: one
/// started at 19.3 lost its cell).
#[test]
fn a_run_starts_once_the_load_falls_under_its_threshold() {
    let (outcome, slept) = wait(
        Limit::Within(QUIET_WAIT_SECONDS),
        &[QuietReading::Load(19.3), QuietReading::Load(7.5)],
    );

    assert_eq!(outcome.unwrap(), Some(QuietReading::Load(7.5)));
    assert_eq!(slept, vec![POLL_SECONDS]);
}

/// With no time to keep, a machine that stays loaded holds a run back no
/// longer than its wait: it then starts, and says why it was not quiet.
#[test]
fn a_run_on_a_machine_that_stays_loaded_starts_once_its_wait_ends() {
    let readings = vec![QuietReading::Cores(BUILDING); 1000];

    let (outcome, slept) = wait(Limit::Within(QUIET_WAIT_SECONDS), &readings);

    assert_eq!(outcome.unwrap(), Some(QuietReading::Cores(BUILDING)));
    // Each pause follows a reading's sample, and one more reading ends it.
    let samples = QUIET_SAMPLE_SECONDS * (slept.len() as u64 + 1);
    let waited = slept.iter().sum::<u64>() + samples;
    assert!(
        (QUIET_WAIT_SECONDS..QUIET_WAIT_SECONDS + POLL_SECONDS + QUIET_SAMPLE_SECONDS)
            .contains(&waited),
        "{waited}"
    );
}

/// Given the time its waits must end by (the quiet window's end), a run
/// waits up to it, longer than its usual wait, and then starts no trial on
/// a machine that fails the check: it stops, naming the reading.
#[test]
fn a_run_given_an_end_stops_at_it_on_a_machine_still_loaded() {
    let readings = vec![QuietReading::Cores(BUILDING); 1000];

    let (outcome, slept) = wait(Limit::By(2 * QUIET_WAIT_SECONDS), &readings);

    let error = outcome.unwrap_err().to_string();
    assert!(
        error.starts_with("the machine was not quiet by the end given (`--quiet-until`): a core of the run's was idle 12.0 %"),
        "{error}"
    );
    assert!(slept.iter().sum::<u64>() > QUIET_WAIT_SECONDS);
}

/// Given an end, a machine that turns quiet before it starts the run.
#[test]
fn a_run_given_an_end_starts_once_quiet_before_it() {
    let (outcome, _) = wait(
        Limit::By(60),
        &[QuietReading::Cores(BUILDING), QuietReading::Cores(FREE)],
    );

    assert_eq!(outcome.unwrap(), Some(QuietReading::Cores(FREE)));
}

/// Where the OS gives neither pressure nor load, a run starts at once.
#[test]
fn a_run_starts_at_once_where_the_os_gives_no_load() {
    let (outcome, slept) = wait(Limit::By(60), &[]);

    assert_eq!(outcome.unwrap(), None);
    assert_eq!(slept, Vec::<u64>::new());
}

/// `--quiet-until` takes the quiet window's end as `run/quiet-hold` writes
/// it, and gives the seconds left from now, none once it has passed.
#[test]
fn the_end_given_is_read_as_the_quiet_window_writes_it() {
    let until: jiff::Timestamp = "2026-11-26T05:30Z".parse().unwrap();
    let now: jiff::Timestamp = "2026-11-26T05:00Z".parse().unwrap();

    assert_eq!(seconds_left(until, now), 1800);
    assert_eq!(seconds_left(now, until), 0);
}
