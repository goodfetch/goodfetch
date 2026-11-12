use std::sync::Arc;

use super::RunIndex;

#[test]
fn a_second_start_of_a_run_id_joins_the_first_starts_reservation() {
    let mut index = RunIndex::default();
    let first = index.reserve("run-1");

    let second = index.reserve("run-1");

    assert!(Arc::ptr_eq(&first, &second));
}

#[test]
fn a_reservation_whose_start_started_no_run_is_forgotten() {
    let mut index = RunIndex::default();
    let failed = index.reserve("run-1");
    index.release("run-1", &failed);

    let again = index.reserve("run-1");

    assert!(!Arc::ptr_eq(&failed, &again));
}

#[test]
fn a_reservation_another_start_joined_stays_when_one_start_fails() {
    let mut index = RunIndex::default();
    let failed = index.reserve("run-1");
    let joined = index.reserve("run-1");
    index.release("run-1", &failed);

    let again = index.reserve("run-1");

    assert!(Arc::ptr_eq(&joined, &again));
}
