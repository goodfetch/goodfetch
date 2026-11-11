use std::path::Path;

use super::{counted_output, order, spider_executable};
use crate::compare::matrix::{self, SpiderBuild};

/// Each of Spider's side's builds runs the executable Cargo left in the
/// build's own target folder, under the profile's: the main row's from
/// `spider`, a feature's from `spider-<feature>`.
#[test]
fn each_spider_build_runs_the_executable_built_in_its_own_folder() {
    let executable = |build| spider_executable(Path::new("/t/compare-spider"), "release", build);

    assert_eq!(
        (
            executable(SpiderBuild::Http),
            executable(SpiderBuild::Chrome),
            executable(SpiderBuild::Jemalloc),
        ),
        (
            Path::new("/t/compare-spider/spider/release/goodfetch-compare-spider").to_owned(),
            Path::new("/t/compare-spider/spider-chrome/release/goodfetch-compare-spider")
                .to_owned(),
            Path::new("/t/compare-spider/spider-jemalloc/release/goodfetch-compare-spider")
                .to_owned(),
        )
    );
}

/// Each Callgrind run's output is kept in the run's folder under a name of
/// its own, its scenario, cell and number, with no space in it, so a later
/// run of the same cell does not replace it and the reports repository
/// files it (#2281).
#[test]
fn each_callgrind_run_keeps_its_output_under_its_scenario_cell_and_number() {
    let cell = |name| {
        matrix::cell(matrix::Scenario::Raw, name).expect("invariant: scenario 1 has this cell")
    };
    let kept = Path::new("/t/compare/callgrind");

    assert_eq!(
        (
            counted_output(kept, cell("foxhound"), 1),
            counted_output(kept, cell("foxhound"), 3),
            counted_output(kept, cell("spider capacity 4096"), 2),
        ),
        (
            Path::new("/t/compare/callgrind/raw-foxhound-1.out").to_owned(),
            Path::new("/t/compare/callgrind/raw-foxhound-3.out").to_owned(),
            Path::new("/t/compare/callgrind/raw-spider-capacity-4096-2.out").to_owned(),
        )
    );
}

/// Every measured cell's 10 trials run once each, in one order that mixes
/// the cells: no cell's trials all run before another's start.
#[test]
fn each_cells_trials_run_once_each_interleaved_with_the_others() {
    let cells: Vec<_> = matrix::measured().collect();
    let mut state = 7;

    let order = order(&cells, 10, &mut state);

    let mut ran: Vec<(&str, u32)> = order
        .iter()
        .map(|(cell, _, number)| (cell.name, *number))
        .collect();
    let first_half: Vec<&str> = ran[..ran.len() / 2].iter().map(|(name, _)| *name).collect();
    let mixed = cells
        .iter()
        .all(|(cell, _)| first_half.contains(&cell.name));
    ran.sort_unstable();
    let mut expected: Vec<(&str, u32)> = cells
        .iter()
        .flat_map(|(cell, _)| (1..=10).map(move |number| (cell.name, number)))
        .collect();
    expected.sort_unstable();
    assert_eq!((ran, mixed), (expected, true));
}

/// Only a run of the matrix's full trials waits for a quiet machine before
/// each timed trial: a quicker try, which its report flags as short of the
/// 10, never sleeps, so a check that the cells run holds no slot idle.
#[test]
fn only_a_run_of_the_full_trials_waits_for_a_quiet_machine() {
    let waits = |trials| {
        super::waits_for_quiet(&super::Options {
            trials,
            growth: false,
            spider_pages: None,
            scenarios: crate::compare::matrix::Scenario::ALL.to_vec(),
            callgrind_outputs: Path::new("/t/compare/callgrind").to_owned(),
        })
    };

    assert_eq!((waits(10), waits(3), waits(1)), (true, false, false));
}

/// A counted run goes on while its crawl progresses: it is stopped, with the
/// measured reason, once the Proving ground has received no request for
/// [`super::STALL_SECONDS`], or once it reaches
/// [`super::COUNTED_BOUND_SECONDS`].
#[test]
fn a_counted_run_is_stopped_once_its_crawl_stalls_or_reaches_the_bound() {
    let stall = super::STALL_SECONDS;
    let bound = super::COUNTED_BOUND_SECONDS;

    let going = (
        super::stopped(super::Watch::Counted(1), 300, stall - 1, 240),
        super::stopped(super::Watch::Counted(1), bound - 1, 0, 990),
    );
    let stalled = super::stopped(super::Watch::Counted(1), 400, stall, 240);
    let bounded = super::stopped(super::Watch::Counted(2), bound, 3, 990);

    assert_eq!(
        (going, stalled, bounded),
        (
            (None, None),
            Some(format!(
                "under `strace -f`, run 1 stopped after 400 s: the Proving ground received no request for {stall} s after 240 requests"
            )),
            Some(format!(
                "under `strace -f`, run 2 stopped after {bound} s, the longest a counted run may take, at 990 requests"
            )),
        )
    );
}

/// A timed trial gets the same watch: stopped once the Proving ground has
/// received no request for [`super::STALL_SECONDS`], or once it reaches
/// [`super::TIMED_BOUND_SECONDS`] for the 1,000-page site (scaled to a
/// larger site's pages), with the measured reason.
#[test]
fn a_timed_trial_is_stopped_once_its_crawl_stalls_or_reaches_its_bound() {
    use super::Watch;
    let stall = super::STALL_SECONDS;
    let bound = super::TIMED_BOUND_SECONDS;

    let going = (
        super::stopped(Watch::Timed(3, 1000), 200, stall - 1, 10),
        super::stopped(Watch::Timed(3, 100_000), bound + 1, 0, 990),
    );
    let stalled = super::stopped(Watch::Timed(3, 1000), 130, stall, 0);
    let bounded = super::stopped(Watch::Timed(4, 1000), bound, 2, 990);

    assert_eq!(
        (going, stalled, bounded),
        (
            (None, None),
            Some(format!(
                "timed trial 3 stopped after 130 s: the Proving ground received no request for {stall} s after 0 requests"
            )),
            Some(format!(
                "timed trial 4 stopped after {bound} s, the longest a timed trial may take, at 990 requests"
            )),
        )
    );
}

/// A stopped timed trial runs once more: its retry's figures count, marked
/// with why the first stopped; a second stop leaves the cell with both
/// reasons, and a trial that finishes is never run again.
#[test]
fn a_stopped_timed_trial_is_retried_once() {
    use super::{Outcome, Retried, retried};
    let run = |outcomes: Vec<Outcome<u32>>| {
        let mut outcomes = outcomes.into_iter();
        let mut attempts = 0;
        let result = retried(|| {
            attempts += 1;
            Ok(outcomes.next().expect("no more attempts than outcomes"))
        })
        .unwrap();
        let shown = match result {
            Retried::Done { measured, retried } => format!("done {measured} {retried:?}"),
            Retried::Stopped(reason) => format!("stopped: {reason}"),
        };
        (shown, attempts)
    };

    assert_eq!(
        (
            run(vec![Outcome::Done(7)]),
            run(vec![Outcome::Stopped("stall".to_owned()), Outcome::Done(8)]),
            run(vec![
                Outcome::Stopped("stall".to_owned()),
                Outcome::Stopped("bound".to_owned()),
            ]),
        ),
        (
            ("done 7 None".to_owned(), 1),
            ("done 8 Some(\"stall\")".to_owned(), 2),
            ("stopped: bound; its first attempt: stall".to_owned(), 2),
        )
    );
}
