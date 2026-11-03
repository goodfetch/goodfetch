//! `xtask targets` (ADR 0134): the Performance targets report each
//! milestone's owner test links. `startup` times each Product's Attached
//! engine from its start to its answer to `hello` with hyperfine; `just
//! targets` then measures the idle Attached engine's, the idle Daemon's
//! and the crawl's peaks with `xtask rss measure`; `report` reads those and
//! the latest `just compare` report's pages per second and gives each
//! target its figure: met at or under it, missed over it with the gap and
//! the optimization pass's write-up, or not measured. It runs on the
//! owner's machine, is labelled `local-controlled` and never published (ADR
//! 0076), fails on no miss and declares no Feature.

mod measured;
mod report;
mod startup;
mod target;

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::Result;
use clap::{Args, Subcommand};

use crate::environment::Environment;
use crate::output;

/// The optimization pass's write-ups of the targets it missed (#1577).
const GAPS_FILE: &str = "ci/performance-target-gaps.toml";

#[derive(Subcommand)]
pub(crate) enum Command {
    /// Times each Product's Attached engine from its start to its answer to
    /// `hello` with hyperfine, after the declared warm-up runs, and writes
    /// hyperfine's export of each to `startup-<product>.json` in `--out`.
    Startup(StartupArguments),
    /// Writes the report, `report.md` and `report.json`, into the measured
    /// folder: each target, its figure, and met, missed with its gap, or
    /// not measured.
    Report(ReportArguments),
}

#[derive(Args)]
pub(crate) struct StartupArguments {
    /// The test tool manifest that names each Product's executable.
    #[arg(long)]
    tools: PathBuf,
    /// The folder the measurements are written to.
    #[arg(long)]
    out: PathBuf,
}

#[derive(Args)]
pub(crate) struct ReportArguments {
    /// The folder `startup` and `xtask rss measure` wrote to: hyperfine's
    /// `startup-<product>.json` exports and the scenarios' peaks, `peaks.jsonl`.
    #[arg(long)]
    measured: PathBuf,
    /// The latest `just compare` report, its `report.json`.
    #[arg(long, default_value = "target/compare/report.json")]
    spider: PathBuf,
    /// The optimization pass's write-ups, keyed by target and Product.
    #[arg(long, default_value = GAPS_FILE)]
    gaps: PathBuf,
}

pub(crate) fn run(command: Command, environment: &Environment) -> Result<ExitCode> {
    match command {
        Command::Startup(arguments) => {
            startup::measure(
                &environment.path(&arguments.tools),
                &environment.path(&arguments.out),
                &environment.goodfetch_variables,
            )?;
        }
        Command::Report(arguments) => {
            let folder = environment.path(&arguments.measured);
            let measured = measured::read(
                &folder,
                &environment.path(&arguments.spider),
                &environment.path(&arguments.gaps),
                &target::Target::ALL.map(|(target, _)| target.name()),
            )?;
            let rows = target::rows(&measured);
            let markdown = report::write(&folder, &rows, &measured)?;
            output::line(&markdown);
        }
    }
    // A missed target is the owner's to weigh at the owner test, never a
    // failure (ADR 0134).
    Ok(ExitCode::SUCCESS)
}
