//! `main`'s latest Release run, which the Feature report and the Journey
//! report read (ADRs 0063 and 0072): the newest commit of `main` that the
//! Release suite's test runs name, across its dispatches, since a Release
//! run tests the commit its `commit` input names.

use super::store::{Attempt, TestRun};

/// The suite whose runs decide what is tested and proved.
pub(crate) const RELEASE_SUITE: &str = "release";

/// The branch whose Release runs count.
pub(crate) const MAIN: &str = "main";

/// The commit of `main`'s latest Release run and the Release suite's test
/// runs of it, across its dispatches; none when no attempt holds a Release
/// run of `main`.
pub(crate) fn latest_on_main(attempts: &[Attempt]) -> Option<(String, Vec<&TestRun>)> {
    let runs: Vec<&TestRun> = attempts
        .iter()
        .filter(|attempt| attempt.run.branch.as_deref() == Some(MAIN))
        .flat_map(|attempt| &attempt.test_runs)
        .filter(|run| run.suite == RELEASE_SUITE)
        .collect();
    let latest = runs
        .iter()
        .max_by_key(|run| (run.started_at, run.run, run.attempt))
        .map(|run| run.commit.clone())?;
    let of_latest = runs
        .into_iter()
        .filter(|run| run.commit == latest)
        .collect();
    Some((latest, of_latest))
}
