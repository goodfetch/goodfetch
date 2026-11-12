//! `xtask compare` (ADRs 0134 and 0138): the comparison matrix, Foxhound
//! against the crawlers nearest it (Spider, Crawlee and Scrapy), on one machine with nothing else
//! running. Each scenario sets every tool to the closest equivalent
//! configuration and the report writes that mapping; a tool that cannot do
//! a scenario has a cell saying so, and a Foxhound value that does not
//! exist yet is marked pending (the owner's comments of 2026-10-30 on
//! #1577). Spider's main rows read its pages at the smallest channel
//! capacity that loses none, and labelled sensitivity rows beside them run
//! it one setting apart (#2213). `run` crawls each scenario's 1,000-page Proving ground site
//! (`scale.test`'s, the multi-domain site for the polite scenario, the
//! script-rendered site for the headless browser) with each measured cell's
//! tool in the declared number of trials, interleaved in random order, each
//! tool in a fresh process at the same concurrency and, in a run of the
//! full trials, once the machine is quiet, measuring every metric of the
//! owner's list from outside the tools; then
//! counts each cell's system calls under strace, stopping a run whose crawl
//! stalls and leaving its cell not measured, crawls the 100,000-page
//! site for the growth of resident memory when asked, and counts scenario
//! 1's instructions per page under Callgrind, which gives
//! `foxhound.spider_instruction_ratio` (ADR 0140) on each tool's main row.
//! It also runs the Collie scenario (issue 2282, `collie`): Collie
//! against Playwright on one browser, which gives
//! `collie.playwright_command_ratio`. `report` holds every
//! trial to the same-work check, then writes each cell's median and spread
//! of every metric, labelled `local-controlled` with the machine and never
//! published (ADR 0076); otherwise it fails naming each trial that did
//! other work. It is no Live check, gates nothing and declares no Feature.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs and process bans target async crates (STYLE.md)"
)]

mod collie;
mod foxhound;
mod matrix;
mod observed;
mod relay;
mod report;
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
    /// The folder of Spider's side's builds: `xtask/compare-spider` built
    /// once for each build the matrix's Spider rows run, each into the
    /// target folder of its name under this one (`spider`, `spider-chrome`,
    /// `spider-bloom` and so on).
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
    #[arg(long, default_value = "xtask/compare-crawlee/crawl.mjs")]
    crawlee: PathBuf,
    /// Scrapy's side, `xtask/compare-scrapy`'s script, run by the Python
    /// environment `uv sync` makes beside it.
    #[arg(long, default_value = "xtask/compare-scrapy/crawl.py")]
    scrapy: PathBuf,
    /// The trials file written; the report goes beside it, and the
    /// Callgrind cell's outputs into `callgrind/` beside it.
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
    /// The Collie scenario's side, `xtask/compare-collie`'s script, run by
    /// Node.js once its packages are installed.
    #[arg(long, default_value = "xtask/compare-collie/act.ts")]
    collie_driver: PathBuf,
    /// Runs only this scenario's cells; repeatable. Every scenario by
    /// default, the Collie scenario's included.
    #[arg(long = "scenario", value_enum)]
    scenarios: Vec<Asked>,
}

/// A scenario a run can be asked for: one of the crawl matrix's, or the
/// Collie scenario (issue 2282), Collie against Playwright on one browser.
#[derive(Clone, Copy, PartialEq, Eq, ValueEnum)]
enum Asked {
    Raw,
    Durable,
    Polite,
    Extraction,
    Browser,
    Collie,
}

impl Asked {
    /// The crawl matrix's scenarios of `asked`: every one when none is.
    fn crawls(asked: &[Self]) -> Vec<Scenario> {
        if asked.is_empty() {
            return Scenario::ALL.to_vec();
        }
        asked.iter().filter_map(|asked| asked.crawl()).collect()
    }

    /// Whether `asked` holds the Collie scenario: it does when empty.
    fn collie(asked: &[Self]) -> bool {
        asked.is_empty() || asked.contains(&Self::Collie)
    }

    /// The crawl matrix's scenario; none for the Collie scenario.
    fn crawl(self) -> Option<Scenario> {
        match self {
            Self::Raw => Some(Scenario::Raw),
            Self::Durable => Some(Scenario::Durable),
            Self::Polite => Some(Scenario::Polite),
            Self::Extraction => Some(Scenario::Extraction),
            Self::Browser => Some(Scenario::Browser),
            Self::Collie => None,
        }
    }
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
            let collie_driver = environment.path(&arguments.collie_driver);
            let proving_ground = tools.path(Tool::ProvingGround)?;
            let collie = Asked::collie(&arguments.scenarios)
                .then(|| -> Result<_> {
                    Ok(collie::Tools {
                        proving_ground,
                        engine: tools.path(Tool::Collie)?,
                        driver: &collie_driver,
                        browser: &browser,
                        goodfetch_variables: &environment.goodfetch_variables,
                    })
                })
                .transpose()?;
            let mut trials = run::run(
                &run::Executables {
                    proving_ground,
                    foxhound: tools.path(Tool::Foxhound)?,
                    spider_builds: &environment.path(&arguments.spider_builds),
                    spider_profile: &arguments.spider_profile,
                    browser: &browser,
                    crawlee: &environment.path(&arguments.crawlee),
                    scrapy: &environment.path(&arguments.scrapy),
                    collie,
                },
                String::from_utf8_lossy(&commit.stdout).trim().to_owned(),
                &run::Options {
                    trials: arguments.trials,
                    growth: arguments.growth,
                    spider_pages: arguments.spider_pages,
                    scenarios: Asked::crawls(&arguments.scenarios),
                    callgrind_outputs: folder.join(CALLGRIND_OUTPUTS),
                },
                &mut |so_far| write(so_far, &out),
            )?;
            write(&trials, &out)?;
            report(&mut trials, &out, environment)
        }
        Command::Report(arguments) => {
            let path = environment.path(&arguments.trials);
            let text =
                std::fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
            let mut trials: Comparison = serde_json::from_slice(&text)
                .with_context(|| format!("parsing {}", path.display()))?;
            report(&mut trials, &path, environment)
        }
        Command::Trial(arguments) => {
            let (cell, config) = trial::cell_of(arguments.scenario, &arguments.cell)?;
            let output_file = || trial::counting_output(arguments.output.as_deref());
            let mode = match arguments.mode {
                Mode::Timed => trial::Mode::Timed,
                Mode::Syscalls => trial::Mode::Syscalls(output_file()?),
                Mode::Callgrind => trial::Mode::Callgrind(output_file()?),
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

/// The report file: the crawl matrix's report, and the Collie scenario's
/// section beside it.
#[derive(serde::Serialize)]
struct Written<'a> {
    #[serde(flatten)]
    report: report::Report<'a>,
    collie: collie::Section,
}

/// Holds the trials to the same-work check, the Collie scenario's
/// included, then writes the report beside the trials file at
/// `trials_path` and prints it; no figure is written when the check fails.
/// A strace run that did other work fails nothing: its cell's system calls
/// are reported as not measured, and the trials file keeps the run as it
/// was. The Collie scenario's ratio is held to the Today of its row in
/// the checkout's TESTING.md.
fn report(
    trials: &mut Comparison,
    trials_path: &Path,
    environment: &Environment,
) -> Result<ExitCode> {
    same_work::strace_runs_of_other_work_set_aside(trials);
    let trials = &*trials;
    let mut findings = same_work::check(trials);
    findings.extend(collie::check(&trials.collie));
    if !findings.is_empty() {
        return Ok(check::report(&findings));
    }
    let testing =
        std::fs::read_to_string(environment.root.join(crate::register::document::FILE)).ok();
    let section = collie::section(&trials.collie, testing.as_deref());
    let report = report::of(trials);
    let mut markdown = report::markdown(&report, trials);
    markdown.push_str(&collie::markdown(&section));
    let written = Written {
        report,
        collie: section,
    };
    let folder = trials_path.parent().unwrap_or_else(|| Path::new("."));
    let json = folder.join("report.json");
    std::fs::write(&json, serde_json::to_vec_pretty(&written)?)
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
