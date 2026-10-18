//! "all green", the pull-request tier's one required check beside the Chromium
//! build gate (ADRs 0061, 0064 and 0074). It passes only when every job it
//! needs succeeded, and when no earlier attempt of its run had a failed test
//! step, read through GitHub's attempts API: a job is re-run only when no test
//! step failed in it (ADR 0064).
//!
//! A test step is a step whose name starts with `Test` (TESTING.md, No
//! retries); every step that runs a suite or a test runner is named so.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs ban targets async crates (STYLE.md)"
)]

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context as _, Result};
use serde::Deserialize;

use crate::check::Finding;

/// The prefix that marks a test step.
const TEST_STEP_PREFIX: &str = "Test";

/// GitHub's largest page of jobs.
const PAGE_SIZE: usize = 100;

pub(crate) struct Run<'a> {
    pub(crate) api_url: &'a str,
    pub(crate) token: Option<&'a str>,
    pub(crate) repository: &'a str,
    pub(crate) id: u64,
    /// This attempt's number, from 1.
    pub(crate) attempt: u32,
}

/// One entry of `${{ toJSON(needs) }}`.
#[derive(Deserialize)]
struct Need {
    result: String,
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

pub(crate) fn check(needs_file: &Path, run: &Run<'_>) -> Result<Vec<Finding>> {
    let text = std::fs::read_to_string(needs_file)
        .with_context(|| format!("reading {}", needs_file.display()))?;
    let needs: BTreeMap<String, Need> =
        serde_json::from_str(&text).with_context(|| format!("parsing {}", needs_file.display()))?;
    let mut findings: Vec<Finding> = needs
        .iter()
        .filter(|(_, need)| need.result != "success")
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
                step.name.starts_with(TEST_STEP_PREFIX)
                    && step.conclusion.as_deref() == Some("failure")
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

fn attempt_jobs(run: &Run<'_>, attempt: u32) -> Result<Vec<Job>> {
    let mut jobs = Vec::new();
    for page in 1.. {
        let url = format!(
            "{}/repos/{}/actions/runs/{}/attempts/{attempt}/jobs?per_page={PAGE_SIZE}&page={page}",
            run.api_url.trim_end_matches('/'),
            run.repository,
            run.id,
        );
        let mut request = ureq::get(&url)
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .header("User-Agent", "goodfetch-xtask");
        if let Some(token) = run.token {
            request = request.header("Authorization", format!("Bearer {token}"));
        }
        let page: JobsPage = request
            .call()
            .and_then(|mut response| response.body_mut().read_json())
            .with_context(|| format!("reading the jobs of attempt {attempt} from {url}"))?;
        let last = page.jobs.len() < PAGE_SIZE;
        jobs.extend(page.jobs);
        if last || jobs.len() >= page.total_count {
            break;
        }
    }
    Ok(jobs)
}
