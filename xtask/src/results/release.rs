//! `main`'s latest Release run, which the Feature report and the Journey
//! report read (ADRs 0063 and 0072): the newest commit of `main` that the
//! Release suite's test runs name in every cell of its matrix, across its
//! dispatches, since a Release run tests the commit its `commit` input names
//! and its cells may come from several dispatches. A commit some of whose
//! cells have not run is not read (ADR 0072, as ADR 0108 amends it).

use super::artifact;
use super::store::{Attempt, TestRun};
use crate::release_suite::Cell;

/// The suite whose runs decide what is tested and proved.
pub(crate) const RELEASE_SUITE: &str = "release";

/// The branch whose Release runs count.
pub(crate) const MAIN: &str = "main";

/// A cell's configuration, as its test runs record it: OS, architecture
/// and runtime.
type Configuration = (String, String, Option<String>);

/// Each cell's configuration, from the JUnit artifact its job uploads.
fn cells() -> Vec<Configuration> {
    Cell::ALL
        .iter()
        .map(|cell| {
            let parsed = artifact::parse(&cell.junit_artifact())
                .expect("invariant: each cell's artifact name parses");
            (parsed.os, parsed.arch, parsed.runtime)
        })
        .collect()
}

/// The commit of `main`'s latest Release run whose every cell ran, and the
/// Release suite's test runs of it, across its dispatches; none when no
/// commit of `main` has a test run in every cell.
pub(crate) fn latest_on_main(attempts: &[Attempt]) -> Option<(String, Vec<&TestRun>)> {
    let runs: Vec<&TestRun> = attempts
        .iter()
        .filter(|attempt| attempt.run.branch.as_deref() == Some(MAIN))
        .flat_map(|attempt| &attempt.test_runs)
        .filter(|run| run.suite == RELEASE_SUITE)
        .collect();
    let cells = cells();
    let every_cell_ran = |commit: &str| {
        cells.iter().all(|(os, arch, runtime)| {
            runs.iter().any(|run| {
                run.commit == commit
                    && run.os == *os
                    && run.arch == *arch
                    && run.runtime == *runtime
            })
        })
    };
    let latest = runs
        .iter()
        .filter(|run| every_cell_ran(&run.commit))
        .max_by_key(|run| (run.started_at, run.run, run.attempt))
        .map(|run| run.commit.clone())?;
    let of_latest = runs
        .into_iter()
        .filter(|run| run.commit == latest)
        .collect();
    Some((latest, of_latest))
}
