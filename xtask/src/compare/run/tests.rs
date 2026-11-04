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
