use super::order;
use crate::compare::matrix;

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
        super::stopped(1, 300, stall - 1, 240),
        super::stopped(1, bound - 1, 0, 990),
    );
    let stalled = super::stopped(1, 400, stall, 240);
    let bounded = super::stopped(2, bound, 3, 990);

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
