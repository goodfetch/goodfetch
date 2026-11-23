//! A commit's Release suite verdict while the repository is private (ADRs
//! 0064 and 0108): it passes when every cell of the private-period matrix is
//! green on it, from one or more dispatches. A cell is green when the
//! latest attempt that ran it passed; a failed test step in any attempt
//! leaves it red for good, since a test is never retried.

use anyhow::Result;

use super::cells::Cell;
use super::history::{Ended, History};
use crate::check::Finding;

/// Where one cell stands on the commit.
enum State {
    Green,
    Red,
    /// Failed with no failed test step: held back, timed out in the queue
    /// or broken before its tests, so it may run again.
    Failed,
    Pending,
    NotRun,
}

pub(crate) fn verdict(history: &mut History<'_>, commit: &str) -> Result<Vec<Finding>> {
    let runs = history.runs(Some(&format!("head_sha={commit}")))?;
    let mut findings = Vec::new();
    for cell in Cell::ALL {
        let mut state = State::NotRun;
        let mut decided = false;
        // Runs are listed newest first: the first run that ran the cell
        // holds its latest attempt.
        for run in &runs {
            let jobs: Vec<_> = history
                .jobs(run.id)?
                .iter()
                .filter(|job| cell.owns_job(&job.name))
                .collect();
            if jobs.iter().any(|job| job.failed_test_step) {
                state = State::Red;
                break;
            }
            if decided {
                continue;
            }
            let ran: Vec<_> = jobs
                .iter()
                .filter(|job| job.ended != Ended::NotRun)
                .collect();
            let Some(attempt) = ran.iter().map(|job| job.attempt).max() else {
                continue;
            };
            let latest: Vec<_> = ran.iter().filter(|job| job.attempt == attempt).collect();
            state = if latest.iter().any(|job| job.ended == Ended::Pending) {
                State::Pending
            } else if latest.iter().all(|job| job.ended == Ended::Success) {
                State::Green
            } else {
                State::Failed
            };
            decided = true;
        }
        let (kind, message) = match state {
            State::Green => continue,
            State::Red => (
                "cell-red",
                "a test step failed in one of its attempts, so the commit is red (ADR 0064)",
            ),
            State::Failed => (
                "cell-failed",
                "its latest attempt failed with no failed test step; dispatch the cell again",
            ),
            State::Pending => ("cell-pending", "it is queued or running"),
            State::NotRun => (
                "cell-not-run",
                "no dispatch ran it on this commit; dispatch it with `just release-ci`",
            ),
        };
        findings.push(Finding::new(kind, cell.key(), message));
    }
    Ok(findings)
}
