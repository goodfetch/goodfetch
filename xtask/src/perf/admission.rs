//! Benchmark admission (ADR 0071): a pull request that adds a benchmark or
//! changes its body, which gives it a new ID and so no base, runs it 20
//! times on one build. Every gated figure must spread less than 0.5% across
//! those runs (the highest over the lowest), or the benchmark is fixed
//! before it gates.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs ban targets async crates (STYLE.md)"
)]

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};

use crate::check::Finding;

use super::budget;
use super::compare::{Row, Verdict};
use super::figure::{Sign, hundredths};
use super::gungraun;

/// One new figure across its admission runs.
pub(crate) struct Admission {
    id: String,
    /// The figure's values, one per run that measured it.
    values: Vec<u64>,
}

impl Admission {
    /// The spread in hundredths of a percent, rounded up: none when every
    /// run measured the same, even 0 (a body holding nothing at the
    /// process's heap peak), and without bound when only some measured 0.
    fn spread(&self) -> Option<u64> {
        let lowest = *self.values.iter().min()?;
        let highest = *self.values.iter().max()?;
        if lowest == 0 {
            return Some(if highest == 0 { 0 } else { u64::MAX });
        }
        let spread = u128::from(highest - lowest) * 10_000;
        u64::try_from(spread.div_ceil(u128::from(lowest))).ok()
    }

    fn finding(&self) -> Option<Finding> {
        if self.values.len() < budget::ADMISSION_RUNS {
            return Some(Finding::new(
                "admission-runs",
                &self.id,
                format!(
                    "ran {} times for its admission, not {}",
                    self.values.len(),
                    budget::ADMISSION_RUNS
                ),
            ));
        }
        let spread = self.spread().unwrap_or(u64::MAX);
        (spread >= budget::ADMISSION_SPREAD_BASIS_POINTS).then(|| {
            Finding::new(
                "admission-spread",
                &self.id,
                format!(
                    "spread {} across its admission runs, not under {}; fix the benchmark before it gates",
                    percent(spread),
                    percent(budget::ADMISSION_SPREAD_BASIS_POINTS)
                ),
            )
        })
    }
}

fn percent(basis_points: u64) -> String {
    if basis_points == u64::MAX {
        return "without bound".to_owned();
    }
    format!(
        "{}%",
        hundredths(i128::from(basis_points), Sign::WhenNegative)
    )
}

/// Each new figure of `rows` across the runs in `folder`, one gungraun
/// report (`*.jsonl`) per run.
pub(crate) fn read(rows: &[Row], folder: &Path) -> Result<Vec<Admission>> {
    let mut runs: Vec<PathBuf> = std::fs::read_dir(folder)
        .with_context(|| format!("reading {}", folder.display()))?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<std::io::Result<_>>()
        .with_context(|| format!("reading {}", folder.display()))?;
    runs.retain(|path| {
        path.extension()
            .is_some_and(|extension| extension == "jsonl")
    });
    runs.sort();
    let mut measured_runs = Vec::new();
    for run in &runs {
        measured_runs.push(gungraun::read(&[run.as_path()])?);
    }
    Ok(rows
        .iter()
        .filter(|row| row.verdict == Verdict::New && row.measured.figure.is_benchmark())
        .map(|row| {
            let id = row.measured.id();
            let values = measured_runs
                .iter()
                .filter_map(|run| {
                    run.iter()
                        .find(|measured| measured.id() == id)
                        .map(|measured| measured.head)
                })
                .collect();
            Admission { id, values }
        })
        .collect())
}

pub(crate) fn findings(admissions: &[Admission]) -> Vec<Finding> {
    admissions.iter().filter_map(Admission::finding).collect()
}

/// The admission's own table, under the comparison's.
pub(crate) fn render(admissions: &[Admission]) -> String {
    let mut table = String::from(
        "| New benchmark and figure | Runs | Lowest | Highest | Spread | Verdict |\n\
         |---|---:|---:|---:|---:|---|\n",
    );
    for admission in admissions {
        let lowest = admission.values.iter().min();
        let highest = admission.values.iter().max();
        let _ = writeln!(
            table,
            "| {} | {} | {} | {} | {} | {} |",
            admission.id,
            admission.values.len(),
            lowest.map_or_else(|| "—".to_owned(), u64::to_string),
            highest.map_or_else(|| "—".to_owned(), u64::to_string),
            admission.spread().map_or_else(|| "—".to_owned(), percent),
            if admission.finding().is_some() {
                "not admitted"
            } else {
                "admitted"
            },
        );
    }
    table
}
