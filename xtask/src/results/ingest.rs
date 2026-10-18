//! One run attempt into the store: the attempt and its jobs from GitHub's
//! interface, and its JUnit artifacts from a folder holding the run's
//! downloaded artifacts, one folder each (`gh run download`). An artifact
//! belongs to the attempt when it was uploaded after the attempt started,
//! since a run's artifacts outlive its attempts. Each JUnit artifact holds
//! `context.json`, which its job writes: the job's ID and the order and
//! `Entropy` seeds it ran with.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs ban targets async crates (STYLE.md)"
)]

use std::collections::BTreeSet;
use std::num::NonZeroU32;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};
use jiff::Timestamp;
use serde::{Deserialize, Serialize};

use super::artifact;
use super::junit::{self, Ending, TestCase};
use super::store::{self, Attempt, JobRun, KeptFile, Outcome, RunAttempt, TestRun};
use crate::github::Api;

/// GitHub's largest page.
const PAGE: usize = 100;

/// The file each JUnit artifact holds beside its JUnit files.
const CONTEXT: &str = "context.json";

/// The tier whose skipped checks are inconclusive (ADR 0064).
const LIVE_TIER: &str = "live";

pub(crate) struct Source<'a> {
    pub(crate) api: Api<'a>,
    pub(crate) repository: &'a str,
    pub(crate) run: u64,
    pub(crate) attempt: u32,
    /// The folder of the run's downloaded artifacts.
    pub(crate) artifacts: &'a Path,
}

#[derive(Deserialize)]
struct ApiAttempt {
    path: String,
    event: String,
    head_branch: Option<String>,
    head_sha: String,
    head_commit: HeadCommit,
    run_started_at: Timestamp,
}

#[derive(Deserialize)]
struct HeadCommit {
    tree_id: String,
}

#[derive(Deserialize)]
struct ApiJobs {
    jobs: Vec<ApiJob>,
}

#[derive(Deserialize)]
struct ApiJob {
    name: String,
    labels: Vec<String>,
    conclusion: Option<String>,
    started_at: Option<Timestamp>,
    completed_at: Option<Timestamp>,
}

#[derive(Deserialize)]
struct ApiArtifacts {
    artifacts: Vec<ApiArtifact>,
}

#[derive(Deserialize)]
struct ApiArtifact {
    name: String,
    created_at: Timestamp,
    expired: bool,
}

/// What a job writes beside its JUnit files.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Context {
    job: String,
    order_seed: Option<String>,
    entropy_seed: Option<String>,
    /// How many times the job ran each test it ran (`xtask repeat`).
    #[serde(default)]
    repeat: Option<NonZeroU32>,
}

/// Writes the attempt's file into the store at `store` and returns its path.
pub(crate) fn ingest(source: &Source<'_>, store: &Path) -> Result<PathBuf> {
    let (attempt, kept) = read(source)?;
    store::write(store, &attempt, &kept)
}

#[derive(Deserialize)]
struct ApiRuns {
    workflow_runs: Vec<ApiRun>,
}

#[derive(Deserialize)]
struct ApiRun {
    id: u64,
    run_attempt: u32,
}

/// Every attempt of the runs that finished since `since` (a day, UTC) that
/// the store at `store` lacks, for the daily ingest (ADR 0135): the runs no
/// per-run ingest follows, such as the pull-request tier's, and any a
/// per-run ingest missed.
pub(crate) fn pending(
    api: &Api<'_>,
    repository: &str,
    store: &Path,
    since: jiff::civil::Date,
) -> Result<Vec<(u64, u32)>> {
    let stored = store::stored(store)?;
    let mut pending = BTreeSet::new();
    for page in 1.. {
        let listed: ApiRuns = api.get(&format!(
            "/repos/{repository}/actions/runs?status=completed&created=%3E%3D{since}&per_page={PAGE}&page={page}"
        ))?;
        let full = listed.workflow_runs.len() == PAGE;
        for run in listed.workflow_runs {
            for attempt in 1..=run.run_attempt {
                if !stored.contains(&(run.id, attempt)) {
                    pending.insert((run.id, attempt));
                }
            }
        }
        if !full {
            break;
        }
    }
    Ok(pending.into_iter().collect())
}

/// A test that failed in the attempt, with where it ran.
#[derive(Serialize, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Failed {
    /// `<runner>=<test ID>`, as `xtask repeat --test` takes it.
    test: String,
    suite: String,
    tier: String,
    os: String,
    arch: String,
    runtime: Option<String>,
}

/// The OS and architecture a diagnostic job runs on, each `None` for any.
pub(crate) struct Platform<'a> {
    pub(crate) os: Option<&'a str>,
    pub(crate) arch: Option<&'a str>,
}

/// The tests that failed in the attempt on `platform`, once each per
/// configuration, of the runners `xtask repeat` repeats; never a Live check's.
pub(crate) fn failed(source: &Source<'_>, platform: &Platform<'_>) -> Result<Vec<Failed>> {
    let (attempt, _) = read(source)?;
    let failed: BTreeSet<Failed> = attempt
        .test_runs
        .into_iter()
        .filter(|run| run.outcome == Outcome::Failed && run.tier != LIVE_TIER)
        .filter(|run| platform.os.is_none_or(|os| run.os == os))
        .filter(|run| platform.arch.is_none_or(|arch| run.arch == arch))
        .filter(|run| ["nextest", "vitest", "playwright", "pytest"].contains(&run.runner.as_str()))
        .map(|run| Failed {
            test: format!("{}={}", run.runner, run.test),
            suite: run.suite,
            tier: run.tier,
            os: run.os,
            arch: run.arch,
            runtime: run.runtime,
        })
        .collect();
    Ok(failed.into_iter().collect())
}

fn read(source: &Source<'_>) -> Result<(Attempt, Vec<KeptFile>)> {
    let base = format!("/repos/{}/actions/runs/{}", source.repository, source.run);
    let attempt_path = format!("{base}/attempts/{}", source.attempt);
    let api_attempt: ApiAttempt = source.api.get(&attempt_path)?;
    let run = RunAttempt {
        repository: source.repository.to_owned(),
        workflow: api_attempt.path,
        run: source.run,
        attempt: source.attempt,
        event: api_attempt.event,
        branch: api_attempt.head_branch,
        commit: api_attempt.head_sha,
        tree: api_attempt.head_commit.tree_id,
        started_at: api_attempt.run_started_at,
    };
    let mut jobs = Vec::new();
    for page in 1.. {
        let listed: ApiJobs = source
            .api
            .get(&format!("{attempt_path}/jobs?per_page={PAGE}&page={page}"))?;
        let full = listed.jobs.len() == PAGE;
        jobs.extend(listed.jobs.into_iter().map(job_run));
        if !full {
            break;
        }
    }
    let mut names = Vec::new();
    for page in 1.. {
        let listed: ApiArtifacts = source
            .api
            .get(&format!("{base}/artifacts?per_page={PAGE}&page={page}"))?;
        let full = listed.artifacts.len() == PAGE;
        names.extend(
            listed
                .artifacts
                .into_iter()
                .filter(|artifact| {
                    artifact::is_junit(&artifact.name)
                        && !artifact.expired
                        && artifact.created_at >= run.started_at
                })
                .map(|artifact| artifact.name),
        );
        if !full {
            break;
        }
    }
    names.sort();
    let mut test_runs = Vec::new();
    let mut kept = Vec::new();
    for name in &names {
        let folder = source.artifacts.join(name);
        if !folder.is_dir() {
            bail!(
                "artifact `{name}` of this attempt was not downloaded into {}",
                source.artifacts.display()
            );
        }
        let (runs, files) = read_artifact(&run, name, &folder)?;
        test_runs.extend(runs);
        kept.extend(files);
    }
    Ok((
        Attempt {
            run,
            test_runs,
            jobs,
        },
        kept,
    ))
}

fn read_artifact(
    run: &RunAttempt,
    name: &str,
    folder: &Path,
) -> Result<(Vec<TestRun>, Vec<KeptFile>)> {
    let configuration = artifact::parse(name)?;
    let mut files = Vec::new();
    collect(folder, folder, &mut files)?;
    files.sort();
    let context_path = folder.join(CONTEXT);
    let context: Context = serde_json::from_slice(
        &std::fs::read(&context_path)
            .with_context(|| format!("artifact `{name}` lacks its {CONTEXT}"))?,
    )
    .with_context(|| format!("parsing {}", context_path.display()))?;
    let mut test_runs = Vec::new();
    let mut kept = Vec::new();
    for relative in files {
        let path = folder.join(&relative);
        let contents =
            std::fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
        let file_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .context("a JUnit file's name is UTF-8")?;
        let is_xml = Path::new(file_name)
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("xml"));
        if is_xml {
            let runner = junit::runner(file_name);
            let runtime = match (
                configuration.runtime.as_deref(),
                junit::file_runtime(file_name),
            ) {
                (Some(artifact), Some(file)) => Some(format!("{artifact}-{file}")),
                (artifact, file) => artifact.or(file).map(str::to_owned),
            };
            let text = std::str::from_utf8(&contents)
                .with_context(|| format!("{} is not UTF-8", path.display()))?;
            let cases = junit::test_cases(runner, text)
                .with_context(|| format!("reading {}", path.display()))?;
            let live = configuration.tier == LIVE_TIER;
            for (test, outcome, duration_seconds) in cases
                .into_iter()
                .flat_map(|case| runs_of(case, runner, context.repeat, live))
            {
                test_runs.push(TestRun {
                    test,
                    runner: runner.to_owned(),
                    suite: configuration.suite.clone(),
                    tier: configuration.tier.clone(),
                    job: context.job.clone(),
                    os: configuration.os.clone(),
                    arch: configuration.arch.clone(),
                    runtime: runtime.clone(),
                    shard: configuration.shard.clone(),
                    commit: run.commit.clone(),
                    tree: run.tree.clone(),
                    workflow: run.workflow.clone(),
                    run: run.run,
                    attempt: run.attempt,
                    started_at: run.started_at,
                    outcome,
                    duration_seconds,
                    order_seed: context.order_seed.clone(),
                    entropy_seed: context.entropy_seed.clone(),
                });
            }
        }
        kept.push(KeptFile {
            path: format!("{name}/{}", relative.replace('\\', "/")),
            contents,
        });
    }
    Ok((test_runs, kept))
}

/// The test runs one JUnit test case records. A test the job repeated
/// (`repeat` in its context) is reported once per run by nextest,
/// Playwright and pytest, whose pytest-repeat adds `[<run>-<count>]` to the
/// name, but once in all by Vitest, with one failure per failed run: those
/// become the failed runs, then the passed ones, sharing its time.
fn runs_of(
    case: TestCase,
    runner: &str,
    repeat: Option<NonZeroU32>,
    live: bool,
) -> Vec<(String, Outcome, f64)> {
    let outcome = match case.ending {
        Ending::Passed => Outcome::Passed,
        Ending::Failed => Outcome::Failed,
        Ending::Skipped if live => Outcome::Inconclusive,
        Ending::Skipped => Outcome::Skipped,
    };
    let Some(count) = repeat else {
        return vec![(case.test, outcome, case.duration_seconds)];
    };
    match runner {
        "vitest" if case.ending != Ending::Skipped => {
            let failed = case.failures.min(count.get());
            let each = case.duration_seconds / f64::from(count.get());
            (0..count.get())
                .map(|run| {
                    let outcome = if run < failed {
                        Outcome::Failed
                    } else {
                        Outcome::Passed
                    };
                    (case.test.clone(), outcome, each)
                })
                .collect()
        }
        "pytest" => {
            let suffix = format!("-{count}]");
            let test = case
                .test
                .strip_suffix(&suffix)
                .and_then(|rest| rest.rsplit_once('['))
                .filter(|(_, run)| !run.is_empty() && run.bytes().all(|byte| byte.is_ascii_digit()))
                .map_or(case.test.clone(), |(test, _)| test.to_owned());
            vec![(test, outcome, case.duration_seconds)]
        }
        _ => vec![(case.test, outcome, case.duration_seconds)],
    }
}

/// Every file under `folder`, relative to `root`.
fn collect(root: &Path, folder: &Path, files: &mut Vec<String>) -> Result<()> {
    for entry in
        std::fs::read_dir(folder).with_context(|| format!("reading {}", folder.display()))?
    {
        let path = entry?.path();
        if path.is_dir() {
            collect(root, &path, files)?;
        } else {
            let relative = path.strip_prefix(root)?.to_string_lossy().into_owned();
            files.push(relative);
        }
    }
    Ok(())
}

fn job_run(job: ApiJob) -> JobRun {
    let self_hosted = job
        .labels
        .iter()
        .any(|label| label.eq_ignore_ascii_case("self-hosted"));
    let duration_seconds = match (job.started_at, job.completed_at) {
        (Some(started), Some(completed)) if completed > started => {
            u64::try_from(completed.as_second() - started.as_second()).unwrap_or(0)
        }
        _ => 0,
    };
    JobRun {
        runner_os: runner_os(&job.labels, self_hosted).to_owned(),
        job: job.name,
        labels: job.labels,
        self_hosted,
        started_at: job.started_at,
        completed_at: job.completed_at,
        duration_seconds,
        conclusion: job.conclusion,
    }
}

/// The OS of a job's runner, from the labels its `runs-on` asked for: a
/// GitHub-hosted image's name, or the owner's machine (macOS, ADR 0108).
fn runner_os(labels: &[String], self_hosted: bool) -> &'static str {
    let has = |prefix: &str| labels.iter().any(|label| label.starts_with(prefix));
    if self_hosted {
        "macos"
    } else if has("ubuntu-") {
        "linux"
    } else if has("windows-") {
        "windows"
    } else if has("macos-") {
        "macos"
    } else {
        "unknown"
    }
}
