//! The Release suite's private-period runs (ADRs 0061 and 0108): the
//! workflow's first job, which chooses the cells a manual dispatch starts
//! under the ceiling of included minutes, and a commit's verdict across its
//! dispatches. Native macOS Intel and Windows arm64, and the merge queue,
//! join once the repository is public.

mod cells;
mod history;
mod plan;
mod skew;
mod time;
mod verdict;

pub(crate) use cells::Cell;
pub(crate) use skew::SKEW_JOBS;

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::Result;

use crate::check::{self, Finding};
use crate::environment::Environment;
use crate::github::{self, Api};
use crate::rate::Visibility;

pub(crate) use plan::{Dispatch, JOB_BUDGET_MINUTES, plan};

/// Whether every cell is green on `commit`, across its dispatches.
pub(crate) fn verdict(api: Api<'_>, repository: &str, commit: &str) -> Result<Vec<Finding>> {
    verdict::verdict(&mut history::History::new(api, repository), commit)
}

/// `repository`'s visibility, as GitHub gives it on its run `run`: when
/// read, not when the run started.
pub(crate) fn visibility(api: Api<'_>, repository: &str, run: u64) -> Result<Visibility> {
    Ok(history::History::new(api, repository).run(run)?.visibility)
}

/// `xtask release-suite`'s commands.
#[derive(clap::Subcommand)]
pub(crate) enum Command {
    /// The Release workflow's first job: checks the commit, chooses the
    /// cells to start and holds the hosted ones back when the run would pass
    /// the month's ceiling of included minutes; writes the job's outputs and
    /// summary.
    Plan {
        #[command(flatten)]
        github: github::Arguments,
        /// This run's ID.
        #[arg(long)]
        run_id: u64,
        /// `GITHUB_REF`, the branch the workflow was dispatched on.
        #[arg(long = "ref")]
        git_ref: String,
        /// `all`, or cell names separated by commas.
        #[arg(long, default_value = "all")]
        cells: String,
        /// On `main`, a commit of `main`; its head when empty.
        #[arg(long)]
        commit: Option<String>,
        /// The file of the job's outputs (`GITHUB_OUTPUT`).
        #[arg(long)]
        outputs: PathBuf,
        /// The job's summary (`GITHUB_STEP_SUMMARY`).
        #[arg(long)]
        summary: PathBuf,
    },
    /// Fails unless every cell is green on the commit, from one or more
    /// dispatches; a failed test step in any attempt leaves it red.
    Verdict {
        #[command(flatten)]
        github: github::Arguments,
        /// The commit of `main` the runs tested.
        #[arg(long)]
        commit: String,
    },
}

pub(crate) fn run(command: Command, environment: &Environment) -> Result<ExitCode> {
    match command {
        Command::Plan {
            github,
            run_id,
            git_ref,
            cells,
            commit,
            outputs,
            summary,
        } => {
            let dispatch = Dispatch {
                repository: &github.repository,
                run_id,
                git_ref: &git_ref,
                cells: &cells,
                commit: commit.as_deref(),
                outputs: &environment.path(&outputs),
                summary: &environment.path(&summary),
            };
            Ok(check::report(&plan(
                github.api(environment.token()),
                &dispatch,
            )?))
        }
        Command::Verdict { github, commit } => Ok(check::report(&verdict(
            github.api(environment.token()),
            &github.repository,
            &commit,
        )?)),
    }
}
