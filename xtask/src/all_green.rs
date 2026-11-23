//! "all green", the pull-request tier's one required check beside the Chromium
//! build gate (ADRs 0061, 0064 and 0074). It passes only when every job it
//! needs succeeded, and when no earlier attempt of its run had a failed test
//! step, read through GitHub's attempts API: a job is re-run only when no test
//! step failed in it (ADR 0064).
//!
//! A test step is one `workflow::is_test_step` names (TESTING.md, No
//! retries); `xtask no-retry` refuses a step that runs a suite or a test
//! runner under another name.
//!
//! A job `xtask affected` did not select is skipped, and passes: the
//! `affected` job's `jobs` output lists the selected jobs' IDs (ADR 0032).
//! So does a job of the hosted tier when its `plan` job scheduled it no step:
//! the plan's output named after the job is `[]` (ADR 0135's amendment of
//! 2026-11-22).
//! Any other skipped job fails, so a skip never stands in for a pass.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs ban targets async crates (STYLE.md)"
)]

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context as _, Result};
use serde::Deserialize;

use crate::affected;
use crate::check::Finding;
use crate::environment::Environment;
use crate::github::{self, Api, List};
use crate::workflow;

/// The change-detection job, whose `jobs` output is the selection.
const AFFECTED_JOB: &str = "affected";

/// The hosted tier's planning job: its output named after a job is the JSON
/// list of the steps that job runs.
const PLAN_JOB: &str = "plan";

struct Run<'a> {
    api: Api<'a>,
    repository: &'a str,
    id: u64,
    /// This attempt's number, from 1.
    attempt: u32,
}

/// One entry of `${{ toJSON(needs) }}`.
#[derive(Deserialize)]
struct Need {
    result: String,
    #[serde(default)]
    outputs: BTreeMap<String, String>,
}

#[derive(Deserialize)]
struct Job {
    name: String,
    #[serde(default)]
    steps: Vec<Step>,
}

#[derive(Deserialize)]
struct Step {
    name: String,
    conclusion: Option<String>,
}

fn check(needs_file: &Path, run: &Run<'_>) -> Result<Vec<Finding>> {
    let text = std::fs::read_to_string(needs_file)
        .with_context(|| format!("reading {}", needs_file.display()))?;
    let needs: BTreeMap<String, Need> =
        serde_json::from_str(&text).with_context(|| format!("parsing {}", needs_file.display()))?;
    let not_selected = not_selected(&needs)?;
    let mut findings: Vec<Finding> = needs
        .iter()
        .filter(|(job, need)| {
            need.result != "success"
                && !(need.result == "skipped" && not_selected.contains(job.as_str()))
        })
        .map(|(job, need)| {
            Finding::new(
                "job-not-successful",
                job,
                format!("the job's result is `{}`", need.result),
            )
        })
        .collect();
    for attempt in 1..run.attempt {
        for job in attempt_jobs(run, attempt)? {
            let failed_test_step = job.steps.iter().any(|step| {
                workflow::is_test_step(&step.name) && step.conclusion.as_deref() == Some("failure")
            });
            if failed_test_step {
                findings.push(Finding::new(
                    "failed-test-step-in-earlier-attempt",
                    format!("attempt-{attempt}/{}", job.name),
                    "a test step failed in an earlier attempt, so re-running the job cannot make the run green (ADR 0064)",
                ));
            }
        }
    }
    Ok(findings)
}

/// The needed jobs `xtask affected` could have selected and left out, none
/// when the `affected` job did not succeed; and each job of the hosted tier
/// whose `plan` job succeeded and scheduled it no step.
fn not_selected(needs: &BTreeMap<String, Need>) -> Result<BTreeSet<&str>> {
    let mut skippable = BTreeSet::new();
    if let Some(plan) = needs.get(PLAN_JOB)
        && plan.result == "success"
    {
        for job in needs.keys().filter(|job| *job != PLAN_JOB) {
            let Some(steps) = plan.outputs.get(job) else {
                continue;
            };
            let steps: Vec<String> = serde_json::from_str(steps)
                .with_context(|| format!("reading the plan job's `{job}` output"))?;
            if steps.is_empty() {
                skippable.insert(job.as_str());
            }
        }
    }
    let Some(affected) = needs.get(AFFECTED_JOB) else {
        return Ok(skippable);
    };
    if affected.result != "success" {
        return Ok(skippable);
    }
    let jobs = affected
        .outputs
        .get("jobs")
        .context("the affected job has no `jobs` output")?;
    let selected: BTreeSet<String> =
        serde_json::from_str(jobs).context("reading the affected job's `jobs` output")?;
    skippable.extend(needs.keys().map(String::as_str).filter(|job| {
        affected::selectable_job_ids().any(|id| id == *job) && !selected.contains(*job)
    }));
    Ok(skippable)
}

fn attempt_jobs(run: &Run<'_>, attempt: u32) -> Result<Vec<Job>> {
    run.api
        .get_all(
            &format!(
                "/repos/{}/actions/runs/{}/attempts/{attempt}/jobs",
                run.repository, run.id,
            ),
            List::In("jobs"),
        )
        .with_context(|| format!("reading the jobs of attempt {attempt}"))
}

/// `xtask all-green`'s arguments.
#[derive(clap::Args)]
pub(crate) struct Arguments {
    /// A file holding the workflow's `${{ toJSON(needs) }}`.
    #[arg(long)]
    needs: PathBuf,
    #[command(flatten)]
    github: github::Arguments,
    #[arg(long)]
    run_id: u64,
    /// This run attempt's number.
    #[arg(long)]
    attempt: u32,
}

pub(crate) fn run(arguments: &Arguments, environment: &Environment) -> Result<ExitCode> {
    let run = Run {
        api: arguments.github.api(environment.token()),
        repository: &arguments.github.repository,
        id: arguments.run_id,
        attempt: arguments.attempt,
    };
    Ok(crate::check::report(&check(
        &environment.path(&arguments.needs),
        &run,
    )?))
}
