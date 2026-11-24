//! The skew jobs a Release run starts (ADR 0066), as the tested commit's
//! generated file names them: `xtask compat jobs` writes it.

use anyhow::{Context as _, Result};
use serde::Deserialize;

use crate::github::Api;

/// The generated jobs, which the plan reads at the tested commit.
pub(crate) const SKEW_JOBS: &str = "ci/release/skew-jobs.json";

#[derive(Deserialize)]
struct SkewJobs {
    jobs: Vec<Job>,
}

#[derive(Deserialize)]
struct Job {
    name: String,
}

/// The names of `commit`'s skew jobs, none when it holds no such file. They
/// come from GitHub because the plan's checkout is the branch's head, which
/// may not be the tested commit.
pub(super) fn jobs(api: Api<'_>, repository: &str, commit: &str) -> Result<Vec<String>> {
    let file: Option<SkewJobs> = api
        .find_file(repository, SKEW_JOBS, commit)
        .with_context(|| format!("reading the skew jobs of {commit}"))?;
    Ok(file
        .into_iter()
        .flat_map(|file| file.jobs)
        .map(|job| job.name)
        .collect())
}
