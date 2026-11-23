//! The Release workflow's first job (ADR 0108): it checks the commit a
//! dispatch asks for, chooses the cells to start and, before any hosted cell
//! starts, checks the run against the 600-minute ceiling a month, counted in
//! Linux-equivalent minutes from the month's recorded Release jobs. Only
//! minutes that bill count: in a public repository a standard hosted runner
//! counts none (ADR 0108's note of 2026-11-23). It writes its choice as the
//! job's outputs and its reasons to the job's summary.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::Path;

use anyhow::{Context as _, Result};

use super::cells::{self, Cell, LinuxJob, Runner};
use super::history::{Ended, History, Run};
use super::time::Month;
use crate::check::Finding;
use crate::github::Api;
use crate::issues::{Issues, Label};
use crate::output;
use crate::rate::Visibility;

/// The minutes that bill a month, included minutes or billed ones, that
/// manual-dispatch Release runs' hosted jobs may use (ADR 0108).
pub(crate) const CEILING: u32 = 600;

/// ADR 0061's budget for a Release job, which estimates a cell with no
/// recorded run: 45 minutes, at its runner's rate.
const JOB_BUDGET_MINUTES: u32 = 45;

pub(crate) const MAIN_REF: &str = "refs/heads/main";

/// The title of the `needs-triage` issue a run over the ceiling on its own
/// opens or updates.
const ISSUE_TITLE: &str = "A Release run's estimate passes the 600-minute ceiling";

/// The marker key of that issue.
const ISSUE_KEY: &str = "release-ceiling";

pub(crate) struct Dispatch<'a> {
    pub(crate) repository: &'a str,
    /// This run, whose month and head the plan reads.
    pub(crate) run_id: u64,
    /// `GITHUB_REF`: the branch the workflow was dispatched on.
    pub(crate) git_ref: &'a str,
    /// `all`, or cell names separated by commas.
    pub(crate) cells: &'a str,
    /// On `main`, a commit of `main` to test; its head when empty.
    pub(crate) commit: Option<&'a str>,
    /// `GITHUB_OUTPUT`.
    pub(crate) outputs: &'a Path,
    /// `GITHUB_STEP_SUMMARY`.
    pub(crate) summary: &'a Path,
}

/// What the ceiling lets the run's hosted cells do.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Decision {
    Start,
    /// The run's own estimate passes the ceiling, but it is the month's
    /// first run on `main` whose hosted cells start: it starts, and an issue
    /// says so.
    StartOverCeiling,
    Held,
}

/// The month's count and the run's estimate, in Linux-equivalent minutes.
struct Count {
    used: u32,
    estimate: u32,
    /// What must fit under the ceiling for the run's hosted cells to start.
    needed: u32,
}

pub(crate) fn plan(api: Api<'_>, dispatch: &Dispatch<'_>) -> Result<Vec<Finding>> {
    let requested = cells::parse_selection(dispatch.cells)?;
    let mut history = History::new(api, dispatch.repository);
    let this = history.run(dispatch.run_id)?;
    let on_main = dispatch.git_ref == MAIN_REF;
    let commit = match (on_main, dispatch.commit.filter(|commit| !commit.is_empty())) {
        (false, Some(commit)) => {
            return Ok(vec![Finding::new(
                "commit-off-main",
                commit,
                "a commit can be chosen only on `main`; off `main` the run tests the branch's pushed head (ADR 0108)",
            )]);
        }
        (_, None) => this.head_sha.clone(),
        (true, Some(commit)) if commit == this.head_sha => this.head_sha.clone(),
        (true, Some(commit)) => {
            if !history.contains(&this.head_sha, commit)? {
                return Ok(vec![Finding::new(
                    "commit-not-on-main",
                    commit,
                    "the commit is not on `main`'s history, so no job checks it out (ADR 0108)",
                )]);
            }
            commit.to_owned()
        }
    };

    let runs = history.runs(None)?;
    let estimates = estimates(&mut history, &runs, &this)?;
    let hosted: Vec<Cell> = requested
        .iter()
        .copied()
        .filter(|cell| matches!(cell.runner(), Runner::Hosted(_)))
        .collect();
    let bills = hosted.iter().any(
        |cell| matches!(cell.runner(), Runner::Hosted(runner) if runner.bills_in(this.visibility)),
    );
    let owner_machine: Vec<Cell> = requested
        .iter()
        .copied()
        .filter(|cell| cell.runner() == Runner::OwnerMachine && on_main)
        .collect();
    let used = used(&mut history, &runs, &this, &estimates)?;
    let estimate: u32 = hosted.iter().map(|cell| estimates[cell]).sum();
    let main_run: u32 = estimates.values().sum();
    let count = Count {
        used,
        estimate,
        needed: if on_main {
            used + estimate
        } else {
            used + estimate + main_run
        },
    };
    // A run whose hosted cells bill nothing adds nothing to the month's
    // count, so the ceiling never holds it.
    let decision = if !bills || count.needed <= CEILING {
        Decision::Start
    } else if on_main
        && estimate > CEILING
        && !hosted_cells_started_on_main(&mut history, &runs, &this)?
    {
        Decision::StartOverCeiling
    } else {
        Decision::Held
    };

    let started: Vec<Cell> = Cell::ALL
        .into_iter()
        .filter(|cell| {
            owner_machine.contains(cell) || (decision != Decision::Held && hosted.contains(cell))
        })
        .collect();
    write_outputs(
        dispatch.outputs,
        &commit,
        &started,
        decision,
        &count,
        this.month,
    )?;
    write_summary(
        dispatch.summary,
        &Summary {
            commit: &commit,
            requested: &requested,
            started: &started,
            on_main,
            visibility: this.visibility,
            decision,
            count: &count,
            month: this.month,
        },
    )?;
    if decision == Decision::StartOverCeiling {
        keep_issue(api, dispatch.repository, &this, &count)?;
    }
    Ok(Vec::new())
}

/// Each hosted cell's estimate: the minutes its jobs took in the latest
/// attempt of its latest recorded run, or one job budget before any (270
/// for every hosted cell, ADR 0108), as its runner counts them in this
/// run's repository.
fn estimates(history: &mut History<'_>, runs: &[Run], this: &Run) -> Result<BTreeMap<Cell, u32>> {
    let mut estimates = BTreeMap::new();
    for cell in Cell::ALL {
        let Runner::Hosted(runner) = cell.runner() else {
            continue;
        };
        let mut recorded = None;
        for run in runs.iter().filter(|run| run.id != this.id) {
            let jobs: Vec<_> = history
                .jobs(run.id)?
                .iter()
                .filter(|job| {
                    cell.owns_job(&job.name) && matches!(job.ended, Ended::Success | Ended::Failure)
                })
                .collect();
            let Some(attempt) = jobs.iter().map(|job| job.attempt).max() else {
                continue;
            };
            recorded = Some(
                jobs.iter()
                    .filter(|job| job.attempt == attempt)
                    .map(|job| job.minutes.unwrap_or(0))
                    .sum(),
            );
            break;
        }
        estimates.insert(
            cell,
            runner.counted(recorded.unwrap_or(JOB_BUDGET_MINUTES), this.visibility),
        );
    }
    Ok(estimates)
}

/// The month's count: every hosted job of the month's Release runs, in
/// whole minutes at its rate, as its runner counts them in this run's
/// repository. A job still queued or running counts as its cell's estimate,
/// once per run, and any other as one minute.
///
/// The repository's visibility is read now, not when each job ran, so in a
/// public repository a standard runner's minutes from before it was public
/// count none either (ADR 0108's note of 2026-11-23).
fn used(
    history: &mut History<'_>,
    runs: &[Run],
    this: &Run,
    estimates: &BTreeMap<Cell, u32>,
) -> Result<u32> {
    let mut used = 0;
    for run in runs.iter().filter(|run| run.month == this.month) {
        let mut pending_cells = BTreeSet::new();
        for job in history.jobs(run.id)? {
            let Some(runner) = job.runner else {
                continue;
            };
            used += match job.ended {
                Ended::Success | Ended::Failure => {
                    runner.counted(job.minutes.unwrap_or(0), this.visibility)
                }
                Ended::NotRun => 0,
                Ended::Pending => match Cell::ALL.into_iter().find(|cell| cell.owns_job(&job.name))
                {
                    Some(cell) if pending_cells.insert(cell) => {
                        estimates.get(&cell).copied().unwrap_or(0)
                    }
                    Some(_) => 0,
                    None => runner.counted(1, this.visibility),
                },
            };
        }
    }
    Ok(used)
}

/// Whether an earlier Release run on `main` this month started a hosted cell.
fn hosted_cells_started_on_main(
    history: &mut History<'_>,
    runs: &[Run],
    this: &Run,
) -> Result<bool> {
    for run in runs
        .iter()
        .filter(|run| run.id != this.id && run.month == this.month && run.branch == "main")
    {
        let started = history.jobs(run.id)?.iter().any(|job| {
            job.ended != Ended::NotRun
                && Cell::ALL.into_iter().any(|cell| {
                    matches!(cell.runner(), Runner::Hosted(_)) && cell.owns_job(&job.name)
                })
        });
        if started {
            return Ok(true);
        }
    }
    Ok(false)
}

/// The job's outputs, which the cells' jobs read: the commit, the started
/// cells, each Linux job's matrix, and the count.
fn write_outputs(
    path: &Path,
    commit: &str,
    started: &[Cell],
    decision: Decision,
    count: &Count,
    month: Month,
) -> Result<()> {
    let keys: Vec<&str> = started.iter().map(|cell| cell.key()).collect();
    let mut text = format!(
        "commit={commit}\ncells={}\n",
        serde_json::to_string(&keys).context("writing the cells")?
    );
    for job in LinuxJob::ALL {
        let matrix: Vec<serde_json::Value> = started
            .iter()
            .filter(|cell| cell.linux_job() == Some(job))
            .map(|cell| cell.matrix_entry())
            .collect();
        let _ = writeln!(
            text,
            "{}={}",
            job.output(),
            serde_json::to_string(&matrix).context("writing a Linux matrix")?
        );
    }
    let _ = write!(
        text,
        "held={}\nused={}\nestimate={}\nneeded={}\nmonth-turns={}\n",
        decision == Decision::Held,
        count.used,
        count.estimate,
        count.needed,
        month.next()
    );
    output::append(path, &text)
}

struct Summary<'a> {
    commit: &'a str,
    requested: &'a [Cell],
    started: &'a [Cell],
    on_main: bool,
    visibility: Visibility,
    decision: Decision,
    count: &'a Count,
    month: Month,
}

fn write_summary(path: &Path, summary: &Summary<'_>) -> Result<()> {
    let free = match summary.visibility {
        Visibility::Public => "; standard hosted runners of this public repository count none",
        Visibility::Private => "",
    };
    let mut text = format!(
        "## Release suite plan\n\nCommit `{}`. Minutes that bill this month, in Linux-equivalent minutes (Linux 1, Windows 2, macOS 11{free}): {} used, this run's hosted cells estimated at {}, ceiling {CEILING} (ADR 0108).\n\n| Cell | Where | This run |\n|---|---|---|\n",
        summary.commit, summary.count.used, summary.count.estimate
    );
    for cell in Cell::ALL {
        let (place, state) = match cell.runner() {
            Runner::Hosted(_) => ("GitHub-hosted", "held back by the ceiling"),
            Runner::OwnerMachine => ("the owner's machine", "not run: off `main`"),
        };
        let state = if summary.started.contains(&cell) {
            "started"
        } else if !summary.requested.contains(&cell) {
            "not asked for"
        } else {
            state
        };
        let _ = writeln!(text, "| {} | {place} | {state} |", cell.label());
    }
    match summary.decision {
        Decision::Held => {
            let _ = write!(
                text,
                "\nNo hosted cell started: the run needs {} minutes that bill{}, past the ceiling of {CEILING}. The month turns on {}.\n",
                summary.count.needed,
                if summary.on_main {
                    ""
                } else {
                    ", a full run on `main` included"
                },
                summary.month.next()
            );
        }
        Decision::StartOverCeiling => {
            let _ = write!(
                text,
                "\nThis run's own estimate passes the ceiling; as the month's first Release run on `main` whose hosted cells start, it starts anyway, and a `needs-triage` issue says so. Merge shards or speed up the tests.\n"
            );
        }
        Decision::Start => {}
    }
    output::append(path, &text)
}

/// Opens the `needs-triage` issue, or brings it up to date, as a budget
/// breach does (ADR 0061), found again by its marker.
fn keep_issue(api: Api<'_>, repository: &str, run: &Run, count: &Count) -> Result<()> {
    let body = format!(
        "Release run {} estimated its hosted cells at {} Linux-equivalent minutes, past the ceiling of {CEILING} minutes that bill a month (ADR 0108). It started as the month's first Release run on `main` whose hosted cells start. Merge shards or speed up the tests (ADR 0061).",
        run.id, count.estimate
    );
    let mut issues = Issues::new(&api, repository);
    issues
        .keep(
            Label::NeedsTriage,
            ISSUE_KEY,
            ISSUE_TITLE,
            &body,
            run.created_at,
        )
        .context("opening or updating the ceiling's `needs-triage` issue")
}
