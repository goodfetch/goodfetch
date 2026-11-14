//! `run`: every measured cell's declared trials in one random order, each
//! against a fresh Proving ground, then the Collie scenario's (when asked
//! for), the sizes cell's, the system-call cell's, the growth cell's (when
//! asked for) and the Callgrind cell's runs in the same way, written to the
//! trials file. Every executable a crawl trial runs is dropped from the
//! page cache before it starts (spec #2165).

use std::io::{Read as _, Seek as _, SeekFrom};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context as _, Result, bail};

use crate::output;
use crate::rss::crawl::serve;
use crate::rss::page_cache;

use self::machine::{load, machine, quiet};
use self::watch::{Retried, Watch, retried, watched};
use super::matrix::{self, Cell, Config, Row, Scenario, SpiderBuild};
use super::trial::{self, Counted, Outcome, Ruled, callgrind_missing, strace_missing};
use super::trials::{
    CONCURRENCY, CRAWLBERG_VERSION, CRAWLEE_RS_COMMIT, CRAWLEE_VERSION, CollieScenario, Comparison,
    CountedCell, CountedRun, Declared, FASTCRW_VERSION, GROWTH_PAGES, GrowthCell, INSTRUCTION_RUNS,
    INSTRUCTION_SCENARIOS, INSTRUCTION_TOOLS, Measured, OPENSPIDER_COMMIT, PAGESIFT_VERSION,
    PLAYWRIGHT_VERSION, RULE_RUNS, RuledRun, SCRAPY_PLAYWRIGHT_VERSION, SCRAPY_VERSION, SITE_PAGES,
    SIZE_CONCURRENCIES, SIZE_TOOLS, SIZES, SPIDER_VERSION, STATISTIC, SizesCell, StoppedPoint,
    StoppedRun, Trial, size_not_run,
};

mod collie;
mod machine;
mod watch;

/// The executables a run takes.
pub(crate) struct Executables<'a> {
    pub(crate) proving_ground: &'a Path,
    pub(crate) foxhound: &'a Path,
    /// The folder of the Rust sides' builds, Spider's, crawlee-rs's,
    /// pagesift's, OpenSpider's, fastCRW's and crawlberg's, and the folder
    /// Cargo names after the profile each was built with (see
    /// [`spider_executable`], [`crawlee_rs_executable`],
    /// [`pagesift_executable`], [`openspider_executable`],
    /// [`fastcrw_executable`] and [`crawlberg_executable`]).
    pub(crate) spider_builds: &'a Path,
    pub(crate) spider_profile: &'a str,
    /// Crawlee's side's script, run by Node.js, and Scrapy's, run by the
    /// Python environment beside it.
    pub(crate) crawlee: &'a Path,
    pub(crate) scrapy: &'a Path,
    /// Chrome for Testing's folder, which every tool of the headless
    /// browser scenario runs.
    pub(crate) browser: &'a Path,
    /// The Collie scenario's, when the run asked for it.
    pub(crate) collie: Option<super::collie::Tools<'a>>,
}

/// What a run was asked for.
pub(crate) struct Options {
    /// The name the machine is given, which the report names it by.
    pub(crate) machine: Option<String>,
    /// The command that reruns the run, which the report gives.
    pub(crate) command: String,
    /// Trials of each measured cell.
    pub(crate) trials: u32,
    /// Whether the growth cell runs.
    pub(crate) growth: bool,
    /// Gives Spider the site of this many pages instead, which the
    /// same-work check refuses: the check shown failing.
    pub(crate) spider_pages: Option<u64>,
    /// The scenarios whose cells run.
    pub(crate) scenarios: Vec<Scenario>,
    /// The folder the Callgrind cell's outputs are kept in, one file a run
    /// (see [`counted_output`]): a profile by function is read from them,
    /// and the reports repository files them with the run.
    pub(crate) callgrind_outputs: PathBuf,
}

/// Whether each timed trial waits for a quiet machine: only in a run of the
/// matrix's full [`super::trials::TRIALS`], so a quicker try, which its
/// report flags as short, never sleeps.
pub(crate) fn waits_for_quiet(options: &Options) -> bool {
    options.trials >= super::trials::TRIALS
}

/// The measured cells of the scenarios a run asked for.
fn measured(options: &Options) -> Vec<(&'static Cell, Config)> {
    matrix::measured()
        .filter(|(cell, _)| options.scenarios.contains(&cell.scenario))
        .collect()
}

/// The hostile pages' cells of the scenarios a run asked for (#2439).
fn ruled(options: &Options) -> Vec<(&'static Cell, Config)> {
    matrix::ruled()
        .filter(|(cell, _)| options.scenarios.contains(&cell.scenario))
        .collect()
}

/// `SplitMix64`, for the trials' order: nothing here needs more.
fn next(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

/// `runs(item)` runs of each of `items`, each with its number from 1,
/// shuffled by `state` (Fisher–Yates).
fn order<T: Copy>(items: &[T], runs: impl Fn(T) -> u32, state: &mut u64) -> Vec<(T, u32)> {
    let mut order: Vec<_> = items
        .iter()
        .flat_map(|&item| (1..=runs(item)).map(move |number| (item, number)))
        .collect();
    for last in (1..order.len()).rev() {
        let pick = usize::try_from(next(state) % (last as u64 + 1))
            .expect("invariant: an index below the trial count fits a usize");
        order.swap(last, pick);
    }
    order
}

/// The seed of the trials' order, drawn from the wall clock: any seed will
/// do, and the one drawn is recorded with the trials.
#[expect(
    clippy::disallowed_methods,
    reason = "xtask has no Clock or Entropy handle (ADR 0059): a one-off comparison draws its recorded order seed from the wall clock"
)]
fn drawn_seed() -> u64 {
    let now = jiff::Timestamp::now();
    now.as_second().unsigned_abs() ^ u64::from(now.subsec_nanosecond().unsigned_abs()) << 32
}

/// The executable of `build` among Spider's side's builds in `builds`:
/// `just compare` builds each into the target folder
/// [`SpiderBuild::folder`] names, where Cargo puts it under `profile`'s
/// folder.
pub(crate) fn spider_executable(builds: &Path, profile: &str, build: SpiderBuild) -> PathBuf {
    builds.join(build.folder()).join(profile).join(format!(
        "goodfetch-compare-spider{}",
        std::env::consts::EXE_SUFFIX
    ))
}

/// crawlee-rs's side's executable in `builds`, beside Spider's builds:
/// `just compare` builds it into the target folder
/// [`matrix::CRAWLEE_RS_FOLDER`] names, where Cargo puts it under
/// `profile`'s folder.
pub(crate) fn crawlee_rs_executable(builds: &Path, profile: &str) -> PathBuf {
    builds
        .join(matrix::CRAWLEE_RS_FOLDER)
        .join(profile)
        .join(format!(
            "goodfetch-compare-crawlee-rs{}",
            std::env::consts::EXE_SUFFIX
        ))
}

/// pagesift's side's executable in `builds`, beside Spider's builds:
/// `just compare` builds it into the target folder
/// [`matrix::PAGESIFT_FOLDER`] names, where Cargo puts it under `profile`'s
/// folder.
pub(crate) fn pagesift_executable(builds: &Path, profile: &str) -> PathBuf {
    builds
        .join(matrix::PAGESIFT_FOLDER)
        .join(profile)
        .join(format!(
            "goodfetch-compare-pagesift{}",
            std::env::consts::EXE_SUFFIX
        ))
}

/// OpenSpider's side's executable in `builds`, beside Spider's builds:
/// `just compare` builds it into the target folder
/// [`matrix::OPENSPIDER_FOLDER`] names, where Cargo puts it under
/// `profile`'s folder.
pub(crate) fn openspider_executable(builds: &Path, profile: &str) -> PathBuf {
    builds
        .join(matrix::OPENSPIDER_FOLDER)
        .join(profile)
        .join(format!(
            "goodfetch-compare-openspider{}",
            std::env::consts::EXE_SUFFIX
        ))
}

/// fastCRW's side's executable in `builds`, beside Spider's builds:
/// `just compare` builds it into the target folder
/// [`matrix::FASTCRW_FOLDER`] names, where Cargo puts it under `profile`'s
/// folder, and installs fastCRW's server into that profile's folder (see
/// [`trial::fastcrw_server`]).
pub(crate) fn fastcrw_executable(builds: &Path, profile: &str) -> PathBuf {
    builds
        .join(matrix::FASTCRW_FOLDER)
        .join(profile)
        .join(format!(
            "goodfetch-compare-fastcrw{}",
            std::env::consts::EXE_SUFFIX
        ))
}

/// crawlberg's side's executable in `builds`, beside Spider's builds:
/// `just compare` builds it into the target folder
/// [`matrix::CRAWLBERG_FOLDER`] names, where Cargo puts it under
/// `profile`'s folder.
pub(crate) fn crawlberg_executable(builds: &Path, profile: &str) -> PathBuf {
    builds
        .join(matrix::CRAWLBERG_FOLDER)
        .join(profile)
        .join(format!(
            "goodfetch-compare-crawlberg{}",
            std::env::consts::EXE_SUFFIX
        ))
}

/// chromiumoxide's side's executable in `builds`, beside Spider's builds:
/// `just compare` builds it into the target folder
/// [`matrix::CHROMIUMOXIDE_FOLDER`] names, where Cargo puts it under
/// `profile`'s folder (issue 2444).
pub(crate) fn chromiumoxide_executable(builds: &Path, profile: &str) -> PathBuf {
    builds
        .join(matrix::CHROMIUMOXIDE_FOLDER)
        .join(profile)
        .join(format!(
            "goodfetch-compare-chromiumoxide{}",
            std::env::consts::EXE_SUFFIX
        ))
}

/// Where a trial crawls: the site's pages and the requests it has in
/// flight at once, and whether its samples read the tool's executable's
/// own pages.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Crawl {
    pub(crate) pages: u64,
    pub(crate) concurrency: u32,
    pub(crate) executable_pages: bool,
}

impl Crawl {
    /// `scenario`'s crawl of the site of `pages` pages, with the requests
    /// the scenario has in flight: every cell's but the sizes cell's.
    fn of(scenario: Scenario, pages: u64) -> Self {
        Self {
            pages,
            concurrency: scenario.concurrency(),
            executable_pages: false,
        }
    }
}

/// The executables a trial of `config` runs: the tool's, the interpreter
/// that runs it when it is a script, and fastCRW's server, which its side
/// starts.
fn executables_of(executables: &Executables<'_>, config: Config) -> Vec<PathBuf> {
    let executable = match config {
        Config::Foxhound { .. } => executables.foxhound.to_owned(),
        Config::Spider { build, .. } => {
            spider_executable(executables.spider_builds, executables.spider_profile, build)
        }
        Config::Crawlee => executables.crawlee.to_owned(),
        Config::Scrapy => executables.scrapy.to_owned(),
        Config::CrawleeRs => {
            crawlee_rs_executable(executables.spider_builds, executables.spider_profile)
        }
        Config::Pagesift => {
            pagesift_executable(executables.spider_builds, executables.spider_profile)
        }
        Config::OpenSpider => {
            openspider_executable(executables.spider_builds, executables.spider_profile)
        }
        Config::FastCrw => {
            fastcrw_executable(executables.spider_builds, executables.spider_profile)
        }
        Config::Crawlberg => {
            crawlberg_executable(executables.spider_builds, executables.spider_profile)
        }
    };
    let program = trial::invocation(config, &executable).program;
    let server = matches!(config, Config::FastCrw).then(|| trial::fastcrw_server(&executable));
    let mut run = vec![executable];
    if program != run[0] {
        run.push(program);
    }
    run.extend(server);
    run
}

/// One trial of `cell` against a fresh Proving ground, through `xtask
/// compare trial`, which prints its figures as one JSON line, once each
/// executable it runs is dropped from the page cache, so it pages them in
/// by its own faults, as a first start does. `mode` and `output` name a
/// counted trial's tool and file; `watch`, what the comparison stops once
/// [`watch::stopped`] says why.
#[expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous: a trial runs as a blocking child process (STYLE.md)"
)]
fn trial<T: serde::de::DeserializeOwned>(
    executables: &Executables<'_>,
    cell: &Cell,
    config: Config,
    crawl: Crawl,
    mode: (&str, Option<&Path>),
    watch: Watch,
) -> Result<Outcome<T>> {
    let run = executables_of(executables, config);
    let executable = &run[0];
    for path in &run {
        page_cache::drop_cached(path)?;
    }
    let serving = serve(
        executables.proving_ground,
        cell.scenario.network().network_profile(),
    )?;
    let probe = std::env::current_exe().context("finding xtask's own executable")?;
    let mut command = Command::new(probe);
    command
        .args(["compare", "trial", "--scenario", cell.scenario.name()])
        .args(["--cell", cell.name, "--executable"])
        .arg(executable)
        .args(["--address", &serving.address])
        .args(["--pages", &crawl.pages.to_string()])
        .args(["--concurrency", &crawl.concurrency.to_string()])
        .args(["--mode", mode.0])
        .arg("--browser")
        .arg(executables.browser);
    if let Config::Spider { capacity, .. } = config {
        command.args(["--capacity", &capacity.to_string()]);
    }
    if let Some(output) = mode.1 {
        command.arg("--output").arg(output);
    }
    if crawl.executable_pages {
        command.arg("--executable-pages");
    }
    // A file rather than a pipe: a watched trial is waited for by polling,
    // with no reader of a pipe beside it.
    let mut printed = tempfile::tempfile().context("creating the trial's output file")?;
    let mut child = command
        .stdout(
            printed
                .try_clone()
                .context("sharing the trial's output file")?,
        )
        .stderr(Stdio::inherit())
        .spawn()
        .context("running xtask compare trial")?;
    if let Some(reason) = watched(&mut child, &serving.address, watch)? {
        return Ok(Outcome::Stopped(reason));
    }
    let status = child.wait().context("waiting for xtask compare trial")?;
    drop(serving);
    if !status.success() {
        bail!(
            "{}'s trial in scenario {} failed",
            cell.name,
            cell.scenario.name()
        );
    }
    let mut figures = Vec::new();
    printed
        .seek(SeekFrom::Start(0))
        .and_then(|_| printed.read_to_end(&mut figures))
        .context("reading the trial's figures")?;
    serde_json::from_slice(&figures)
        .map(Outcome::Done)
        .with_context(|| {
            format!(
                "parsing the trial's figures: {}",
                String::from_utf8_lossy(&figures)
            )
        })
}

/// The pages of the site `config`'s tool crawls in `scenario`, where the
/// comparison's site has `pages` ([`Scenario::site_pages`]): Spider's at
/// `--spider-pages` when given.
fn pages_of(scenario: Scenario, config: Config, pages: u64, spider_pages: Option<u64>) -> u64 {
    let pages = scenario.site_pages(pages);
    match config {
        Config::Spider { .. } => spider_pages.unwrap_or(pages),
        Config::Foxhound { .. }
        | Config::Crawlee
        | Config::Scrapy
        | Config::CrawleeRs
        | Config::Pagesift
        | Config::OpenSpider
        | Config::FastCrw
        | Config::Crawlberg => pages,
    }
}

/// Whether a measured cell runs its strace and growth passes as well as
/// its timed trials: every scenario's but the long one's (see
/// [`Scenario::timed_only`]).
fn runs_counted((cell, _): &(&'static Cell, Config)) -> bool {
    !cell.scenario.timed_only()
}

/// A timed trial's cell, its tool's configuration and its crawl.
type Point = (&'static Cell, Config, Crawl);

/// Each of `cells` on its scenario's site where the comparison's has
/// `pages` (see [`pages_of`]), with the requests its scenario has in
/// flight.
fn main_points(cells: &[(&'static Cell, Config)], options: &Options, pages: u64) -> Vec<Point> {
    cells
        .iter()
        .map(|&(cell, config)| {
            let site_pages = pages_of(cell.scenario, config, pages, options.spider_pages);
            (cell, config, Crawl::of(cell.scenario, site_pages))
        })
        .collect()
}

/// The sizes cell's points: each scenario 1 main cell of [`SIZE_TOOLS`]
/// the run measures, at each site of [`SIZES`] and each concurrency of
/// [`SIZE_CONCURRENCIES`] it runs at ([`size_not_run`]), its samples
/// reading the executable's own pages; Spider's at 64 in flight on a page
/// channel of [`matrix::SPIDER_CAPACITY_AT_64`].
fn size_points(options: &Options) -> Vec<Point> {
    let cells: Vec<_> = measured(options)
        .into_iter()
        .filter(|(cell, config)| {
            cell.scenario == Scenario::Raw
                && cell.row == Row::Main
                && SIZE_TOOLS.contains(&config.tool())
        })
        .collect();
    SIZES
        .iter()
        .flat_map(|&pages| {
            SIZE_CONCURRENCIES.iter().map(move |&concurrency| Crawl {
                pages,
                concurrency,
                executable_pages: true,
            })
        })
        .flat_map(|crawl| {
            cells
                .iter()
                .filter(move |(_, config)| size_not_run(config.tool(), crawl.concurrency).is_none())
                .map(move |&(cell, config)| {
                    (cell, at_concurrency(config, crawl.concurrency), crawl)
                })
        })
        .collect()
}

/// `config` at `concurrency` in flight: Spider's page channel at 64 is
/// [`matrix::SPIDER_CAPACITY_AT_64`], every other configuration its row's.
fn at_concurrency(config: Config, concurrency: u32) -> Config {
    match config {
        Config::Spider { build, .. } if concurrency == 64 => Config::Spider {
            build,
            capacity: matrix::SPIDER_CAPACITY_AT_64,
        },
        Config::Foxhound { .. }
        | Config::Spider { .. }
        | Config::Crawlee
        | Config::Scrapy
        | Config::CrawleeRs
        | Config::Pagesift
        | Config::OpenSpider
        | Config::FastCrw
        | Config::Crawlberg => config,
    }
}

/// Runs `options.trials` timed trials of each of `points`, one of a cell
/// that runs one ([`Cell::one_trial`]), in one random order, each watched
/// (see [`watch::stopped`]): a stopped trial runs once more, and a point
/// whose trial is stopped twice is not measured, its later trials not run. Returns the trials and the points stopped.
fn timed(
    executables: &Executables<'_>,
    options: &Options,
    points: &[Point],
    state: &mut u64,
    pass: &str,
) -> Result<(Vec<Trial>, Vec<StoppedPoint>)> {
    let mut trials = Vec::new();
    let mut stopped: Vec<StoppedPoint> = Vec::new();
    // Whether the point of `scenario`'s `cell` at `pages` and
    // `concurrency` was stopped twice.
    let is_stopped =
        |stopped: &[StoppedPoint], scenario, cell: &str, pages: u64, concurrency: u32| {
            stopped.iter().any(|point| {
                point.run.scenario == scenario
                    && point.run.cell == cell
                    && point.site_pages == pages
                    && point.concurrency == concurrency
            })
        };
    for ((cell, config, crawl), number) in
        order(points, |(cell, _, _)| cell.runs(options.trials), state)
    {
        if is_stopped(
            &stopped,
            cell.scenario,
            cell.name,
            crawl.pages,
            crawl.concurrency,
        ) {
            continue;
        }
        let named = format!(
            "{pass}{} {} trial {number} ({} pages, {} in flight)",
            cell.scenario.name(),
            cell.name,
            crawl.pages,
            crawl.concurrency,
        );
        let mut started_at = None;
        let outcome = retried(|| {
            started_at = if waits_for_quiet(options) {
                quiet()
            } else {
                load()
            };
            let outcome = trial(
                executables,
                cell,
                config,
                crawl,
                ("timed", None),
                Watch::Timed(number, crawl.pages),
            )?;
            if let Outcome::Stopped(reason) = &outcome {
                output::line(&format!("{named}: {reason}."));
            }
            Ok(outcome)
        })?;
        let (measured, retried): (Measured, _) = match outcome {
            Retried::Done { measured, retried } => (measured, retried),
            Retried::Stopped(reason) => {
                output::line(&format!(
                    "{named} was stopped twice; the cell is not measured there."
                ));
                stopped.push(StoppedPoint {
                    site_pages: crawl.pages,
                    concurrency: crawl.concurrency,
                    run: StoppedRun {
                        scenario: cell.scenario,
                        cell: cell.name.to_owned(),
                        tool: config.tool(),
                        number,
                        reason,
                    },
                });
                continue;
            }
        };
        output::line(&format!(
            "{named}: {} pages in {:.3} s, peak {} bytes",
            measured.work.pages, measured.seconds, measured.peak_bytes,
        ));
        trials.push(Trial {
            scenario: cell.scenario,
            cell: cell.name.to_owned(),
            tool: config.tool(),
            number,
            site_pages: crawl.pages,
            concurrency: crawl.concurrency,
            load: started_at,
            retried,
            measured,
        });
    }
    // A stopped point's trials before its stop are no figure either.
    trials.retain(|trial| {
        !is_stopped(
            &stopped,
            trial.scenario,
            &trial.cell,
            trial.site_pages,
            trial.concurrency,
        )
    });
    Ok((trials, stopped))
}

/// The stopped runs of `points`, each at its cell's one size.
fn runs_of(points: Vec<StoppedPoint>) -> Vec<StoppedRun> {
    points.into_iter().map(|point| point.run).collect()
}

/// The file in `folder` the counting tool of `cell`'s run `number` writes
/// its output to: named after the run's scenario, cell and number, with no
/// space in it.
pub(crate) fn counted_output(folder: &Path, cell: &Cell, number: u32) -> PathBuf {
    folder.join(format!(
        "{}-{}-{number}.out",
        cell.scenario.name(),
        cell.name.replace(' ', "-")
    ))
}

/// Empties `folder` of an earlier run's Callgrind outputs, creating it when
/// it is not there, so the folder holds one run's outputs.
fn emptied_of_outputs(folder: &Path) -> Result<()> {
    std::fs::create_dir_all(folder).with_context(|| format!("creating {}", folder.display()))?;
    let entries =
        std::fs::read_dir(folder).with_context(|| format!("reading {}", folder.display()))?;
    for entry in entries {
        let path = entry
            .with_context(|| format!("reading {}", folder.display()))?
            .path();
        if path.extension().is_some_and(|extension| extension == "out") {
            std::fs::remove_file(&path).with_context(|| format!("removing {}", path.display()))?;
        }
    }
    Ok(())
}

/// Whether one of `cell`'s runs in `done` did not fetch the site of `pages`
/// pages: under strace, the report will read the cell as not measured, so
/// its later runs would count nothing kept.
pub(crate) fn set_aside(done: &[CountedRun], cell: &Cell, pages: u64) -> bool {
    done.iter().any(|run| {
        run.scenario == cell.scenario && run.cell == cell.name && !run.work.is_the_site_of(pages)
    })
}

/// Runs a counted cell: `runs` runs of each of `cells` under `mode`, one of
/// a cell that runs one ([`Cell::one_trial`]), each
/// run's counting tool writing its output into `outputs`. Under strace
/// (`watched`), a run whose crawl stalls is stopped (see [`watch::stopped`]): its
/// cell is not measured, and its later runs do not run; nor do the later
/// runs of a cell one of whose runs did not fetch the site, which the
/// report sets aside as not measured (the same-work check's
/// `strace_runs_of_other_work_set_aside`).
fn counted(
    executables: &Executables<'_>,
    options: &Options,
    cells: &[(&'static Cell, Config)],
    runs: u32,
    (mode, watched): (&str, bool),
    outputs: &Path,
    state: &mut u64,
) -> Result<CountedCell> {
    let mut done: Vec<CountedRun> = Vec::new();
    let mut stopped: Vec<StoppedRun> = Vec::new();
    for ((cell, config), number) in order(cells, |(cell, _)| cell.runs(runs), state) {
        if stopped
            .iter()
            .any(|run| run.scenario == cell.scenario && run.cell == cell.name)
            || watched
                && set_aside(
                    &done,
                    cell,
                    pages_of(cell.scenario, config, SITE_PAGES, options.spider_pages),
                )
        {
            continue;
        }
        let out = counted_output(outputs, cell, number);
        let pages = pages_of(cell.scenario, config, SITE_PAGES, options.spider_pages);
        let outcome = trial(
            executables,
            cell,
            config,
            Crawl::of(cell.scenario, pages),
            (mode, Some(&out)),
            if watched {
                Watch::Counted(number)
            } else {
                Watch::None
            },
        )?;
        let counted: Counted = match outcome {
            Outcome::Done(counted) => counted,
            Outcome::Stopped(reason) => {
                output::line(&format!(
                    "{} {} under {mode} {number}: {reason}; the cell is not measured.",
                    cell.scenario.name(),
                    cell.name,
                ));
                stopped.push(StoppedRun {
                    scenario: cell.scenario,
                    cell: cell.name.to_owned(),
                    tool: config.tool(),
                    number,
                    reason,
                });
                continue;
            }
        };
        output::line(&format!(
            "{} {} under {mode} {number}: {} pages, {} counted, {} syncs",
            cell.scenario.name(),
            cell.name,
            counted.work.pages,
            counted.count,
            counted.syncs
        ));
        done.push(CountedRun {
            scenario: cell.scenario,
            cell: cell.name.to_owned(),
            tool: config.tool(),
            number,
            count: counted.count,
            syncs: counted.syncs,
            work: counted.work,
            extracted: counted.extracted,
        });
    }
    Ok(CountedCell::Measured {
        trials: runs,
        runs: done,
        stopped,
    })
}

/// What a run of `options` declares before its first trial.
fn declared(options: &Options) -> Declared {
    Declared {
        trials: options.trials,
        statistic: STATISTIC.to_owned(),
        concurrency: CONCURRENCY,
        site: format!("/sites/{SITE_PAGES}/1"),
        spider: SPIDER_VERSION.to_owned(),
        crawlee: CRAWLEE_VERSION.to_owned(),
        scrapy: SCRAPY_VERSION.to_owned(),
        crawlee_rs: CRAWLEE_RS_COMMIT.to_owned(),
        pagesift: PAGESIFT_VERSION.to_owned(),
        openspider: OPENSPIDER_COMMIT.to_owned(),
        fastcrw: FASTCRW_VERSION.to_owned(),
        crawlberg: CRAWLBERG_VERSION.to_owned(),
        playwright: PLAYWRIGHT_VERSION.to_owned(),
        scrapy_playwright: SCRAPY_PLAYWRIGHT_VERSION.to_owned(),
        browser: crate::test_facts::CHROME_FOR_TESTING_BUILD.to_owned(),
        scenarios: options.scenarios.clone(),
        command: options.command.clone(),
    }
}

/// The system-call cell: each of `cells`' runs under strace, or not run,
/// with why, on a host without it.
fn syscalls(
    executables: &Executables<'_>,
    options: &Options,
    cells: &[(&'static Cell, Config)],
    state: &mut u64,
) -> Result<CountedCell> {
    if let Some(reason) = strace_missing() {
        output::line(&format!("The system-call cell does not run: {reason}."));
        return Ok(CountedCell::NotRun { reason });
    }
    // strace's counts are read once and kept nowhere.
    let folder = tempfile::tempdir().context("creating the counts' folder")?;
    counted(
        executables,
        options,
        cells,
        options.trials,
        ("syscalls", true),
        folder.path(),
        state,
    )
}

/// Why a cell the run has not reached yet is not run, as the trials file
/// kept before it says.
fn not_reached(cell: &str) -> String {
    format!("the run ended before its {cell} cell")
}

/// Runs the sizes cell's trials, [`size_points`], or says why it does not.
fn sizes(executables: &Executables<'_>, options: &Options, state: &mut u64) -> Result<SizesCell> {
    let points = size_points(options);
    if points.is_empty() {
        return Ok(SizesCell::NotRun {
            reason: "the run asked for no scenario 1 main cell of Foxhound or Spider".to_owned(),
        });
    }
    let (trials_run, stopped) = timed(executables, options, &points, state, "sizes ")?;
    Ok(SizesCell::Measured {
        trials: options.trials,
        trials_run,
        stopped,
    })
}

/// Runs the growth cell's trials of `cells` when the run asks for it with
/// `--growth`, or says why it does not.
fn growth(
    executables: &Executables<'_>,
    options: &Options,
    cells: &[(&'static Cell, Config)],
    state: &mut u64,
) -> Result<GrowthCell> {
    if !options.growth {
        return Ok(GrowthCell::NotRun {
            reason: format!(
                "the growth cell runs with `--growth`: each measured cell crawls the {GROWTH_PAGES}-page site {} times, hours at `run_state.durability: full`",
                options.trials
            ),
        });
    }
    let points = main_points(cells, options, GROWTH_PAGES);
    let (trials_run, stopped) = timed(executables, options, &points, state, "growth ")?;
    Ok(GrowthCell::Measured {
        trials: options.trials,
        site_pages: GROWTH_PAGES,
        trials_run,
        stopped: runs_of(stopped),
    })
}

/// Runs each hostile pages' cell of `cells` [`RULE_RUNS`] times, or the
/// run's trials when fewer, in one random order: the rules its tool broke
/// on each task of its pass. Each run bounds its own tasks' crawls
/// (`trial::rules`), so it is not watched.
fn rules(
    executables: &Executables<'_>,
    options: &Options,
    cells: &[(&'static Cell, Config)],
    state: &mut u64,
) -> Result<Vec<RuledRun>> {
    let mut runs = Vec::new();
    for ((cell, config), number) in order(
        cells,
        |(cell, _)| cell.runs(RULE_RUNS.min(options.trials)),
        state,
    ) {
        let ruled: Ruled = match trial(
            executables,
            cell,
            config,
            Crawl::of(cell.scenario, SITE_PAGES),
            ("rules", None),
            Watch::None,
        )? {
            Outcome::Done(ruled) => ruled,
            Outcome::Stopped(reason) => bail!("an unwatched run was stopped: {reason}"),
        };
        for task in &ruled.tasks {
            output::line(&format!(
                "{} {} run {number}, {}: {} requests, {} retry_after_ignored, {} over_in_flight_cap, {} honeypot, flagged {}, {} refused, {} duplicate, {} trap, {} of {} wanted pages{}",
                cell.scenario.name(),
                cell.name,
                task.task,
                task.requests,
                task.retry_after_ignored,
                task.over_in_flight_cap,
                task.honeypot,
                task.flagged,
                task.refused,
                task.duplicate,
                task.trap,
                task.wanted_fetched,
                task.wanted,
                task.stopped
                    .as_ref()
                    .map_or_else(String::new, |reason| format!(", {reason}")),
            ));
        }
        runs.push(RuledRun {
            scenario: cell.scenario,
            cell: cell.name.to_owned(),
            tool: config.tool(),
            number,
            tasks: ruled.tasks,
        });
    }
    Ok(runs)
}

/// Runs every declared trial and returns the trials file's contents. `keep`
/// is handed them as they stand once the timed trials are in, again once
/// the Collie scenario's are, again once the sizes cell is, again once the
/// system-call cell is, again once the growth cell is and again once the
/// Callgrind cell is, so a failure in a later part, the hostile pages'
/// passes included, loses none of them.
pub(crate) fn run(
    executables: &Executables<'_>,
    commit: String,
    options: &Options,
    keep: &mut impl FnMut(&Comparison) -> Result<()>,
) -> Result<Comparison> {
    if !page_cache::DROPS {
        output::line(page_cache::NOT_DROPPED);
    }
    let machine = machine(options.machine.clone());
    let order_seed = drawn_seed();
    let mut state = order_seed;
    let measured = measured(options);
    let (trials, stopped) = timed(
        executables,
        options,
        &main_points(&measured, options, SITE_PAGES),
        &mut state,
        "",
    )?;
    let mut comparison = Comparison {
        declared: declared(options),
        machine,
        commit,
        order_seed,
        trials,
        stopped: runs_of(stopped),
        syscalls: CountedCell::NotRun {
            reason: not_reached("system-call"),
        },
        growth: GrowthCell::NotRun {
            reason: not_reached("growth"),
        },
        sizes: SizesCell::NotRun {
            reason: not_reached("sizes"),
        },
        instructions: CountedCell::NotRun {
            reason: not_reached("Callgrind"),
        },
        collie: CollieScenario::NotRun {
            reason: not_reached("Collie"),
        },
        rules: Vec::new(),
    };
    keep(&comparison)?;
    comparison.collie =
        self::collie::collie_trials(executables.collie.as_ref(), options, &mut state)?;
    keep(&comparison)?;
    comparison.sizes = sizes(executables, options, &mut state)?;
    keep(&comparison)?;
    let counted_cells: Vec<_> = measured.iter().copied().filter(runs_counted).collect();
    comparison.syscalls = syscalls(executables, options, &counted_cells, &mut state)?;
    keep(&comparison)?;
    comparison.growth = growth(executables, options, &counted_cells, &mut state)?;
    keep(&comparison)?;
    // ADR 0140's rows compare Foxhound's instructions with Spider's, each
    // on its main rows, in scenario 1 and in the extraction scenario.
    let instruction_cells: Vec<_> = measured
        .iter()
        .copied()
        .filter(|(cell, config)| {
            INSTRUCTION_SCENARIOS.contains(&cell.scenario)
                && cell.row == Row::Main
                && INSTRUCTION_TOOLS.contains(&config.tool())
        })
        .collect();
    comparison.instructions = match callgrind_missing() {
        _ if instruction_cells.is_empty() => CountedCell::NotRun {
            reason: "the run asked for no cell of scenario 1 or of the extraction scenario"
                .to_owned(),
        },
        Some(reason) => {
            output::line(&format!("The Callgrind cell does not run: {reason}."));
            CountedCell::NotRun { reason }
        }
        None => {
            emptied_of_outputs(&options.callgrind_outputs)?;
            let cell = counted(
                executables,
                options,
                &instruction_cells,
                INSTRUCTION_RUNS,
                ("callgrind", false),
                &options.callgrind_outputs,
                &mut state,
            )?;
            output::line(&format!(
                "Callgrind's outputs are kept in {}.",
                options.callgrind_outputs.display()
            ));
            cell
        }
    };
    keep(&comparison)?;
    comparison.rules = rules(executables, options, &ruled(options), &mut state)?;
    Ok(comparison)
}

#[cfg(test)]
mod tests;
