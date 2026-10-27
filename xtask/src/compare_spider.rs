//! `xtask compare-spider` (ADR 0134): a one-off comparison of Foxhound's
//! crawl with Spider's, the Rust HTTP crawler nearest it, on the owner's
//! machine. `run` crawls the Proving ground's 1,000-page site with each
//! tool in the declared number of trials, interleaved in random order, each
//! tool in a fresh process at the same concurrency, then `report`s: each
//! tool's median pages per second and peak resident memory, labelled
//! `local-controlled` with the machine and never published (ADR 0076),
//! once every trial fetched the same pages and was served the same bytes,
//! and a failure naming the trial that did other work otherwise. It is no
//! Live check, gates nothing and declares no Feature; M4's competitor
//! adapters may absorb it.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs and process bans target async crates (STYLE.md)"
)]

mod report;
mod run;
mod same_work;
mod trial;
mod trials;

use std::path::{Path, PathBuf};
use std::process::{Command as Process, ExitCode};

use anyhow::{Context as _, Result};
use clap::{Args, Subcommand};
use serde::Deserialize;

use crate::check;
use crate::environment::Environment;
use crate::output;

use trials::{Comparison, Tool};

#[derive(Subcommand)]
pub(crate) enum Command {
    /// Runs every declared trial of both tools, writes the trials file and
    /// reports them.
    Run(RunArguments),
    /// Checks that every trial of the trials file did the same work, then
    /// writes the report beside it: `report.json` and `report.md`.
    Report(ReportArguments),
    /// Runs one tool through one crawl as this process's only child and
    /// prints `seconds=<s> peak-bytes=<n>`; `run` runs each trial through
    /// it.
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
    #[arg(long, default_value = "target/compare-spider/trials.json")]
    out: PathBuf,
    /// Gives Spider the site of this many pages instead, which the
    /// same-work check refuses: the check shown failing.
    #[arg(long, hide = true)]
    spider_pages: Option<u64>,
}

#[derive(Args)]
pub(crate) struct ReportArguments {
    /// The trials file.
    #[arg(long, default_value = "target/compare-spider/trials.json")]
    trials: PathBuf,
}

#[derive(Args)]
pub(crate) struct TrialArguments {
    #[arg(long, value_enum)]
    tool: Tool,
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
                arguments.spider_pages,
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
            let (seconds, peak) = trial::run(
                arguments.tool,
                &arguments.executable,
                &crate::rss::crawl::Site {
                    address: &arguments.address,
                    pages: arguments.pages,
                    concurrency: arguments.concurrency,
                },
            )?;
            output::line(&format!("seconds={seconds} peak-bytes={peak}"));
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
