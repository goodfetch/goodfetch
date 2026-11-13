//! `xtask compare` (ADRs 0134 and 0138): the comparison matrix, Foxhound
//! against the crawlers nearest it (Spider, Crawlee, Scrapy and crawlee-rs), on one machine with nothing else
//! running. Each scenario sets every tool to the closest equivalent
//! configuration and the report writes that mapping; a tool that cannot do
//! a scenario has a cell saying so, and a Foxhound value that does not
//! exist yet is marked pending (the owner's comments of 2026-10-30 on
//! #1577). Spider's main rows read its pages at the smallest channel
//! capacity that loses none, and labelled sensitivity rows beside them run
//! it one setting apart (#2213). `run` crawls each scenario's 1,000-page Proving ground site
//! (`scale.test`'s, the multi-domain site for the polite scenario, the
//! article site for the extraction scenario, where each tool's pages are
//! also held to the site's right answers as its trial ends (#2136), the
//! script-rendered site for the headless browser, the realistic-page site
//! for its scenario, and a 100,000-page site for the long one, which runs
//! only its timed trials) with each measured cell's tool in the declared
//! number of trials, interleaved in random order, each tool in a fresh
//! process at the same concurrency (64 and 256 in the concurrency sweep's
//! scenarios, which with the realistic-page and long scenarios run scenario
//! 1's settings and carry no bar, #2316) and, in a run of the full trials,
//! once the machine is quiet, measuring every metric of the owner's list
//! from outside the tools; then
//! counts each cell's system calls under strace, stopping a run whose crawl
//! stalls and leaving its cell not measured, crawls the 100,000-page
//! site for the growth of resident memory when asked, and counts scenario
//! 1's and the extraction scenario's instructions per page under
//! Callgrind, which gives `foxhound.spider_instruction_ratio` and the
//! extraction scenario's two ratios (ADR 0140) on each tool's main rows.
//! The hostile pages' passes (#2439) instead crawl the Proving ground's
//! `traps` and `limits` tasks a few times and count the rules each tool
//! broke there, from the Proving ground's findings, with no bar and no
//! standing. `report` holds every
//! trial to the same-work check, then writes each cell's median and spread
//! of every metric and Foxhound's verdicts against each competitor,
//! labelled `local-controlled`, measured on the machine it names, with the
//! command that reruns it; otherwise it fails naming each trial that did
//! other work, and each of Foxhound's whose pages differ from the right
//! answer. It is no Live check, gates nothing and declares no Feature.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs and process bans target async crates (STYLE.md)"
)]

mod extracted;
mod foxhound;
mod hostile;
mod matrix;
mod observed;
mod report;
mod rerun;
mod run;
mod same_work;
mod sampler;
mod trial;
mod trials;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context as _, Result};
use clap::{Args, Subcommand, ValueEnum};

use crate::check;
use crate::environment::Environment;
use crate::output;
use crate::test_tools::{Tool, ToolManifest};

use matrix::Scenario;
use trials::Comparison;

/// Crawlee's side's script, as the recipe runs it.
const CRAWLEE: &str = "xtask/compare-crawlee/crawl.mjs";
/// Scrapy's side's script, as the recipe runs it.
const SCRAPY: &str = "xtask/compare-scrapy/crawl.py";
/// The trials file the recipe writes.
const TRIALS_FILE: &str = "target/compare/trials.json";
/// The folder beside the trials file where a run keeps the Callgrind cell's
/// outputs.
const CALLGRIND_OUTPUTS: &str = "callgrind";

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
    /// The folder of the Rust sides' builds: `xtask/compare-spider` built
    /// once for each build the matrix's Spider rows run, each into the
    /// target folder of its name under this one (`spider`, `spider-chrome`,
    /// `spider-bloom` and so on), and `xtask/compare-crawlee-rs` built into
    /// `crawlee-rs`.
    #[arg(long)]
    spider_builds: PathBuf,
    /// The folder Cargo names after the profile those builds were made
    /// with: `release` for the release profile.
    #[arg(long, default_value = "release")]
    spider_profile: String,
    /// Chrome for Testing's folder, which every tool of the headless
    /// browser scenario runs: by default, the build Foxhound pins as
    /// Playwright installed it (`PLAYWRIGHT_BROWSERS_PATH`).
    #[arg(long)]
    browser: Option<PathBuf>,
    /// Crawlee's side, `xtask/compare-crawlee`'s script, run by Node.js
    /// once its packages are installed beside it.
    #[arg(long, default_value = CRAWLEE)]
    crawlee: PathBuf,
    /// Scrapy's side, `xtask/compare-scrapy`'s script, run by the Python
    /// environment `uv sync` makes beside it.
    #[arg(long, default_value = SCRAPY)]
    scrapy: PathBuf,
    /// The trials file written; the report goes beside it, and the
    /// Callgrind cell's outputs into `callgrind/` beside it.
    #[arg(long, default_value = TRIALS_FILE)]
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
    /// Runs only this scenario's cells; repeatable. Every scenario by
    /// default.
    #[arg(long = "scenario", value_enum)]
    scenarios: Vec<Scenario>,
    /// The machine's name, which the report names it by, such as "Hetzner
    /// Cloud CCX43, 16 dedicated vCPUs": a reader knows a machine by its
    /// product or model, not by its CPU's.
    #[arg(long)]
    machine: Option<String>,
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
    /// A hostile pages' cell's run (#2439): the rules its tool broke.
    Rules,
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
    /// Chrome for Testing's folder, for the headless browser scenario.
    #[arg(long)]
    browser: Option<PathBuf>,
    /// Where a counted trial's tool writes its counts.
    #[arg(long)]
    output: Option<PathBuf>,
}

pub(crate) fn run(command: Command, environment: &Environment) -> Result<ExitCode> {
    match command {
        Command::Run(arguments) => {
            let tools = ToolManifest::read(&environment.path(&arguments.tools))?;
            let commit = crate::git::command(&environment.root)
                .args(["rev-parse", "HEAD"])
                .output()
                .context("running git rev-parse")?;
            let browser = match &arguments.browser {
                Some(browser) => environment.path(browser),
                None => trial::chrome_folder().with_context(|| {
                    format!(
                        "Playwright's Chrome for Testing {} is not installed: run `pnpm exec playwright install chromium`, or name its folder with --browser",
                        crate::test_facts::CHROME_FOR_TESTING_BUILD
                    )
                })?,
            };
            let out = environment.path(&arguments.out);
            let folder = out.parent().unwrap_or_else(|| Path::new("."));
            let mut trials = run::run(
                &run::Executables {
                    proving_ground: tools.path(Tool::ProvingGround)?,
                    foxhound: tools.path(Tool::Foxhound)?,
                    spider_builds: &environment.path(&arguments.spider_builds),
                    spider_profile: &arguments.spider_profile,
                    browser: &browser,
                    crawlee: &environment.path(&arguments.crawlee),
                    scrapy: &environment.path(&arguments.scrapy),
                },
                String::from_utf8_lossy(&commit.stdout).trim().to_owned(),
                &run::Options {
                    machine: arguments.machine.clone(),
                    command: rerun::of(&arguments),
                    trials: arguments.trials,
                    growth: arguments.growth,
                    spider_pages: arguments.spider_pages,
                    scenarios: if arguments.scenarios.is_empty() {
                        Scenario::ALL.to_vec()
                    } else {
                        arguments.scenarios.clone()
                    },
                    callgrind_outputs: folder.join(CALLGRIND_OUTPUTS),
                },
                &mut |so_far| write(so_far, &out),
            )?;
            write(&trials, &out)?;
            report(&mut trials, &out)
        }
        Command::Report(arguments) => {
            let path = environment.path(&arguments.trials);
            let text =
                std::fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
            let mut trials: Comparison = serde_json::from_slice(&text)
                .with_context(|| format!("parsing {}", path.display()))?;
            report(&mut trials, &path)
        }
        Command::Trial(arguments) => {
            let (cell, config) = trial::cell_of(arguments.scenario, &arguments.cell)?;
            let output_file = || trial::counting_output(arguments.output.as_deref());
            let mode = match arguments.mode {
                Mode::Timed => trial::Mode::Timed,
                Mode::Syscalls => trial::Mode::Syscalls(output_file()?),
                Mode::Callgrind => trial::Mode::Callgrind(output_file()?),
                Mode::Rules => trial::Mode::Rules,
            };
            // Absolute, since a tool other than Foxhound runs in its data
            // folder.
            let executable = std::path::absolute(&arguments.executable)
                .context("finding the tool's executable")?;
            let plan = trial::Plan {
                cell,
                config,
                executable: &executable,
                site: crate::rss::crawl::Site {
                    address: &arguments.address,
                    pages: arguments.pages,
                    concurrency: arguments.concurrency,
                },
                mode,
                goodfetch_variables: &environment.goodfetch_variables,
                browser: arguments.browser.as_deref(),
            };
            let line = match plan.mode {
                trial::Mode::Timed => serde_json::to_string(&plan.timed()?)?,
                trial::Mode::Rules => serde_json::to_string(&plan.ruled()?)?,
                trial::Mode::Syscalls(_) | trial::Mode::Callgrind(_) => {
                    serde_json::to_string(&plan.counted()?)?
                }
            };
            output::line(&line);
            Ok(ExitCode::SUCCESS)
        }
    }
}

/// Writes the trials file at `out`, creating its folder: the whole run's,
/// and before that what the run has measured so far, which `report` reads
/// as a run that did not reach its later cells.
fn write(trials: &Comparison, out: &Path) -> Result<()> {
    let folder = out.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(folder).with_context(|| format!("creating {}", folder.display()))?;
    std::fs::write(out, serde_json::to_vec_pretty(trials)?)
        .with_context(|| format!("writing {}", out.display()))
}

/// Holds the trials to the same-work check, then writes the report beside
/// the trials file at `trials_path` and prints it; no figure is written
/// when the check fails. A strace run that did other work fails nothing:
/// its cell's system calls are reported as not measured, and the trials
/// file keeps the run as it was.
fn report(trials: &mut Comparison, trials_path: &Path) -> Result<ExitCode> {
    same_work::strace_runs_of_other_work_set_aside(trials);
    let trials = &*trials;
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
