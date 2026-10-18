//! The minute ceiling of test health while the repository is private (ADR
//! 0064, amended by ADRs 0072 and 0108; TESTING.md, After merge). The
//! test-health jobs, the diagnostic runs and the coverage and in-diff
//! mutation jobs share 1,000 of the account's included minutes a month,
//! counted from the job durations the store records: each job's minutes
//! rounded up, times its runner's multiplier, and nothing for the owner's
//! machine. Before a run starts, its kind is admitted only while the month
//! leaves it room, so the constrained runs stop first, then the diagnostic
//! runs; the coverage and mutation jobs stop once the month reaches 550.
//! The daily test-health job is never asked: its timeout bounds it.

use std::fmt;
use std::io::Write as _;
use std::path::Path;

use anyhow::{Context as _, Result};
use jiff::civil::Date;
use jiff::tz::TimeZone;

use crate::output;
use crate::results::{self, JobRun};

/// The included minutes these jobs share each month (ADR 0064).
const CEILING: u64 = 1_000;

/// Where the coverage and in-diff mutation jobs stop (ADR 0072).
const COVERAGE_CEILING: u64 = 550;

/// The workflows whose hosted jobs count toward the ceiling. The coverage
/// and in-diff mutation jobs' workflows join them with ADR 0072's ticket.
const COUNTED_WORKFLOWS: [&str; 2] = [
    ".github/workflows/test-health.yml",
    ".github/workflows/diagnostic.yml",
];

/// The runs the ceiling admits or skips.
#[derive(Clone, Copy, clap::ValueEnum)]
pub(crate) enum Run {
    /// The constrained test-health run: room for its 120-minute timeout.
    Constrained,
    /// A diagnostic run on hosted Linux: room for its 10-minute timeout.
    Diagnostic,
    /// The coverage job: below 550.
    Coverage,
    /// The in-diff mutation job on hosted runners: below 550.
    Mutation,
}

impl Run {
    fn admits(self, used: u64) -> bool {
        match self {
            Self::Constrained => used + 120 <= CEILING,
            Self::Diagnostic => used + 10 <= CEILING,
            Self::Coverage | Self::Mutation => used < COVERAGE_CEILING,
        }
    }
}

impl fmt::Display for Run {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Constrained => "constrained test-health run",
            Self::Diagnostic => "diagnostic run",
            Self::Coverage => "coverage job",
            Self::Mutation => "in-diff mutation job",
        })
    }
}

/// Prints `minutes=<used>` and `admitted=<true|false>` for the month of
/// `today`, and appends a skip to `summary` when there is one.
pub(crate) fn check(store: &Path, run: Run, today: Date, summary: Option<&Path>) -> Result<()> {
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
        .map(linux_minutes)
        .sum();
    let admitted = run.admits(used);
    output::line(&format!("minutes={used}"));
    output::line(&format!("admitted={admitted}"));
    if !admitted && let Some(summary) = summary {
        let next_month = today
            .first_of_month()
            .checked_add(jiff::Span::new().months(1))?;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(summary)
            .with_context(|| format!("opening {}", summary.display()))?;
        writeln!(
            file,
            "### Skipped: the {run}\n\nThis month's test-health jobs have used {used} of their {CEILING} included minutes (ADR 0064), \
             which leaves no room for the {run}. It runs again from {next_month}."
        )
        .with_context(|| format!("writing {}", summary.display()))?;
    }
    Ok(())
}

/// A job's Linux-equivalent minutes (ADR 0108): its minutes rounded up,
/// times 1 on Linux, 2 on Windows and 11 on macOS, and none on the owner's
/// machine. A hosted runner of no known image counts as macOS, never less.
fn linux_minutes(job: &JobRun) -> u64 {
    if job.self_hosted {
        return 0;
    }
    let has = |prefix: &str| job.labels.iter().any(|label| label.starts_with(prefix));
    let multiplier = if has("ubuntu-") {
        1
    } else if has("windows-") {
        2
    } else {
        11
    };
    job.duration_seconds.div_ceil(60) * multiplier
}
