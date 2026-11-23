use std::cell::Cell;

use super::{Divergence, Replay, ReplayOutput, check_determinism};
use crate::observation::{ObservationLog, Operation, Record};

/// A run of `seed` whose third record holds `third`, with two lines of
/// `tracing` output, the second holding `traced`.
fn replay(seed: u64, third: u64, traced: &str) -> Replay {
    let mut log = ObservationLog::<u64>::new();
    log.push(Record::new("a", Operation::Other(seed)).at(0));
    log.push(Record::new("b", Operation::Other(seed + 1)).at(5));
    log.push(Record::new("a", Operation::Other(third)).at(9));
    let tracing =
        format!("2026-01-05T09:00:00Z INFO started\n2026-01-05T09:00:00.009Z INFO {traced}\n");
    Replay::new(&log, tracing.as_bytes())
}

#[test]
fn a_seed_that_replays_the_same_passes() {
    let runs = Cell::new(0);
    let checked = check_determinism([3, 4], |seed| {
        runs.set(runs.get() + 1);
        replay(seed, 7, "sent")
    });

    assert_eq!(checked, Ok(()));
    assert_eq!(runs.get(), 4, "each seed runs twice");
}

#[test]
fn a_leak_into_the_observation_log_names_its_first_differing_record() {
    let runs = Cell::new(0_u64);
    let checked = check_determinism([3, 4], |seed| {
        runs.set(runs.get() + 1);
        // Seed 4's second run reads something outside the seed.
        let leaked = if runs.get() == 4 { 8 } else { 7 };
        replay(seed, leaked, "sent")
    });

    let divergence = checked.expect_err("seed 4's runs differ");
    assert_eq!(
        divergence,
        Divergence {
            seed: 4,
            output: ReplayOutput::ObservationLog,
            line: 3,
            first: Some(r#"{"at":9,"host":"a","operation":{"other":7}}"#.to_owned()),
            second: Some(r#"{"at":9,"host":"a","operation":{"other":8}}"#.to_owned()),
        }
    );
    assert_eq!(
        divergence.to_string(),
        "seed 4 does not replay: line 3 of its Observation log differs between two runs\n  first run:  {\"at\":9,\"host\":\"a\",\"operation\":{\"other\":7}}\n  second run: {\"at\":9,\"host\":\"a\",\"operation\":{\"other\":8}}"
    );
}

#[test]
fn a_leak_into_tracing_output_alone_is_found_there() {
    let runs = Cell::new(0_u64);
    let checked = check_determinism([3], |seed| {
        runs.set(runs.get() + 1);
        replay(seed, 7, if runs.get() == 1 { "sent 1" } else { "sent 2" })
    });

    let divergence = checked.expect_err("the second run traced another line");
    assert_eq!(
        (divergence.output, divergence.line),
        (ReplayOutput::Tracing, 2)
    );
}

#[test]
fn a_run_that_stops_early_differs_at_the_record_it_lacks() {
    let runs = Cell::new(0_u64);
    let checked = check_determinism([3], |seed| {
        runs.set(runs.get() + 1);
        if runs.get() == 1 {
            replay(seed, 7, "sent")
        } else {
            let mut log = ObservationLog::<u64>::new();
            log.push(Record::new("a", Operation::Other(seed)).at(0));
            Replay::new(&log, b"")
        }
    });

    let divergence = checked.expect_err("the second run recorded less");
    let shown = divergence.to_string();
    assert_eq!(
        (divergence.output, divergence.line, divergence.second),
        (ReplayOutput::ObservationLog, 2, None)
    );
    assert!(shown.contains("second run: (nothing)"), "{shown}");
}
