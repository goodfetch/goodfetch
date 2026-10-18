//! The Release workflow's recorded runs and jobs, read through GitHub's
//! Actions interface: the durations the month's count and the estimates are
//! made from (ADR 0108). Until the results store holds them (ADR 0064), the
//! Actions interface is the record, as ADR 0061's budget check reads it.

use std::collections::BTreeMap;

use anyhow::{Context as _, Result};
use serde::Deserialize;

use super::cells::Rate;
use super::time::{Month, Timestamp};
use crate::github::Api;

/// The Release suite's workflow file.
pub(crate) const WORKFLOW: &str = "release-suite.yml";

/// GitHub's largest page.
const PAGE_SIZE: usize = 100;

#[derive(Deserialize)]
struct RunJson {
    id: u64,
    created_at: String,
    head_branch: Option<String>,
    head_sha: String,
}

#[derive(Deserialize)]
struct RunsPage {
    total_count: usize,
    workflow_runs: Vec<RunJson>,
}

#[derive(Deserialize)]
struct JobJson {
    name: String,
    #[serde(default)]
    labels: Vec<String>,
    run_attempt: Option<u32>,
    started_at: Option<String>,
    completed_at: Option<String>,
    conclusion: Option<String>,
    #[serde(default)]
    steps: Vec<StepJson>,
}

#[derive(Deserialize)]
struct StepJson {
    name: String,
    conclusion: Option<String>,
}

#[derive(Deserialize)]
struct JobsPage {
    total_count: usize,
    jobs: Vec<JobJson>,
}

/// A run of the Release workflow.
pub(crate) struct Run {
    pub(crate) id: u64,
    pub(crate) month: Month,
    pub(crate) branch: String,
    pub(crate) head_sha: String,
}

impl TryFrom<RunJson> for Run {
    type Error = anyhow::Error;

    fn try_from(run: RunJson) -> Result<Self> {
        Ok(Self {
            id: run.id,
            month: Month::of(&run.created_at)?,
            branch: run.head_branch.unwrap_or_default(),
            head_sha: run.head_sha,
        })
    }
}

/// How a job ended, or that it has not.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Ended {
    Success,
    /// Failed, timed out or cancelled after it started.
    Failure,
    /// Skipped, or cancelled before it started: it ran nothing.
    NotRun,
    /// Queued or running.
    Pending,
}

/// A job of one attempt of a run.
pub(crate) struct Job {
    pub(crate) name: String,
    /// `None` on the owner's machine, which uses no included minutes.
    pub(crate) rate: Option<Rate>,
    pub(crate) attempt: u32,
    /// The whole minutes it ran, once it ended after starting.
    pub(crate) minutes: Option<u32>,
    pub(crate) ended: Ended,
    /// Whether a step whose name starts with `Test` failed (ADR 0064).
    pub(crate) failed_test_step: bool,
}

impl TryFrom<JobJson> for Job {
    type Error = anyhow::Error;

    fn try_from(job: JobJson) -> Result<Self> {
        let started = job
            .started_at
            .as_deref()
            .map(Timestamp::parse)
            .transpose()?;
        let completed = job
            .completed_at
            .as_deref()
            .map(Timestamp::parse)
            .transpose()?;
        let ended = match job.conclusion.as_deref() {
            None => Ended::Pending,
            Some("success") => Ended::Success,
            Some("skipped") => Ended::NotRun,
            Some(_) if started.is_none() || started == completed => Ended::NotRun,
            Some(_) => Ended::Failure,
        };
        let minutes = match (ended, started, completed) {
            (Ended::Success | Ended::Failure, Some(started), Some(completed)) => {
                Some(started.whole_minutes_until(completed))
            }
            _ => None,
        };
        let failed_test_step = job.steps.iter().any(|step| {
            step.name.starts_with("Test") && step.conclusion.as_deref() == Some("failure")
        });
        Ok(Self {
            rate: Rate::of_labels(&job.labels)
                .with_context(|| format!("reading the runner of job `{}`", job.name))?,
            name: job.name,
            attempt: job.run_attempt.unwrap_or(1),
            minutes,
            ended,
            failed_test_step,
        })
    }
}

/// The Release workflow's runs and jobs, each read once.
pub(crate) struct History<'a> {
    api: Api<'a>,
    repository: &'a str,
    jobs: BTreeMap<u64, Vec<Job>>,
}

impl<'a> History<'a> {
    pub(crate) fn new(api: Api<'a>, repository: &'a str) -> Self {
        Self {
            api,
            repository,
            jobs: BTreeMap::new(),
        }
    }

    /// Whether `commit` is on `head`'s history: `head` is the commit itself
    /// or ahead of it.
    pub(crate) fn contains(&self, head: &str, commit: &str) -> Result<bool> {
        #[derive(Deserialize)]
        struct Comparison {
            status: String,
        }
        let comparison: Comparison = self
            .api
            .get(&format!(
                "/repos/{}/compare/{commit}...{head}",
                self.repository
            ))
            .with_context(|| format!("comparing {commit} with {head}"))?;
        Ok(matches!(comparison.status.as_str(), "ahead" | "identical"))
    }

    pub(crate) fn run(&self, id: u64) -> Result<Run> {
        let run: RunJson = self
            .api
            .get(&format!("/repos/{}/actions/runs/{id}", self.repository))
            .with_context(|| format!("reading run {id}"))?;
        Run::try_from(run)
    }

    /// The workflow's runs, newest first, filtered by `filter` (a
    /// `key=value` query pair) when there is one.
    pub(crate) fn runs(&self, filter: Option<&str>) -> Result<Vec<Run>> {
        let filter = filter.map(|pair| format!("{pair}&")).unwrap_or_default();
        let mut runs = Vec::new();
        for page in 1.. {
            let page: RunsPage = self
                .api
                .get(&format!(
                    "/repos/{}/actions/workflows/{WORKFLOW}/runs?{filter}per_page={PAGE_SIZE}&page={page}",
                    self.repository,
                ))
                .context("listing the Release workflow's runs")?;
            let last = page.workflow_runs.len() < PAGE_SIZE;
            for run in page.workflow_runs {
                runs.push(Run::try_from(run)?);
            }
            if last || runs.len() >= page.total_count {
                break;
            }
        }
        Ok(runs)
    }

    /// Every job of every attempt of a run.
    pub(crate) fn jobs(&mut self, run: u64) -> Result<&[Job]> {
        if !self.jobs.contains_key(&run) {
            let mut jobs = Vec::new();
            for page in 1.. {
                let page: JobsPage = self
                    .api
                    .get(&format!(
                        "/repos/{}/actions/runs/{run}/jobs?filter=all&per_page={PAGE_SIZE}&page={page}",
                        self.repository
                    ))
                    .with_context(|| format!("reading the jobs of run {run}"))?;
                let last = page.jobs.len() < PAGE_SIZE;
                for job in page.jobs {
                    jobs.push(Job::try_from(job)?);
                }
                if last || jobs.len() >= page.total_count {
                    break;
                }
            }
            self.jobs.insert(run, jobs);
        }
        Ok(self.jobs.get(&run).map_or(&[], Vec::as_slice))
    }
}
