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
use crate::github::{self, Api};
use crate::workflow;

/// The change-detection job, whose `jobs` output is the selection.
const AFFECTED_JOB: &str = "affected";

/// GitHub's largest page of jobs.
const PAGE_SIZE: usize = 100;

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
struct JobsPage {
    total_count: usize,
    jobs: Vec<Job>,
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
/// when the `affected` job did not succeed.
fn not_selected(needs: &BTreeMap<String, Need>) -> Result<BTreeSet<&str>> {
    let Some(affected) = needs.get(AFFECTED_JOB) else {
        return Ok(BTreeSet::new());
    };
    if affected.result != "success" {
        return Ok(BTreeSet::new());
    }
    let jobs = affected
        .outputs
        .get("jobs")
        .context("the affected job has no `jobs` output")?;
    let selected: BTreeSet<String> =
        serde_json::from_str(jobs).context("reading the affected job's `jobs` output")?;
    Ok(needs
        .keys()
        .map(String::as_str)
        .filter(|job| {
            affected::selectable_job_ids().any(|id| id == *job) && !selected.contains(*job)
        })
        .collect())
}

fn attempt_jobs(run: &Run<'_>, attempt: u32) -> Result<Vec<Job>> {
    let api = run.api;
    let mut jobs = Vec::new();
    for page in 1.. {
        let page: JobsPage = api
            .get(&format!(
                "/repos/{}/actions/runs/{}/attempts/{attempt}/jobs?per_page={PAGE_SIZE}&page={page}",
                run.repository, run.id,
            ))
            .with_context(|| format!("reading the jobs of attempt {attempt}"))?;
        let last = page.jobs.len() < PAGE_SIZE;
        jobs.extend(page.jobs);
        if last || jobs.len() >= page.total_count {
            break;
        }
    }
    Ok(jobs)
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
