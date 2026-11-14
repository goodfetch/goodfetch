//! The workloads `xtask pgo` trains the release build's profile on, and the
//! ones it holds out to validate it (ADR 0057's note of 2026-11-14): both
//! Products' work, as the comparison's own trials run it. Foxhound crawls
//! scenario 1's site at four sizes, the article site with extraction, and
//! the resident-memory check's crawl at the catalog's defaults; Collie acts
//! on the lab's pages through its row of the Collie scenario. Held out:
//! Foxhound's crawl-like scenario, over TLS and HTTP/2, and Collie's pages
//! opened one after another.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous: a workload runs as a blocking child process (STYLE.md)"
)]

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Instant;

use anyhow::{Context as _, Result, bail};
use serde::Deserialize;

use super::collie;
use super::matrix::Scenario;
use super::trial::Outcome;
use super::trials::{Contender, Exercise};
use crate::rss;

/// The executables a workload runs.
#[derive(Clone, Copy)]
pub(crate) struct Executables<'a> {
    pub(crate) proving_ground: &'a Path,
    /// Foxhound's and Collie's Attached engines.
    pub(crate) foxhound: &'a Path,
    pub(crate) collie: &'a Path,
    /// Chrome for Testing's folder.
    pub(crate) browser: &'a Path,
    /// `xtask/compare-collie/act.ts`, as an absolute path.
    pub(crate) collie_driver: &'a Path,
    /// xtask's own `GOODFETCH_*` variables, which the tools run without.
    pub(crate) goodfetch_variables: &'a [OsString],
}

/// One workload.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Workload {
    /// Scenario 1's crawl of the site of this many pages.
    Raw(u64),
    /// Scenario 4's crawl of the article site of this many pages, its
    /// pages' titles and Main content extracted.
    Extraction(u64),
    /// The resident-memory check's crawl of the 1,000-page site at the
    /// catalog's defaults (`xtask rss probe`'s `crawl-1000`).
    Defaults,
    /// The crawl-like scenario's crawl of the article site of this many
    /// pages over TLS and HTTP/2 under a 20 ms round trip.
    CrawlLike(u64),
    /// Collie's row of a Collie scenario's cell.
    Collie(Exercise),
}

/// The workloads the profile is trained on, in the order they run.
pub(crate) const TRAINING: [Workload; 12] = [
    Workload::Raw(100),
    Workload::Raw(1_000),
    Workload::Raw(1_000),
    Workload::Raw(1_000),
    Workload::Raw(10_000),
    Workload::Extraction(1_000),
    Workload::Defaults,
    Workload::Collie(Exercise::Click),
    Workload::Collie(Exercise::Form),
    Workload::Collie(Exercise::Navigation),
    Workload::Collie(Exercise::Sessions),
    Workload::Collie(Exercise::AtOnce),
];

/// Scenario 1's timed workload, which the release build's speed is read on.
pub(crate) const SCENARIO_1: Workload = Workload::Raw(1_000);

/// One workload per Product the profile is not trained on.
pub(crate) const HELD_OUT: [Workload; 2] = [
    Workload::CrawlLike(1_000),
    Workload::Collie(Exercise::Opening),
];

impl Workload {
    /// Its name in what `xtask pgo` prints.
    pub(crate) fn name(self) -> String {
        match self {
            Self::Raw(pages) => format!("foxhound raw {pages} pages"),
            Self::Extraction(pages) => format!("foxhound extraction {pages} pages"),
            Self::Defaults => "foxhound crawl-1000 at the defaults".to_owned(),
            Self::CrawlLike(pages) => format!("foxhound crawl-like {pages} pages"),
            Self::Collie(exercise) => format!("collie {exercise:?}"),
        }
    }

    /// Runs the workload once and returns its wall time in seconds: a
    /// Foxhound trial's from its tool's start to its exit, a Collie
    /// trial's between its marks.
    pub(crate) fn run(self, executables: &Executables<'_>) -> Result<f64> {
        match self {
            Self::Raw(pages) => crawl(executables, Scenario::Raw, pages),
            Self::Extraction(pages) => crawl(executables, Scenario::Extraction, pages),
            Self::CrawlLike(pages) => crawl(executables, Scenario::CrawlLike, pages),
            Self::Defaults => defaults(executables),
            Self::Collie(exercise) => act(executables, exercise),
        }
    }
}

/// What a timed trial prints, of which the wall time is read.
#[derive(Deserialize)]
struct Timed {
    seconds: f64,
}

/// One timed trial of Foxhound's cell of `scenario` against a fresh
/// Proving ground, through `xtask compare trial`, as the comparison runs
/// it.
fn crawl(executables: &Executables<'_>, scenario: Scenario, pages: u64) -> Result<f64> {
    rss::page_cache::drop_cached(executables.foxhound)?;
    let serving = rss::crawl::serve(
        executables.proving_ground,
        scenario.network().network_profile(),
    )?;
    let probe = std::env::current_exe().context("finding xtask's own executable")?;
    let output = Command::new(probe)
        .args(["compare", "trial", "--scenario", scenario.name()])
        .args(["--cell", "foxhound", "--executable"])
        .arg(executables.foxhound)
        .args(["--address", &serving.address])
        .args(["--pages", &scenario.site_pages(pages).to_string()])
        .args(["--concurrency", &scenario.concurrency().to_string()])
        .args(["--mode", "timed", "--browser"])
        .arg(executables.browser)
        .stderr(Stdio::inherit())
        .output()
        .context("running xtask compare trial")?;
    drop(serving);
    if !output.status.success() {
        bail!("Foxhound's trial in scenario {} failed", scenario.name());
    }
    let timed: Timed = serde_json::from_slice(&output.stdout).with_context(|| {
        format!(
            "parsing the trial's figures: {}",
            String::from_utf8_lossy(&output.stdout)
        )
    })?;
    Ok(timed.seconds)
}

/// The resident-memory check's 1,000-page crawl at the catalog's defaults,
/// through `xtask rss probe`, timed from its start to its end.
fn defaults(executables: &Executables<'_>) -> Result<f64> {
    let scenario = rss::scenario::Scenario::Crawl1000;
    let serving = rss::crawl::serve(executables.proving_ground, "none")?;
    let probe = std::env::current_exe().context("finding xtask's own executable")?;
    let started = Instant::now();
    let status = Command::new(probe)
        .args(["rss", "probe", "--scenario", scenario.name()])
        .args(["--needs", "foxhound", "--address", &serving.address, "--"])
        .arg(executables.foxhound)
        .args(scenario.arguments())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .status()
        .context("running xtask rss probe")?;
    let seconds = started.elapsed().as_secs_f64();
    drop(serving);
    if !status.success() {
        bail!("xtask rss probe of {} failed", scenario.name());
    }
    Ok(seconds)
}

/// One trial of Collie's row of `exercise`'s cell.
fn act(executables: &Executables<'_>, exercise: Exercise) -> Result<f64> {
    let tools = collie::Tools {
        proving_ground: executables.proving_ground,
        engine: executables.collie,
        driver: executables.collie_driver.to_path_buf(),
        // Collie's row runs neither: its driver is `driver`.
        arms_driver: PathBuf::new(),
        chromiumoxide: PathBuf::new(),
        browser: executables.browser,
        goodfetch_variables: executables.goodfetch_variables,
    };
    rss::page_cache::drop_cached(executables.collie)?;
    match collie::trial(&tools, exercise, Contender::Collie, 1, None)? {
        Outcome::Done(trial) => Ok(trial.wall_seconds),
        Outcome::Stopped(reason) => bail!("Collie's {exercise:?} trial stopped: {reason}"),
    }
}

/// Chrome for Testing's folder, where Playwright installs it.
pub(crate) fn browser() -> Result<PathBuf> {
    super::trial::chrome_folder().with_context(|| {
        format!(
            "Playwright's Chrome for Testing {} is not installed: run `pnpm exec playwright install chromium`",
            crate::test_facts::CHROME_FOR_TESTING_BUILD
        )
    })
}

/// Collie's driver in the checkout at `root`, as an absolute path.
pub(crate) fn collie_driver(root: &Path) -> Result<PathBuf> {
    super::absolute(&root.join(super::COLLIE_DRIVER))
}

/// The median of `values`, as the comparison's report takes it.
pub(crate) fn median(values: &[f64]) -> f64 {
    super::report::median(values)
}
