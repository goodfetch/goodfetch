//! `xtask rss`: the resident-memory check (ADR 0071, as amended by ADRs 0094
//! and 0134). Each scenario runs on each Product's executable it runs on, and
//! the OS's peak resident memory of that process is the scenario's figure.
//! `measure` appends the peaks to a peaks file; `check` holds each one to the
//! committed ceiling for its Product, platform and scenario, in the Release
//! suite on every OS; `propose` turns 20 runs into ceilings for the owner to
//! approve. Until the ceilings file exists, the check reports and gates
//! nothing. Each figure is also shown against its Performance target (ADR
//! 0134), which never gates.

mod ceilings;
mod measure;
pub(crate) mod peak;
pub(crate) mod peaks;
pub(crate) mod platform;
mod report;
pub(crate) mod scenario;

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::Result;
use clap::{Args, Subcommand};

use crate::check::{self, Finding};
use crate::environment::Environment;
use crate::output;

use scenario::Scenario;

/// The ceilings file, which the owner approves (ADR 0071).
const CEILINGS_FILE: &str = "ci/rss-ceilings.toml";

#[derive(Subcommand)]
pub(crate) enum Command {
    /// Runs each scenario on each Product's executable it runs on, as the
    /// test tool manifest names them, and appends each peak to the peaks
    /// file.
    Measure(MeasureArguments),
    /// Holds each peak of the peaks file to its ceiling, writes the JUnit
    /// file and the figures the results store keeps, and summarizes each
    /// peak beside its ceiling and its Performance target. With no ceilings
    /// file it reports and gates nothing.
    Check(CheckArguments),
    /// Proposes the ceilings from at least 20 runs of each scenario on each
    /// executable: 1.5 times the highest peak, rounded up to a whole MiB, as
    /// the ceilings file writes them.
    Propose(ProposeArguments),
    /// Runs one command as this process's only child and prints its peak
    /// resident memory as `peak-bytes=<n>`; with a scenario, drives the
    /// child as the scenario says. `measure` runs each scenario through it.
    #[command(hide = true)]
    Probe(ProbeArguments),
}

#[derive(Args)]
pub(crate) struct MeasureArguments {
    /// The scenarios to run; every scenario when none is named.
    #[arg(value_enum)]
    scenarios: Vec<Scenario>,
    /// The test tool manifest that names each Product's executable.
    #[arg(long)]
    tools: PathBuf,
    /// The peaks file each peak is appended to.
    #[arg(long)]
    out: PathBuf,
    /// How many times each scenario runs on each executable.
    #[arg(long, default_value_t = 1)]
    runs: u32,
}

#[derive(Args)]
pub(crate) struct CheckArguments {
    /// The peaks file.
    #[arg(long)]
    peaks: PathBuf,
    /// The ceilings file.
    #[arg(long, default_value = CEILINGS_FILE)]
    ceilings: PathBuf,
    /// The JUnit file written: one test case per Product and scenario, its
    /// peak, ceiling and target as properties.
    #[arg(long, default_value = "target/junit/rss.xml")]
    junit: PathBuf,
    /// The figures the drift check reads from the results store.
    #[arg(long, default_value = "target/junit/rss/perf-figures.json")]
    figures: PathBuf,
    /// A file the summary is also appended to, such as
    /// `GITHUB_STEP_SUMMARY`.
    #[arg(long)]
    summary: Option<PathBuf>,
}

#[derive(Args)]
pub(crate) struct ProposeArguments {
    /// The peaks file of the runs.
    #[arg(long)]
    peaks: PathBuf,
}

#[derive(Args)]
pub(crate) struct ProbeArguments {
    /// The scenario that drives the child; without one, the child runs to
    /// its end with no input.
    #[arg(long, value_enum, requires = "needs")]
    scenario: Option<Scenario>,
    /// The Product the scenario's Client needs: the executable's own.
    #[arg(long, value_parser = peaks::parse_product)]
    needs: Option<goodfetch_protocol::catalog::Product>,
    /// The program and its arguments.
    #[arg(last = true, required = true)]
    command: Vec<std::ffi::OsString>,
}

pub(crate) fn run(command: Command, environment: &Environment) -> Result<ExitCode> {
    match command {
        Command::Measure(arguments) => {
            let scenarios = if arguments.scenarios.is_empty() {
                Scenario::ALL.to_vec()
            } else {
                arguments.scenarios
            };
            measure::measure(
                &scenarios,
                &environment.path(&arguments.tools),
                &environment.path(&arguments.out),
                arguments.runs,
            )?;
            Ok(ExitCode::SUCCESS)
        }
        Command::Check(arguments) => run_check(&arguments, environment),
        Command::Propose(arguments) => {
            let peaks = peaks::read(&environment.path(&arguments.peaks))?;
            let (proposal, findings) = ceilings::propose(&peaks);
            if findings.is_empty() {
                output::line(&proposal);
            }
            Ok(check::report(&findings))
        }
        Command::Probe(arguments) => {
            let bytes =
                measure::probe(arguments.scenario.zip(arguments.needs), &arguments.command)?;
            output::line(&format!("peak-bytes={bytes}"));
            Ok(ExitCode::SUCCESS)
        }
    }
}

fn run_check(arguments: &CheckArguments, environment: &Environment) -> Result<ExitCode> {
    let peaks = peaks::read(&environment.path(&arguments.peaks))?;
    let ceilings = ceilings::read(&environment.path(&arguments.ceilings))?;
    let rows = report::rows(&peaks, ceilings.as_ref());
    report::write_junit(&environment.path(&arguments.junit), &rows)?;
    report::write_figures(&environment.path(&arguments.figures), &rows)?;
    let summary = report::summary(&rows, ceilings.is_some(), &arguments.ceilings);
    output::line(&summary);
    if let Some(path) = &arguments.summary {
        output::append(&environment.path(path), &summary)?;
    }
    let findings: Vec<Finding> = rows
        .iter()
        .filter_map(|row| {
            let ceiling = row.ceiling?;
            (row.peak > ceiling).then(|| {
                Finding::new(
                    "rss-ceiling",
                    row.subject(),
                    format!(
                        "peak of {} over its ceiling of {} (ADR 0071)",
                        report::mebibytes(row.peak),
                        report::mebibytes(ceiling)
                    ),
                )
            })
        })
        .collect();
    Ok(check::report(&findings))
}
