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
