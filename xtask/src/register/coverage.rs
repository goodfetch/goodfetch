//! `xtask register coverage`, the last step of ADR 0072's coverage job: adds
//! up the job's lcov reports and compares the project's line coverage with the
//! `coverage.project` row of TESTING.md's Quality register. A fall past the
//! tolerance opens or updates one `needs-triage` issue, as `xtask budgets`
//! does, and so does a row with no figure yet, naming its first one. No figure
//! fails it or any merge: only a report or a row it cannot read does.
//!
//! `--write` puts the figure in the row of the checkout it runs in, for the
//! pull request that records it (TESTING.md: only that writes a Today value);
//! it never lowers the row.

use std::fmt::Write as _;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context as _, Result};
use jiff::Timestamp;

use crate::check::{self, Finding};
use crate::environment::Environment;
use crate::issues::{Issues, Label};
use crate::{github, output};

use super::document::{self, FILE as TESTING, MEASURED, Register};

#[derive(clap::Args)]
pub(crate) struct Arguments {
    /// The folder of the job's reports, `target/coverage` as `just coverage`
    /// writes it.
    #[arg(long)]
    reports: PathBuf,
    /// Writes the figure in the row when the row has none or a lower one.
    #[arg(long)]
    write: bool,
    /// The repository whose issues the job keeps, and GitHub's API.
    #[command(flatten)]
    github: github::Arguments,
}

/// The row holding the figure.
const ROW: &str = "coverage.project";

/// How far the figure may fall, as a share of the row's: the register's
/// tolerance for a figure that varies (TESTING.md, Quality register, Rules).
const TOLERANCE: f64 = 0.005;

/// Each language's report, from the reports' folder (`ci/coverage.sh`).
const REPORTS: [(&str, &str); 3] = [
    ("Rust", "rust/lcov.info"),
    ("TypeScript", "vitest/lcov.info"),
    ("Python", "python/lcov.info"),
];

/// The lines of a report, and those a test ran.
#[derive(Clone, Copy, Default)]
struct Lines {
    hit: u32,
    found: u32,
}

impl Lines {
    /// The two counts added up; `None` past what a `u32` holds.
    fn plus(self, other: Self) -> Option<Self> {
        Some(Self {
            hit: self.hit.checked_add(other.hit)?,
            found: self.found.checked_add(other.found)?,
        })
    }

    /// The share of lines a test ran, in per cent.
    fn percent(self) -> f64 {
        f64::from(self.hit) / f64::from(self.found) * 100.0
    }
}

/// One language's lines.
struct Report {
    language: &'static str,
    lines: Lines,
}

pub(crate) fn run(arguments: &Arguments, environment: &Environment) -> Result<ExitCode> {
    let testing = environment.root.join(TESTING);
    let text = std::fs::read_to_string(&testing)
        .with_context(|| format!("reading {}", testing.display()))?;
    let today = Register::read(&text).and_then(|register| {
        register
            .measured
            .iter()
            .find(|row| row.counter() == Some(ROW))
            .map(|row| row.cell("Today").to_owned())
    });
    let (reports, mut findings) = read_reports(&environment.path(&arguments.reports))?;
    if today.is_none() {
        findings.push(Finding::new(
            "coverage-row-missing",
            ROW,
            format!("{TESTING}'s Measured table has no such row"),
        ));
    }
    let Some(today) = today.filter(|_| findings.is_empty()) else {
        return Ok(check::report(&findings));
    };
    let total = reports
        .iter()
        .try_fold(Lines::default(), |total, report| total.plus(report.lines))
        .context("adding up the reports' lines")?;
    let figure = total.percent();
    let recorded = document::figure(&today);
    output::line(&format!(
        "{ROW}: {figure:.2} % of {} lines, the row holds {today}",
        total.found
    ));
    if arguments.write && recorded.is_none_or(|recorded| figure > recorded) {
        let written = document::with_cell(&text, MEASURED, ROW, "Today", &format!("{figure:.2} %"))
            .with_context(|| format!("writing {ROW} in {}", testing.display()))?;
        std::fs::write(&testing, written)
            .with_context(|| format!("writing {}", testing.display()))?;
        output::line(&format!("wrote {figure:.2} % in {ROW}"));
        return Ok(ExitCode::SUCCESS);
    }
    let (key, title, body) = match recorded {
        None => (
            format!("{ROW} first figure"),
            format!("Coverage: write `{ROW}`'s first figure"),
            first_body(figure, &reports),
        ),
        Some(recorded) if figure < recorded * (1.0 - TOLERANCE) => (
            ROW.to_owned(),
            format!("Coverage: `{ROW}` fell"),
            fall_body(figure, recorded, &reports),
        ),
        Some(_) => return Ok(ExitCode::SUCCESS),
    };
    let api = arguments.github.api(environment.token());
    let mut issues = Issues::new(&api, &arguments.github.repository);
    // The project's coverage belongs to no spec: its issue goes to triage.
    issues.keep(
        Label::NeedsTriage,
        &key,
        &title,
        &body,
        Timestamp::now(),
        || Ok(None),
    )?;
    for action in issues.actions() {
        output::line(action);
    }
    Ok(ExitCode::SUCCESS)
}

/// Each language's report, and a finding for each one missing or unreadable:
/// a figure without a language would read as a fall, or as a rise.
fn read_reports(folder: &Path) -> Result<(Vec<Report>, Vec<Finding>)> {
    let mut reports = Vec::new();
    let mut findings = Vec::new();
    for (language, file) in REPORTS {
        let path = folder.join(file);
        match std::fs::read_to_string(&path) {
            Ok(text) => match lines(&text).filter(|lines| lines.found > 0) {
                Some(lines) => reports.push(Report { language, lines }),
                None => findings.push(Finding::new(
                    "coverage-report-unreadable",
                    language,
                    format!("{} holds no line counts it can read", path.display()),
                )),
            },
            Err(error) if error.kind() == ErrorKind::NotFound => findings.push(Finding::new(
                "coverage-report-missing",
                language,
                format!("no {}", path.display()),
            )),
            Err(error) => {
                return Err(error).with_context(|| format!("reading {}", path.display()));
            }
        }
    }
    Ok((reports, findings))
}

/// The `LF` and `LH` records of an lcov report added up; `None` when one is
/// no number.
fn lines(report: &str) -> Option<Lines> {
    report
        .lines()
        .try_fold(Lines::default(), |lines, line| match line.split_once(':') {
            Some(("LF", count)) => lines.plus(Lines {
                hit: 0,
                found: count.parse().ok()?,
            }),
            Some(("LH", count)) => lines.plus(Lines {
                hit: count.parse().ok()?,
                found: 0,
            }),
            _ => Some(lines),
        })
}

fn fall_body(figure: f64, recorded: f64, reports: &[Report]) -> String {
    format!(
        "The coverage job measured {figure:.2} % of the project's lines, under the {recorded:.2} % \
         of `{ROW}` in {TESTING}'s Quality register and past its tolerance of 0.5 %. Read the \
         run's `coverage-report` for what no test runs now (ADR 0072). No merge fails on this.\n\n\
         {}",
        table(reports)
    )
}

fn first_body(figure: f64, reports: &[Report]) -> String {
    format!(
        "The coverage job measured {figure:.2} % of the project's lines, and `{ROW}` in \
         {TESTING}'s Quality register has no figure yet. In a pull request, unpack the run's \
         `coverage-report` into `target/coverage` and run `xtask register coverage --reports \
         target/coverage --write`, which writes it in the row. Later runs compare with it.\n\n{}",
        table(reports)
    )
}

fn table(reports: &[Report]) -> String {
    let mut table = "| Language | Lines run | Lines |\n|---|---|---|\n".to_owned();
    for report in reports {
        let _ = writeln!(
            table,
            "| {} | {} | {} |",
            report.language, report.lines.hit, report.lines.found
        );
    }
    table
}
