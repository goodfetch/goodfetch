//! `xtask compare` (ADRs 0134 and 0138): the comparison matrix, Foxhound
//! against the crawlers nearest it, on one machine with nothing else
//! running. Each scenario sets every tool to the closest equivalent
//! configuration and the report writes that mapping; a tool that cannot do
//! a scenario has a cell saying so, and a Foxhound value that does not
//! exist yet is marked pending (the owner's comments of 2026-10-30 on
//! #1577). `run` crawls the Proving ground's 1,000-page site with each
//! measured cell's tool in the declared number of trials, interleaved in
//! random order, each tool in a fresh process at the same concurrency,
//! measuring every metric of the owner's list from outside the tools; then
//! counts each cell's system calls under strace, crawls the 100,000-page
//! site for the growth of resident memory when asked, and counts scenario
//! 1's instructions per page under Callgrind, which gives
//! `foxhound.spider_instruction_ratio` (ADR 0140). `report` holds every
//! trial to the same-work check, then writes each cell's median and spread
//! of every metric, labelled `local-controlled` with the machine and never
//! published (ADR 0076); otherwise it fails naming each trial that did
//! other work. It is no Live check, gates nothing and declares no Feature.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs and process bans target async crates (STYLE.md)"
)]

mod foxhound;
mod matrix;
mod observed;
mod report;
mod run;
mod same_work;
mod sampler;
mod trial;
mod trials;

use std::path::{Path, PathBuf};
use std::process::{Command as Process, ExitCode};

use anyhow::{Context as _, Result};
use clap::{Args, Subcommand, ValueEnum};
use serde::Deserialize;

use crate::check;
use crate::environment::Environment;
use crate::output;

use matrix::Scenario;
use trials::Comparison;

#[derive(Subcommand)]
pub(crate) enum Command {
    /// Runs every declared trial of every measured cell, writes the trials
    /// file and reports them.
    Run(RunArguments),
    /// Checks that every trial of the trials file did the same work, then
    /// writes the report beside it: `report.json` and `report.md`.
    Report(ReportArguments),
    /// Runs one cell's tool through one crawl as this process's only child
    /// and prints its figures as one JSON line; `run` runs each trial
    /// through it.
    #[command(hide = true)]
    Trial(TrialArguments),
}

#[derive(Args)]
pub(crate) struct RunArguments {
    /// The test tool manifest naming the Proving ground's and Foxhound's
    /// executables.
    #[arg(long)]
    tools: PathBuf,
    /// Spider's side, `xtask/compare-spider`'s executable.
    #[arg(long)]
    spider: PathBuf,
    /// The trials file written; the report goes beside it.
    #[arg(long, default_value = "target/compare/trials.json")]
    out: PathBuf,
    /// Also runs the growth cell: each measured cell crawls the
    /// 100,000-page site, hours at `run_state.durability: full`.
    #[arg(long)]
    growth: bool,
    /// Trials of each measured cell, for a quicker try; a report of fewer
    /// than 10 says so.
    #[arg(long, default_value_t = trials::TRIALS, hide = true)]
    trials: u32,
    /// Gives Spider the site of this many pages instead, which the
    /// same-work check refuses: the check shown failing.
    #[arg(long, hide = true)]
    spider_pages: Option<u64>,
}

#[derive(Args)]
pub(crate) struct ReportArguments {
    /// The trials file.
    #[arg(long, default_value = "target/compare/trials.json")]
    trials: PathBuf,
}

/// How a trial runs its tool.
#[derive(Clone, Copy, ValueEnum)]
enum Mode {
    Timed,
    Syscalls,
    Callgrind,
}

#[derive(Args)]
pub(crate) struct TrialArguments {
    #[arg(long, value_enum)]
    scenario: Scenario,
    /// The cell's name in its scenario.
    #[arg(long)]
    cell: String,
    /// The tool's executable.
    #[arg(long)]
    executable: PathBuf,
    /// The Proving ground's address.
    #[arg(long)]
    address: String,
    /// The pages of the site crawled.
    #[arg(long)]
    pages: u64,
    #[arg(long)]
    concurrency: u32,
    #[arg(long, value_enum)]
    mode: Mode,
    /// Where a counted trial's tool writes its counts.
    #[arg(long)]
    output: Option<PathBuf>,
}

/// The entries of the test tool manifest a run takes.
#[derive(Deserialize)]
struct Tools {
    proving_ground: PathBuf,
    foxhound: PathBuf,
}

pub(crate) fn run(command: Command, environment: &Environment) -> Result<ExitCode> {
    match command {
        Command::Run(arguments) => {
            let path = environment.path(&arguments.tools);
            let text =
                std::fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
            let tools: Tools = serde_json::from_slice(&text)
                .with_context(|| format!("parsing {}", path.display()))?;
            let commit = Process::new("git")
                .args(["rev-parse", "HEAD"])
                .current_dir(&environment.root)
                .output()
                .context("running git rev-parse")?;
            let trials = run::run(
                &run::Executables {
                    proving_ground: &tools.proving_ground,
                    foxhound: &tools.foxhound,
                    spider: &environment.path(&arguments.spider),
                },
                String::from_utf8_lossy(&commit.stdout).trim().to_owned(),
                &run::Options {
                    trials: arguments.trials,
                    growth: arguments.growth,
                    spider_pages: arguments.spider_pages,
                },
            )?;
            let out = environment.path(&arguments.out);
            if let Some(folder) = out.parent() {
                std::fs::create_dir_all(folder)
                    .with_context(|| format!("creating {}", folder.display()))?;
            }
            std::fs::write(&out, serde_json::to_vec_pretty(&trials)?)
                .with_context(|| format!("writing {}", out.display()))?;
            report(&trials, &out)
        }
        Command::Report(arguments) => {
            let path = environment.path(&arguments.trials);
            let text =
                std::fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
            let trials: Comparison = serde_json::from_slice(&text)
                .with_context(|| format!("parsing {}", path.display()))?;
            report(&trials, &path)
        }
        Command::Trial(arguments) => {
            let (cell, config) = trial::cell_of(arguments.scenario, &arguments.cell)?;
            let output_file = || {
                arguments
                    .output
                    .clone()
                    .context("a counted trial takes --output")
            };
            let mode = match arguments.mode {
                Mode::Timed => trial::Mode::Timed,
                Mode::Syscalls => trial::Mode::Syscalls(output_file()?),
                Mode::Callgrind => trial::Mode::Callgrind(output_file()?),
            };
            let plan = trial::Plan {
                cell,
                config,
                executable: &arguments.executable,
                site: crate::rss::crawl::Site {
                    address: &arguments.address,
                    pages: arguments.pages,
                    concurrency: arguments.concurrency,
                },
                mode,
                goodfetch_variables: &environment.goodfetch_variables,
            };
            let line = match plan.mode {
                trial::Mode::Timed => serde_json::to_string(&plan.timed()?)?,
                trial::Mode::Syscalls(_) | trial::Mode::Callgrind(_) => {
                    serde_json::to_string(&plan.counted()?)?
                }
            };
            output::line(&line);
            Ok(ExitCode::SUCCESS)
        }
    }
}

/// Holds the trials to the same-work check, then writes the report beside
/// the trials file at `trials_path` and prints it; no figure is written
/// when the check fails.
fn report(trials: &Comparison, trials_path: &Path) -> Result<ExitCode> {
    let findings = same_work::check(trials);
    if !findings.is_empty() {
        return Ok(check::report(&findings));
    }
    let report = report::of(trials);
    let markdown = report::markdown(&report, trials);
    let folder = trials_path.parent().unwrap_or_else(|| Path::new("."));
    let json = folder.join("report.json");
    std::fs::write(&json, serde_json::to_vec_pretty(&report)?)
        .with_context(|| format!("writing {}", json.display()))?;
    let md = folder.join("report.md");
    std::fs::write(&md, &markdown).with_context(|| format!("writing {}", md.display()))?;
    output::line(&markdown);
    output::line(&format!(
        "Written to {} and {}",
        md.display(),
        json.display()
    ));
    Ok(ExitCode::SUCCESS)
}
