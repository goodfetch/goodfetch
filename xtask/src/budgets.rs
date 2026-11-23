//! `xtask budgets`: ADR 0061's budget check (TESTING.md, Time budgets). It
//! reads each job's last 20 runs in its own tier from the results store
//! (ADR 0064), takes their median (p50) and compares it with the Time
//! budgets table; a breached suite or job opens or updates its one
//! `needs-triage` issue, with the 20 durations. It never fails a run: a
//! breach and a tier with no budget row are reported, and the exit code
//! stays 0.
//!
//! The tiers, while the repository is private: the pull-request tier's
//! runs, on the hosted tier's standard runners (ADR 0135's amendment of
//! 2026-11-22), for the Quick and Pre-merge suites; manual-dispatch Release runs, on hosted
//! runners only, for the Release suite (ADR 0108). Only successful jobs are
//! measured, since a failed one ends early; a suite's whole-run budget is
//! measured on first attempts, since a re-run's carried jobs keep their
//! first start.

#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs ban targets async crates (STYLE.md)"
)]

mod table;

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context as _, Result};
use jiff::Timestamp;

use crate::check::{self, Finding};
use crate::environment::Environment;
use crate::issues::{Issues, Label};
use crate::results::JobRun;
use crate::{github, output, results};

use table::{Measure, Table};

#[derive(clap::Args)]
pub(crate) struct Arguments {
    /// A checkout of the results store.
    #[arg(long)]
    store: PathBuf,
    /// The repository whose issues the check keeps, and GitHub's API.
    #[command(flatten)]
    github: github::Arguments,
}

/// The runs a budget's median is taken over (ADR 0061).
const RUNS: usize = 20;

/// The file holding the table, from the workspace's folder: TESTING.md's
/// "Suites, commands and time budgets" section.
const TESTING: &str = "docs/testing/suites-commands-and-time-budgets.md";

/// Which of its workflow's jobs a tier holds, by name.
enum Jobs {
    Only(&'static [&'static str]),
    AllBut(&'static [&'static str]),
}

impl Jobs {
    fn holds(&self, name: &str) -> bool {
        match self {
            Self::Only(names) => names.contains(&name),
            Self::AllBut(names) => !names.contains(&name),
        }
    }
}

/// What a suite's whole-run budget measures in one run.
#[derive(Clone, Copy)]
enum Whole {
    /// From the run's first job's start to the end of the job of this name.
    AllGreen(&'static str),
    /// From the first job's start to the last job's end.
    Group,
}

impl Whole {
    fn measure(self) -> Measure {
        match self {
            Self::AllGreen(_) => Measure::AllGreen,
            Self::Group => Measure::Group,
        }
    }
}

/// One suite's runs in its own tier, and the row of the Time budgets table
/// that holds its budgets.
struct Tier {
    suite: &'static str,
    /// The table's "Where".
    place: &'static str,
    workflow: &'static str,
    event: &'static str,
    jobs: Jobs,
    /// Whether jobs on a self-hosted runner stay out of its budgets.
    hosted_only: bool,
    whole: Option<Whole>,
}

const PULL_REQUEST: &str = ".github/workflows/pull-request.yml";
const RELEASE_SUITE: &str = ".github/workflows/release-suite.yml";

/// The Chromium build gate holds pinned tests in no suite (TESTING.md, The
/// three suites), so it has budgets of its own.
const CHROMIUM_BUILD_GATE: &str = "Chromium build gate";

/// The hosted tier's job of the Quick suite itself, one cell of the `quick`
/// matrix; its other cells, lint and the Pre-merge jobs count toward the
/// Pre-merge suite's budgets.
const QUICK: &str = "quick (quick)";

const TIERS: [Tier; 3] = [
    Tier {
        suite: "Quick",
        place: "CI, whole workspace",
        workflow: PULL_REQUEST,
        event: "pull_request",
        jobs: Jobs::Only(&[QUICK]),
        hosted_only: false,
        whole: None,
    },
    Tier {
        suite: "Pre-merge",
        place: "Pull request",
        workflow: PULL_REQUEST,
        event: "pull_request",
        jobs: Jobs::AllBut(&[QUICK, CHROMIUM_BUILD_GATE]),
        hosted_only: false,
        whole: Some(Whole::AllGreen("all green")),
    },
    Tier {
        suite: "Release",
        place: "Merge queue",
        workflow: RELEASE_SUITE,
        event: "workflow_dispatch",
        jobs: Jobs::AllBut(&[]),
        hosted_only: true,
        whole: Some(Whole::Group),
    },
];

/// One measured run of a job or a suite.
struct Duration {
    run: u64,
    attempt: u32,
    started_at: Timestamp,
    seconds: u64,
    /// The runner's labels, so a run on the owner's machine says so.
    runner: String,
}

/// A job or suite, its budget and its last runs, newest first.
struct Measured {
    /// What the issue's marker names.
    key: String,
    title: String,
    budget: u64,
    durations: Vec<Duration>,
}

impl Measured {
    /// Twice the median, so an even count's median needs no rounding.
    fn twice_median(&self) -> u64 {
        let mut seconds: Vec<u64> = self.durations.iter().map(|run| run.seconds).collect();
        seconds.sort_unstable();
        let middle = seconds.len() / 2;
        match (seconds.len() % 2, seconds.get(middle)) {
            (1, Some(&value)) => value * 2,
            (_, Some(&value)) => value + seconds.get(middle - 1).copied().unwrap_or(value),
            (_, None) => 0,
        }
    }
}

pub(crate) fn run(arguments: &Arguments, environment: &Environment) -> Result<ExitCode> {
    let testing = environment.root.join(TESTING);
    let table = Table::read(
        &std::fs::read_to_string(&testing)
            .with_context(|| format!("reading {}", testing.display()))?,
    );
    let attempts = results::read_all(&environment.path(&arguments.store))?;
    let api = arguments.github.api(environment.token());
    let mut issues = Issues::new(&api, &arguments.github.repository);
    for tier in &TIERS {
        let runs: Vec<(&results::RunAttempt, &[JobRun])> = attempts
            .iter()
            .filter(|attempt| {
                attempt.run.workflow == tier.workflow && attempt.run.event == tier.event
            })
            .map(|attempt| (&attempt.run, attempt.jobs.as_slice()))
            .collect();
        for measured in measure(tier, &table, &runs) {
            judge(&measured, &mut issues)?;
        }
    }
    for action in issues.actions() {
        output::line(action);
    }
    Ok(ExitCode::SUCCESS)
}

/// Each job's and the suite's last runs in the tier, with their budgets; a
/// missing budget is reported instead.
fn measure(
    tier: &Tier,
    table: &Table,
    runs: &[(&results::RunAttempt, &[JobRun])],
) -> Vec<Measured> {
    let missing = |what: &str| {
        check::note(&[Finding::new(
            "missing-budget",
            format!("{}, {}", tier.suite, tier.place),
            format!("{TESTING}'s Time budgets has {what}"),
        )]);
    };
    let Some(row) = table.row(tier.suite, tier.place) else {
        missing("no row for it");
        return Vec::new();
    };
    let mut measured = Vec::new();
    match row.budget(Measure::Job) {
        Some(budget) => measured.extend(jobs(tier, budget, runs)),
        None => missing("no budget for each job in its row"),
    }
    if let Some(whole) = tier.whole {
        match row.budget(whole.measure()) {
            Some(budget) => measured.extend(whole_runs(tier, whole, budget, runs)),
            None => missing("no budget for the whole suite in its row"),
        }
    }
    measured
}

/// The jobs of a run the tier counts: finished, and hosted when it counts
/// only hosted ones.
fn counted<'a>(tier: &Tier, jobs: &'a [JobRun]) -> impl Iterator<Item = &'a JobRun> {
    let hosted_only = tier.hosted_only;
    jobs.iter()
        .filter(move |job| !(hosted_only && job.self_hosted) && job.started_at.is_some())
}

fn succeeded(job: &JobRun) -> bool {
    job.conclusion.as_deref() == Some("success")
}

fn jobs(tier: &Tier, budget: u64, runs: &[(&results::RunAttempt, &[JobRun])]) -> Vec<Measured> {
    let mut seen = BTreeSet::new();
    let mut by_job: BTreeMap<&str, Vec<Duration>> = BTreeMap::new();
    for (run, jobs) in runs {
        for job in counted(tier, jobs).filter(|job| tier.jobs.holds(&job.job) && succeeded(job)) {
            let Some(started_at) = job.started_at else {
                continue;
            };
            // A re-run attempt carries the jobs it did not run again.
            if !seen.insert((run.run, job.job.as_str(), started_at)) {
                continue;
            }
            by_job.entry(&job.job).or_default().push(Duration {
                run: run.run,
                attempt: run.attempt,
                started_at,
                seconds: job.duration_seconds,
                runner: job.labels.join(", "),
            });
        }
    }
    let workflow = file_name(tier.workflow);
    by_job
        .into_iter()
        .map(|(job, durations)| Measured {
            key: format!("time-budget job {workflow} {job}"),
            title: format!(
                "Time budget: `{job}` ({} suite, {}) over {}",
                tier.suite,
                tier.place,
                duration(budget)
            ),
            budget,
            durations: latest(durations),
        })
        .collect()
}

fn whole_runs(
    tier: &Tier,
    whole: Whole,
    budget: u64,
    runs: &[(&results::RunAttempt, &[JobRun])],
) -> Option<Measured> {
    let durations: Vec<Duration> = runs
        .iter()
        .filter(|(run, _)| run.attempt == 1)
        .filter_map(|(run, jobs)| {
            let jobs: Vec<&JobRun> = counted(tier, jobs).collect();
            let first = jobs.iter().filter_map(|job| job.started_at).min()?;
            let end = match whole {
                Whole::AllGreen(name) => {
                    jobs.iter()
                        .find(|job| job.job == name && succeeded(job))?
                        .completed_at?
                }
                Whole::Group => {
                    if !jobs.iter().all(|job| succeeded(job)) {
                        return None;
                    }
                    jobs.iter().filter_map(|job| job.completed_at).max()?
                }
            };
            // Counting such a run as 0 s would pull the median under its
            // budget, so it is left out, and said so.
            let Ok(seconds) = u64::try_from(first.duration_until(end).as_secs()) else {
                check::note(&[Finding::new(
                    "negative-span",
                    format!("{}, run {}", tier.suite, run.run),
                    "its last job ends before its first starts; left out of the median",
                )]);
                return None;
            };
            let mut runners: Vec<String> = jobs.iter().map(|job| job.labels.join(", ")).collect();
            runners.sort();
            runners.dedup();
            Some(Duration {
                run: run.run,
                attempt: run.attempt,
                started_at: first,
                seconds,
                runner: runners.join("; "),
            })
        })
        .collect();
    if durations.is_empty() {
        return None;
    }
    let title = match whole {
        Whole::AllGreen(name) => format!(
            "Time budget: the {} suite's \"{name}\" over {}",
            tier.suite,
            duration(budget)
        ),
        Whole::Group => format!(
            "Time budget: a {} suite run ({}) over {} per group",
            tier.suite,
            tier.place,
            duration(budget)
        ),
    };
    Some(Measured {
        key: format!("time-budget suite {}", tier.suite),
        title,
        budget,
        durations: latest(durations),
    })
}

/// The last [`RUNS`] of `durations`, newest first.
fn latest(mut durations: Vec<Duration>) -> Vec<Duration> {
    durations.sort_by_key(|run| std::cmp::Reverse((run.started_at, run.run, run.attempt)));
    durations.truncate(RUNS);
    durations
}

/// Reports `measured` against its budget, and keeps its issue when the
/// median is past it.
fn judge(measured: &Measured, issues: &mut Issues<'_>) -> Result<()> {
    let Some(newest) = measured.durations.first() else {
        return Ok(());
    };
    let twice_median = measured.twice_median();
    let median = duration(twice_median.div_ceil(2));
    let budget = duration(measured.budget);
    let count = measured.durations.len();
    if twice_median <= measured.budget * 2 {
        output::line(&format!(
            "within {}: p50 {median} of {count} runs, budget {budget}",
            measured.key
        ));
        return Ok(());
    }
    output::line(&format!(
        "over {}: p50 {median} of {count} runs, budget {budget}",
        measured.key
    ));
    let mut body = format!(
        "The median (p50) of the last {count} runs is {median}, over the budget of {budget} \
         in {TESTING}'s Time budgets. Fix it within two weeks: shard the job, speed up its \
         tests, or move a test to a later suite with the reason in its pull request (ADR 0061).\n\n\
         | Run | Attempt | Started | Duration | Runner |\n|---|---|---|---|---|\n"
    );
    for run in &measured.durations {
        let _ = writeln!(
            body,
            "| {} | {} | {} | {} | {} |",
            run.run,
            run.attempt,
            run.started_at,
            duration(run.seconds),
            run.runner
        );
    }
    issues.keep(
        Label::NeedsTriage,
        &measured.key,
        &measured.title,
        &body,
        newest.started_at,
        // A budget belongs to no spec's test: its issue goes to triage.
        || Ok(None),
    )
}

/// `seconds` in hours, minutes and seconds, leaving out the zero ones.
fn duration(seconds: u64) -> String {
    let parts: Vec<String> = [
        (seconds / 3_600, "h"),
        (seconds / 60 % 60, "min"),
        (seconds % 60, "s"),
    ]
    .into_iter()
    .filter(|(count, _)| *count > 0)
    .map(|(count, unit)| format!("{count} {unit}"))
    .collect();
    if parts.is_empty() {
        "0 s".to_owned()
    } else {
        parts.join(" ")
    }
}

fn file_name(workflow: &str) -> &str {
    Path::new(workflow)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(workflow)
}
