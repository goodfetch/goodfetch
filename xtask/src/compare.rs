//! `xtask compare` (ADRs 0134 and 0138): the comparison matrix, Foxhound
//! against the crawlers nearest it (Spider, Crawlee, Scrapy, crawlee-rs,
//! pagesift, OpenSpider and fastCRW), on one machine with nothing else
//! running. Each scenario sets every tool to the closest equivalent
//! configuration and the report writes that mapping; a tool that cannot do
//! a scenario has a cell saying so, and a Foxhound value that does not
//! exist yet is marked pending (the owner's comments of 2026-10-30 on
//! #1577). Spider's main rows read its pages at the smallest channel
//! capacity that loses none, and labelled sensitivity rows beside them run
//! it one setting apart (#2213). `run` crawls each scenario's Proving ground
//! site, of 1,000 pages (`scale.test`'s, the article site for the extraction
//! scenario, where each tool's pages are also held to the site's right
//! answers as its trial ends (#2136), the script-rendered site for the
//! headless browser, the realistic-page site for its scenario) or, for the
//! polite scenario, the multi-domain site's 10,000, 10 on each of its 1,000
//! domains (#2283), or, for the long one, which runs only its timed trials,
//! a 100,000-page site, with each measured cell's tool in the declared
//! number of trials, interleaved in random order, each tool in a fresh
//! process at the same concurrency (64 and 256 in the concurrency sweep's
//! scenarios, which with the realistic-page and long scenarios run scenario
//! 1's settings and carry no bar, #2316) and, in a run of the full trials,
//! once the machine is quiet, measuring every metric of the owner's list
//! from outside the tools, each executable dropped from the page cache
//! before its trial; then crawls scenario 1's main cells of Foxhound and
//! Spider again at 100, 1,000 and 10,000 pages, each at 1, 16 and 64 in
//! flight (Spider not at 1, where its crawl stops short), for their peak
//! memory (spec #2165); then
//! counts each cell's system calls under strace, stopping a run whose crawl
//! stalls and leaving its cell not measured, crawls the 100,000-page
//! site for the growth of resident memory when asked, and counts scenario
//! 1's and the extraction scenario's instructions per page under
//! Callgrind, which gives `foxhound.spider_instruction_ratio` and the
//! extraction scenario's two ratios (ADR 0140) on each tool's main rows.
//! The hostile pages' passes (#2439) instead crawl the Proving ground's
//! `traps` and `limits` tasks a few times and count the rules each tool
//! broke there, from the Proving ground's findings, with no bar and no
//! standing. It also runs the Collie scenario (issue 2282, `collie`):
//! Collie against Playwright on one browser, which gives
//! `collie.playwright_command_ratio`, Puppeteer and Stagehand beside them
//! as its arms (issues 2443 and 2441). `report` holds every
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

mod collie;
mod extracted;
mod foxhound;
mod hostile;
mod matrix;
mod observed;
mod relay;
mod report;
mod rerun;
mod run;
mod same_work;
mod sampler;
pub(crate) mod training;
mod trial;
mod trials;
mod trust;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context as _, Result};
use clap::{Args, Subcommand, ValueEnum};

use crate::check;
use crate::environment::Environment;
use crate::output;
use crate::test_tools::{Tool, ToolManifest};

use matrix::Scenario;
use trials::{CollieScenario, Comparison};

/// Crawlee's side's script, as the recipe runs it.
const CRAWLEE: &str = "xtask/compare-crawlee/crawl.mjs";
/// Scrapy's side's script, as the recipe runs it.
const SCRAPY: &str = "xtask/compare-scrapy/crawl.py";
/// The Collie scenario's side's script, as the recipe runs it.
const COLLIE_DRIVER: &str = "xtask/compare-collie/act.ts";
/// The Collie scenario's arms' script, as the recipe runs it.
const COLLIE_ARMS_DRIVER: &str = "xtask/compare-collie-arms/act.ts";
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
    /// `spider-bloom` and so on), `xtask/compare-crawlee-rs` built into
    /// `crawlee-rs`, `xtask/compare-pagesift` into `pagesift`,
    /// `xtask/compare-openspider` into `openspider`,
    /// `xtask/compare-fastcrw` into `fastcrw` (with fastCRW's server
    /// installed into `fastcrw/<profile>/bin`) and the Collie scenario's
    /// `xtask/compare-chromiumoxide` into `chromiumoxide`.
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
    /// The Collie scenario's side, `xtask/compare-collie`'s script, run by
    /// Node.js once its packages are installed.
    #[arg(long, default_value = COLLIE_DRIVER)]
    collie_driver: PathBuf,
    /// The Collie scenario's arms, `xtask/compare-collie-arms`'s script,
    /// run by Node.js once its packages are installed.
    #[arg(long, default_value = COLLIE_ARMS_DRIVER)]
    collie_arms_driver: PathBuf,
    /// Runs only this scenario's cells; repeatable. Every scenario by
    /// default, the Collie scenario's included.
    #[arg(long = "scenario", value_enum)]
    scenarios: Vec<Asked>,
    /// The machine's name, which the report names it by, such as "Hetzner
    /// Cloud CCX43, 16 dedicated vCPUs": a reader knows a machine by its
    /// product or model, not by its CPU's.
    #[arg(long)]
    machine: Option<String>,
    /// A line of Foxhound's user configuration's `[settings]`,
    /// `<key> = <TOML value>`, written in every Foxhound trial's home, such
    /// as `governor.worker_threads = "per_core"`; repeatable. None by
    /// default, as shipped; the report's rerun command gives each.
    #[arg(long = "foxhound-setting")]
    foxhound_settings: Vec<String>,
}

/// A scenario a run can be asked for: one of the crawl matrix's, by its
/// own name, or the Collie scenario (issue 2282), Collie against Playwright
/// on one browser.
#[derive(Clone, Copy, PartialEq, Eq, ValueEnum)]
enum Asked {
    Raw,
    Durable,
    Polite,
    Extraction,
    Browser,
    Realistic,
    Long,
    #[value(name = "raw-64")]
    Raw64,
    #[value(name = "raw-256")]
    Raw256,
    Hostile,
    #[value(name = "hostile-16")]
    Hostile16,
    #[value(name = "crawl-like")]
    CrawlLike,
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
            Self::Realistic => Some(Scenario::Realistic),
            Self::Long => Some(Scenario::Long),
            Self::Raw64 => Some(Scenario::Raw64),
            Self::Raw256 => Some(Scenario::Raw256),
            Self::Hostile => Some(Scenario::Hostile),
            Self::Hostile16 => Some(Scenario::Hostile16),
            Self::CrawlLike => Some(Scenario::CrawlLike),
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
    /// Reads the tool's executable's own pages in a timed trial's samples.
    #[arg(long)]
    executable_pages: bool,
    /// A Spider cell's page channel capacity, which `run` gives the trial:
    /// its row's, or the sizes cell's at 64 in flight.
    #[arg(long)]
    capacity: Option<usize>,
    /// A line of Foxhound's user configuration's `[settings]`, which
    /// `run` gives the trial; repeatable.
    #[arg(long = "foxhound-setting")]
    foxhound_settings: Vec<String>,
}

/// `path` made absolute, for a child that runs in another folder.
fn absolute(path: &Path) -> Result<PathBuf> {
    std::fs::canonicalize(path).with_context(|| format!("finding {}", path.display()))
}

/// `run`: every declared trial, the trials file written as the run goes,
/// then the report.
fn measure(arguments: &RunArguments, environment: &Environment) -> Result<ExitCode> {
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
    let proving_ground = tools.path(Tool::ProvingGround)?;
    let collie = Asked::collie(&arguments.scenarios)
        .then(|| -> Result<_> {
            Ok(collie::Tools {
                proving_ground,
                engine: tools.path(Tool::Collie)?,
                driver: absolute(&environment.path(&arguments.collie_driver))?,
                arms_driver: absolute(&environment.path(&arguments.collie_arms_driver))?,
                chromiumoxide: absolute(&run::chromiumoxide_executable(
                    &environment.path(&arguments.spider_builds),
                    &arguments.spider_profile,
                ))?,
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
            foxhound_settings: &arguments.foxhound_settings,
        },
        String::from_utf8_lossy(&commit.stdout).trim().to_owned(),
        &run::Options {
            machine: arguments.machine.clone(),
            command: rerun::of(arguments),
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

pub(crate) fn run(command: Command, environment: &Environment) -> Result<ExitCode> {
    match command {
        Command::Run(arguments) => measure(&arguments, environment),
        Command::Report(arguments) => {
            let path = environment.path(&arguments.trials);
            let text =
                std::fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
            let mut trials: Comparison = serde_json::from_slice(&text)
                .with_context(|| format!("parsing {}", path.display()))?;
            report(&mut trials, &path, environment)
        }
        Command::Trial(arguments) => {
            let (cell, mut config) = trial::cell_of(arguments.scenario, &arguments.cell)?;
            if let (matrix::Config::Spider { capacity, .. }, Some(given)) =
                (&mut config, arguments.capacity)
            {
                *capacity = given;
            }
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
            let authority = cell
                .scenario
                .network()
                .tls()
                .then(|| trust::certificate_authority(&arguments.address))
                .transpose()?;
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
                executable_pages: arguments.executable_pages,
                certificate_authority: authority.as_ref().map(tempfile::NamedTempFile::path),
                foxhound_settings: &arguments.foxhound_settings,
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
    let register = environment.root.join(crate::register::document::FILE);
    let testing = match &trials.collie {
        CollieScenario::NotRun { .. } => String::new(),
        CollieScenario::Measured { .. } => std::fs::read_to_string(&register)
            .with_context(|| format!("reading the Quality register in {}", register.display()))?,
    };
    let section = collie::section(&trials.collie, &testing);
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

#[cfg(test)]
mod tests;
