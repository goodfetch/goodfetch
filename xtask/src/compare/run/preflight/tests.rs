use anyhow::anyhow;

use super::{Probe, Under, flown, probes};
use crate::compare::matrix::{self, Cell, Config, Scenario};
use crate::compare::trial::cell_of;

/// The cell `name` of `scenario`, with its configuration.
fn cell(scenario: Scenario, name: &str) -> (&'static Cell, Config) {
    cell_of(scenario, name).expect("invariant: the matrix measures this cell")
}

/// Each probe as its scenario, its cell and what it runs under.
fn named(probes: &[Probe]) -> Vec<(&'static str, &'static str, Under)> {
    probes
        .iter()
        .map(|probe| (probe.cell.scenario.name(), probe.cell.name, probe.under))
        .collect()
}

/// The pre-flight runs each measured cell plain, then under each counting
/// tool the run will run it under, each default-cost cell under Callgrind
/// alone, the one tool that runs it (#2438), and each hostile pages' and
/// auto cell in its own pass: no tool the run starts goes unproved.
#[test]
fn the_pre_flight_runs_each_cell_under_each_tool_the_run_runs_it_under() {
    let foxhound = cell(Scenario::Raw, "foxhound");
    let spider = cell(Scenario::Raw, "spider");
    let records = cell(Scenario::Raw, "foxhound records on");
    let long = matrix::measured()
        .find(|(cell, _)| cell.scenario == Scenario::Long)
        .expect("invariant: the long scenario has a measured cell");
    let ruled = matrix::ruled()
        .next()
        .expect("invariant: the matrix has a hostile pages' cell");
    let auto = matrix::auto()
        .next()
        .expect("invariant: the matrix has an auto cell");

    let listed = probes(
        &[foxhound, spider, long],
        &[foxhound, spider],
        &[foxhound, records],
        &[ruled],
        &[auto],
    );

    assert_eq!(
        named(&listed),
        vec![
            ("raw", "foxhound", Under::Plain),
            ("raw", "foxhound", Under::Strace),
            ("raw", "foxhound", Under::Callgrind),
            ("raw", "spider", Under::Plain),
            ("raw", "spider", Under::Strace),
            ("long", long.0.name, Under::Plain),
            ("raw", "foxhound records on", Under::Callgrind),
            (ruled.0.scenario.name(), ruled.0.name, Under::Rules),
            (auto.0.scenario.name(), auto.0.name, Under::Auto),
        ]
    );
}

/// A broken Callgrind invocation fails the pre-flight with the tool's own
/// error, under the cell and the tool, and no later probe runs: the run
/// stops before its first timed trial (#2521).
#[test]
fn a_broken_callgrind_invocation_fails_the_pre_flight_at_its_probe() {
    let foxhound = cell(Scenario::Raw, "foxhound");
    let spider = cell(Scenario::Raw, "spider");
    let listed = probes(&[foxhound, spider], &[foxhound], &[foxhound], &[], &[]);
    let mut flew = Vec::new();

    // A stand-in for the trial, whose valgrind cannot write its output.
    let ended = flown(&listed, |probe| {
        flew.push(probe.under);
        match probe.under {
            Under::Callgrind => Err(anyhow!(
                "valgrind: cannot create log file target/compare/callgrind/raw-foxhound-0.out"
            )),
            Under::Plain | Under::Strace | Under::Rules | Under::Auto => Ok(()),
        }
    });

    let error = format!("{:#}", ended.expect_err("the pre-flight fails"));
    assert_eq!(
        (flew, error),
        (
            vec![Under::Plain, Under::Strace, Under::Callgrind],
            "the pre-flight's run of raw foxhound under Callgrind failed, before the first timed trial: valgrind: cannot create log file target/compare/callgrind/raw-foxhound-0.out".to_owned()
        )
    );
}
