//! The minute ceiling of test health (ADR 0064, amended by ADRs 0072, 0108
//! and 0135; TESTING.md, After merge). The test-health jobs, the diagnostic
//! runs and the coverage job share 1,000 of the account's included minutes
//! a month, counted from the job durations the store records: each job's
//! minutes rounded up, times its runner's rate (`crate::rate`), and nothing
//! for the owner's machine, where the in-diff mutation job runs (ADR 0135),
//! nor for a standard runner of a public repository, which bills nothing
//! (ADR 0064's note of 2026-11-23). The repository's visibility is what
//! GitHub gives the asking run, read now, so a month's jobs all count by
//! today's visibility, as the Release ceiling counts its own (ADR 0108's
//! note of 2026-11-23). Before a run starts, its kind is
//! admitted only while the month leaves room for all its hosted jobs, so
//! the constrained runs stop first, then the diagnostic runs; the coverage
//! job stops once the month reaches 550. The daily test-health job is never
//! asked: its timeout bounds it.

use std::fmt;
use std::path::Path;

use anyhow::Result;
use jiff::civil::Date;
use jiff::tz::TimeZone;

use crate::output;
use crate::rate::{Hosted, Visibility};
use crate::results::{self, JobRun};

/// The included minutes these jobs share each month (ADR 0064).
const CEILING: u64 = 1_000;

/// Where the coverage job stops (ADR 0072).
const COVERAGE_CEILING: u64 = 550;

/// The workflows whose hosted jobs count toward the ceiling.
const COUNTED_WORKFLOWS: [&str; 3] = [
    ".github/workflows/test-health.yml",
    ".github/workflows/diagnostic.yml",
    ".github/workflows/coverage.yml",
];

/// The runs the ceiling admits or skips.
#[derive(Clone, Copy, clap::ValueEnum)]
pub(crate) enum Run {
    /// The constrained test-health run: room for its 120-minute timeout.
    Constrained,
    /// A diagnostic run on hosted Linux: room for the 10-minute timeout of
    /// each of its jobs.
    Diagnostic,
    /// The coverage job: below 550.
    Coverage,
}

impl Run {
    /// Whether a run of `jobs` hosted jobs of this kind fits the month.
    fn admits(self, used: u64, jobs: u64) -> bool {
        match self {
            Self::Constrained => used + 120 * jobs <= CEILING,
            Self::Diagnostic => used + 10 * jobs <= CEILING,
            Self::Coverage => used < COVERAGE_CEILING,
        }
    }
}

impl fmt::Display for Run {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Constrained => "constrained test-health run",
            Self::Diagnostic => "diagnostic run",
            Self::Coverage => "coverage job",
        })
    }
}

/// Prints `minutes=<used>` and `admitted=<true|false>` for a run of `jobs`
/// hosted jobs in the month of `today`, in a repository of `visibility`, and
/// appends a skip to `summary` when there is one.
pub(crate) fn check(
    store: &Path,
    run: Run,
    jobs: u64,
    today: Date,
    visibility: Visibility,
    summary: Option<&Path>,
) -> Result<()> {
    let attempts = results::read_month_and_previous(store, today)?;
    let used: u64 = attempts
        .iter()
        .filter(|attempt| COUNTED_WORKFLOWS.contains(&attempt.run.workflow.as_str()))
        .flat_map(|attempt| {
            attempt.jobs.iter().filter(move |job| {
                let started = job.started_at.unwrap_or(attempt.run.started_at);
                let day = started.to_zoned(TimeZone::UTC).date();
                (day.year(), day.month()) == (today.year(), today.month())
            })
        })
        .map(|job| linux_minutes(job, visibility))
        .sum();
    let admitted = run.admits(used, jobs);
    output::line(&format!("minutes={used}"));
    output::line(&format!("admitted={admitted}"));
    if !admitted && let Some(summary) = summary {
        let next_month = today
            .first_of_month()
            .checked_add(jiff::Span::new().months(1))?;
        let reason = match run {
            Run::Coverage => format!(
                "This month's test-health and coverage jobs have used {used} included minutes, \
                 and the {run} starts only below {COVERAGE_CEILING} (ADR 0072)."
            ),
            Run::Constrained | Run::Diagnostic => format!(
                "This month's test-health jobs have used {used} of their {CEILING} included minutes (ADR 0064), \
                 which leaves no room for the {run}'s {jobs} hosted jobs."
            ),
        };
        output::append(
            summary,
            &format!("### Skipped: the {run}\n\n{reason} It runs again from {next_month}.\n"),
        )?;
    }
    Ok(())
}

/// A job's Linux-equivalent minutes (ADR 0108): its minutes rounded up,
/// times its runner's rate, and none on the owner's machine or when its
/// runner does not bill in a repository of `visibility`.
fn linux_minutes(job: &JobRun, visibility: Visibility) -> u64 {
    match Hosted::of_labels(&job.labels) {
        Some(runner) if !job.self_hosted && runner.bills_in(visibility) => {
            job.duration_seconds.div_ceil(60) * u64::from(runner.rate.multiplier())
        }
        _ => 0,
    }
}
