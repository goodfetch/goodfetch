//! The report: for each scenario, each cell's mapping and every metric of
//! the owner's list, each the median of the cell's trials with its spread
//! (the lowest and the highest trial), or why the cell or the metric gives
//! no figure; `foxhound.spider_instruction_ratio` and the extraction
//! scenario's two ratios from the Callgrind cell ([`RATIOS`]); the sizes
//! cell's peak memory of Foxhound and Spider at each size and concurrency,
//! with the bar at 16 in flight (spec #2165); and the standings of Foxhound
//! against each competitor ([`standing`]). On the article site, a cell also
//! says what the output check found of its pages (#2136): a competitor
//! whose pages differ keeps its figures, labelled `output differs: N of
//! <pages>`, in its mapping, its column and its standings.
//! Labelled `local-controlled`, measured on the machine it names, with the
//! command that reruns it, and written only once the same-work check
//! passed.
//!
//! Size: one concern, the report: the figures of each cell and standing,
//! and the Markdown that writes them, which reads each of those figures.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use serde::Serialize;

use super::extracted::{Differing, Extracted};
use super::matrix::{
    self, CELLS, Cell, FILTER_LISTS_OFF, OutputCheck, Reading, Row, Scenario, Status,
};
use super::sampler::{CORE_WINDOW_SAMPLES, INTERVAL_MILLISECONDS};
use super::trials::{
    Comparison, CountedCell, CountedRun, Declared, GrowthCell, LatencyEnd, MILESTONES, Machine,
    QUIET_LOAD, STRACE_QUIET_LOAD, TRIALS, Trial,
};

mod auto;
mod published;
mod rules;
mod sizes;
mod standing;

#[cfg(test)]
mod tests;

use sizes::{Sizes, sizes, sizes_markdown};

/// The span of one window of the core sampling.
const CORE_WINDOW_MILLISECONDS: u64 = CORE_WINDOW_SAMPLES as u64 * INTERVAL_MILLISECONDS;

/// A ratio the Callgrind cell gives: a row of the Quality register (ADR
/// 0140), Foxhound's instructions per page on one of its main rows over
/// Spider's main row's in the same scenario.
struct RatioRow {
    row: &'static str,
    scenario: Scenario,
    /// Foxhound's cell.
    foxhound: &'static str,
    described: &'static str,
}

/// The Callgrind cell's ratios: scenario 1's, and the extraction
/// scenario's two (#2136; the owner's decision of 2026-11-10, #2252 Q7).
/// At Foxhound's defaults Spider does not do the Clean-up rules' work, so
/// that ratio carries no parity Contract; with Foxhound's Filter lists off
/// both read the same Main content, and the distance between the two is
/// the clean-up's cost (ADR 0023).
const RATIOS: [RatioRow; 3] = [
    RatioRow {
        row: "foxhound.spider_instruction_ratio",
        scenario: Scenario::Raw,
        foxhound: "foxhound",
        described: "scenario 1 of ADR 0138's matrix",
    },
    RatioRow {
        row: "foxhound.spider_extraction_instruction_ratio",
        scenario: Scenario::Extraction,
        foxhound: "foxhound",
        described: "the extraction scenario of ADR 0138's matrix, Foxhound at its defaults",
    },
    RatioRow {
        row: "foxhound.spider_extraction_instruction_ratio_filter_lists_off",
        scenario: Scenario::Extraction,
        foxhound: FILTER_LISTS_OFF,
        described: "the extraction scenario of ADR 0138's matrix, Foxhound's Filter lists off",
    },
];

/// Spider's main row, in every scenario the Callgrind cell counts.
const SPIDER: &str = "spider";

/// A metric's unit, as the Markdown writes it.
#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Unit {
    PagesPerSecond,
    Seconds,
    Bytes,
    Cores,
    Count,
    Percent,
    /// A share of the pages, written as a percentage.
    Share,
}

/// A figure over a cell's trials.
#[derive(Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub(crate) enum Figure {
    Measured {
        median: f64,
        low: f64,
        high: f64,
        unit: Unit,
    },
    NotMeasured {
        reason: String,
    },
}

/// One cell's report.
#[derive(Serialize)]
pub(crate) struct CellReport {
    pub(crate) cell: &'static str,
    pub(crate) tool: &'static str,
    /// Whether its figures are the tool's, or a labelled variant's of them.
    pub(crate) row: Row,
    pub(crate) mapping: &'static str,
    pub(crate) pending: &'static [&'static str],
    /// Why the cell ran one trial of each pass, where the run asked for
    /// more ([`crate::compare::matrix::Cell::one_trial`]).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) one_trial: Option<&'static str>,
    /// What the output check found of the pages its trials wrote, on the
    /// article site: under each check, its worst trial's.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) output: Option<Extracted>,
    #[serde(flatten)]
    pub(crate) status: CellStatus,
}

#[derive(Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub(crate) enum CellStatus {
    Measured {
        trials: usize,
        /// What the same-work check found every trial did: the pages the
        /// Proving ground saw it fetch, of the site's, and their bytes.
        pages: u64,
        site_pages: u64,
        bytes: u64,
        metrics: BTreeMap<&'static str, Figure>,
        /// Pages per second in each tenth of the crawl.
        curve: Vec<Figure>,
    },
    NotSupported {
        reason: &'static str,
    },
    /// Its timed trial was stopped twice (see `run::watch::stopped`), or the
    /// ticket that brings it has not landed.
    NotMeasured {
        reason: String,
    },
    /// A hostile pages' cell (#2439): the rules its tool broke on each task
    /// of its pass, over its runs.
    Ruled {
        runs: usize,
        tasks: Vec<rules::RuledTask>,
    },
    /// An auto cell (#2411): over its runs on the rungs site, its rate, the
    /// pages of the site's manifest it rendered and those whose Records
    /// were wrong.
    Auto {
        runs: usize,
        pages: u64,
        rate: Figure,
        rendered: Figure,
        wrong: Figure,
    },
}

/// One scenario's report.
#[derive(Serialize)]
pub(crate) struct ScenarioReport {
    pub(crate) scenario: Scenario,
    pub(crate) title: &'static str,
    pub(crate) cells: Vec<CellReport>,
    pub(crate) standings: Vec<standing::Standings>,
}

/// Scenario 1's figures of a tool, its main row's, which `just targets`
/// reads (ADR 0138: the Spider target is read on scenario 1).
#[derive(Serialize)]
pub(crate) struct Figures {
    pub(crate) pages_per_second: f64,
    pub(crate) peak_bytes: u64,
}

/// One ratio of the Callgrind cell's.
#[derive(Serialize)]
pub(crate) struct InstructionRatio {
    pub(crate) row: &'static str,
    pub(crate) scenario: &'static str,
    /// Foxhound's cell.
    pub(crate) foxhound: &'static str,
    pub(crate) foxhound_per_page: f64,
    pub(crate) spider_per_page: f64,
    /// Foxhound's per page over Spider's, to three decimals, as the
    /// register's figures are.
    pub(crate) ratio: f64,
    /// Spider's label when its pages differ under the check Foxhound's
    /// cell is held to: the ratio is then of unlike work.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) spider_output_differs: Option<String>,
}

/// The Callgrind cell's figures.
#[derive(Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub(crate) enum Instructions<'a> {
    NotRun {
        reason: &'a str,
    },
    /// A ratio for each of [`RATIOS`] whose scenario the cell counted.
    Measured {
        ratios: Vec<InstructionRatio>,
    },
}

/// The report, as `report.json` holds it.
#[derive(Serialize)]
pub(crate) struct Report<'a> {
    pub(crate) label: &'static str,
    pub(crate) published: bool,
    pub(crate) machine: &'a Machine,
    pub(crate) commit: &'a str,
    pub(crate) declared: &'a Declared,
    /// Whether each measured cell ran the matrix's [`TRIALS`] trials at
    /// least: a quicker try's report says it falls short.
    pub(crate) enough_trials: bool,
    pub(crate) order_seed: u64,
    pub(crate) scenarios: Vec<ScenarioReport>,
    pub(crate) tools: BTreeMap<&'static str, Figures>,
    pub(crate) instructions: Instructions<'a>,
    pub(crate) sizes: Sizes<'a>,
}

/// The median of `values`, the mean of the middle two for an even count.
pub(super) fn median(values: &[f64]) -> f64 {
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let middle = sorted.len() / 2;
    match sorted.len() {
        0 => f64::NAN,
        length if length % 2 == 0 => f64::midpoint(sorted[middle - 1], sorted[middle]),
        _ => sorted[middle],
    }
}

/// `count` as a float, for a figure.
#[expect(
    clippy::cast_precision_loss,
    reason = "pages, bytes and counts here stay far below 2^52, where a float is exact"
)]
pub(super) fn float(count: u64) -> f64 {
    count as f64
}

pub(super) fn spread(values: &[f64], unit: Unit) -> Figure {
    if values.is_empty() || values.iter().any(|value| !value.is_finite()) {
        return Figure::NotMeasured {
            reason: "a trial gave no value".to_owned(),
        };
    }
    Figure::Measured {
        median: median(values),
        low: values.iter().copied().fold(f64::INFINITY, f64::min),
        high: values.iter().copied().fold(f64::NEG_INFINITY, f64::max),
        unit,
    }
}

fn not(reason: impl Into<String>) -> Figure {
    Figure::NotMeasured {
        reason: reason.into(),
    }
}

/// A trial's pages per second.
fn rate(trial: &Trial) -> f64 {
    float(trial.measured.work.pages) / trial.measured.seconds
}

/// Per 1,000 of the trial's pages.
fn per_thousand(trial: &Trial, value: f64) -> f64 {
    value * 1000.0 / float(trial.measured.work.pages)
}

/// The tree's peak: the highest sample, or the largest single process's
/// resident set the OS recorded when that is higher.
fn peak(trial: &Trial) -> u64 {
    trial
        .measured
        .sampled
        .as_ref()
        .map_or(0, |sampled| sampled.peak_bytes)
        .max(trial.measured.peak_bytes)
}

/// The median of `values` in whole bytes: the mean of the middle two,
/// rounded down, for an even count; 0 for none.
fn median_bytes(mut values: Vec<u64>) -> u64 {
    values.sort_unstable();
    let middle = values.len() / 2;
    match values.len() {
        0 => 0,
        length if length % 2 == 0 => u64::midpoint(values[middle - 1], values[middle]),
        _ => values[middle],
    }
}

/// Every metric of the owner's list over `trials`, one cell's, with its
/// counted runs and growth trials.
#[expect(
    clippy::too_many_lines,
    reason = "one figure for each metric of the owner's list, in its order"
)]
fn metrics(
    cell: &Cell,
    trials: &[&Trial],
    syscalls: &Result<Vec<&CountedRun>, &str>,
    growth: &Result<Vec<&Trial>, &str>,
) -> BTreeMap<&'static str, Figure> {
    let each = |value: &dyn Fn(&Trial) -> f64, unit| {
        let values: Vec<f64> = trials.iter().map(|trial| value(trial)).collect();
        spread(&values, unit)
    };
    let sampled = |value: &dyn Fn(&super::trials::Sampled) -> f64, unit| {
        let values: Option<Vec<f64>> = trials
            .iter()
            .map(|trial| trial.measured.sampled.as_ref().map(value))
            .collect();
        values.map_or_else(
            || not("this OS has no /proc to sample the process tree from"),
            |values| spread(&values, unit),
        )
    };
    let mut metrics = BTreeMap::new();
    let mut put = |name, figure| {
        metrics.insert(name, figure);
    };
    // Throughput.
    put("pages_per_second", each(&rate, Unit::PagesPerSecond));
    put(
        "first_page_seconds",
        each(&|trial| trial.measured.first_page_seconds, Unit::Seconds),
    );
    let records = trials
        .iter()
        .all(|trial| trial.measured.latency_end == Some(LatencyEnd::Record));
    put(
        "first_record_seconds",
        if records {
            each(
                &|trial| trial.measured.first_record_seconds.unwrap_or(f64::NAN),
                Unit::Seconds,
            )
        } else {
            not(format!(
                "{} keeps no Record; its pages' times run to their fetch",
                cell.tool.name()
            ))
        },
    );
    // Latency per page.
    put(
        "latency_p50_seconds",
        each(&|trial| trial.measured.latency.p50, Unit::Seconds),
    );
    put(
        "latency_p95_seconds",
        each(&|trial| trial.measured.latency.p95, Unit::Seconds),
    );
    put(
        "latency_p99_seconds",
        each(&|trial| trial.measured.latency.p99, Unit::Seconds),
    );
    // CPU.
    put(
        "user_seconds",
        each(&|trial| trial.measured.user_seconds, Unit::Seconds),
    );
    put(
        "system_seconds",
        each(&|trial| trial.measured.system_seconds, Unit::Seconds),
    );
    put(
        "cpu_seconds_per_1000_pages",
        each(
            &|trial| {
                per_thousand(
                    trial,
                    trial.measured.user_seconds + trial.measured.system_seconds,
                )
            },
            Unit::Seconds,
        ),
    );
    put(
        "average_cores",
        each(
            &|trial| {
                (trial.measured.user_seconds + trial.measured.system_seconds)
                    / trial.measured.seconds
            },
            Unit::Cores,
        ),
    );
    let cores: Option<Vec<f64>> = trials
        .iter()
        .map(|trial| trial.measured.sampled.as_ref()?.peak_cores)
        .collect();
    put(
        "peak_cores",
        match cores {
            Some(cores) => spread(&cores, Unit::Cores),
            None if trials.iter().any(|trial| trial.measured.sampled.is_none()) => {
                not("this OS has no /proc to sample the process tree from")
            }
            None => not(format!(
                "a trial ended within one window of the core sampling ({CORE_WINDOW_MILLISECONDS} ms)"
            )),
        },
    );
    // Memory.
    put(
        "peak_rss_bytes",
        each(&|trial| float(peak(trial)), Unit::Bytes),
    );
    let pss: Option<Vec<f64>> = trials
        .iter()
        .map(|trial| trial.measured.sampled.as_ref()?.peak_pss_bytes.map(float))
        .collect();
    put(
        "peak_pss_bytes",
        match pss {
            Some(pss) => spread(&pss, Unit::Bytes),
            None if trials.iter().any(|trial| trial.measured.sampled.is_none()) => {
                not("this OS has no /proc to sample the process tree from")
            }
            None if trials.iter().any(|trial| {
                trial
                    .measured
                    .sampled
                    .as_ref()
                    .is_some_and(|sampled| sampled.samples == 0)
            }) =>
            {
                not("a trial ended before the sampler's first sample of its process tree")
            }
            None => not(
                "a process of the tree kept its PSS from the sampler (its `/proc/<pid>/smaps_rollup` could not be read)",
            ),
        },
    );
    put(
        "average_rss_bytes",
        sampled(&|sampled| float(sampled.average_bytes), Unit::Bytes),
    );
    for (name, pages) in [
        ("rss_per_1000_pages_at_1000", MILESTONES[0]),
        ("rss_per_1000_pages_at_10000", MILESTONES[1]),
        ("rss_per_1000_pages_at_100000", MILESTONES[2]),
    ] {
        let at = |trial: &Trial| {
            trial
                .measured
                .sampled
                .as_ref()
                .and_then(|sampled| sampled.at_pages.iter().find(|(at, _)| *at == pages))
                .map_or(f64::NAN, |(_, bytes)| float(*bytes) * 1000.0 / float(pages))
        };
        let figure = match growth {
            Ok(growth) => {
                let values: Vec<f64> = growth.iter().map(|trial| at(trial)).collect();
                spread(&values, Unit::Bytes)
            }
            Err(reason) if pages <= trials.first().map_or(0, |trial| trial.site_pages) => {
                let values: Vec<f64> = trials.iter().map(|trial| at(trial)).collect();
                match spread(&values, Unit::Bytes) {
                    Figure::NotMeasured { .. } => not(*reason),
                    figure @ Figure::Measured { .. } => figure,
                }
            }
            Err(reason) => not(*reason),
        };
        put(name, figure);
    }
    // Storage.
    put(
        "read_bytes",
        each(&|trial| float(trial.measured.read_bytes), Unit::Bytes),
    );
    put(
        "written_bytes",
        each(&|trial| float(trial.measured.written_bytes), Unit::Bytes),
    );
    put(
        "footprint_bytes_per_1000_pages",
        each(
            &|trial| per_thousand(trial, float(trial.measured.footprint_bytes)),
            Unit::Bytes,
        ),
    );
    match syscalls {
        Ok(runs) => {
            let syncs: Vec<f64> = runs.iter().map(|run| float(run.syncs)).collect();
            let calls: Vec<f64> = runs
                .iter()
                .map(|run| float(run.count) / float(run.work.pages))
                .collect();
            put("syncs", spread(&syncs, Unit::Count));
            put("system_calls_per_page", spread(&calls, Unit::Count));
        }
        Err(reason) => {
            put("syncs", not(*reason));
            put("system_calls_per_page", not(*reason));
        }
    }
    // Network.
    put(
        "sent_bytes",
        each(&|trial| float(trial.measured.sent_bytes), Unit::Bytes),
    );
    put(
        "received_bytes",
        each(&|trial| float(trial.measured.received_bytes), Unit::Bytes),
    );
    put(
        "requests_sent",
        each(&|trial| float(trial.measured.requests_sent), Unit::Count),
    );
    put(
        "tcp_connections",
        each(&|trial| float(trial.measured.connections), Unit::Count),
    );
    put(
        "tls_connections",
        each(&|trial| float(trial.measured.tls_connections), Unit::Count),
    );
    // Process.
    put(
        "peak_threads",
        sampled(&|sampled| float(sampled.peak_threads), Unit::Count),
    );
    put(
        "peak_open_files",
        sampled(&|sampled| float(sampled.peak_open_files), Unit::Count),
    );
    put(
        "startup_seconds",
        each(&|trial| trial.measured.startup_seconds, Unit::Seconds),
    );
    put(
        "executable_bytes",
        each(&|trial| float(trial.measured.executable_bytes), Unit::Bytes),
    );
    put(
        "processes",
        each(&|trial| f64::from(trial.measured.processes), Unit::Count),
    );
    // Correctness.
    put(
        "pages",
        each(&|trial| float(trial.measured.work.pages), Unit::Count),
    );
    put(
        "duplicates",
        each(&|trial| float(trial.measured.duplicates), Unit::Count),
    );
    put(
        "errors",
        each(&|trial| float(trial.measured.errors), Unit::Count),
    );
    put(
        "skipped",
        each(
            &|trial| float(trial.measured.work.missing.len() as u64),
            Unit::Count,
        ),
    );
    let shares: Option<Vec<f64>> = trials
        .iter()
        .map(|trial| {
            let sample_pages = trial.measured.sample_pages?;
            Some(float(sample_pages) / float(trial.measured.work.pages))
        })
        .collect();
    put(
        "sample_share",
        match shares {
            Some(shares) if !shares.is_empty() => spread(&shares, Unit::Share),
            _ => not("it fetches no sample: only Foxhound's `auto` row does"),
        },
    );
    metrics
}

/// Why a cell of the long scenario has no strace or growth figures.
const TIMED_ONLY: &str = "the long scenario runs only its timed trials: strace over its 100,000 \
     pages would take hours, and its site is already the growth cell's length";

/// The runs of `cell` in a counted cell, or why there are none: the cell
/// did not run, a run of `cell` was stopped, or its scenario runs only its
/// timed trials.
fn runs_of<'a>(counted: &'a CountedCell, cell: &Cell) -> Result<Vec<&'a CountedRun>, &'a str> {
    if cell.scenario.timed_only() {
        return Err(TIMED_ONLY);
    }
    let of_cell = |scenario: Scenario, name: &str| scenario == cell.scenario && name == cell.name;
    match counted {
        CountedCell::NotRun { reason } => Err(reason),
        CountedCell::Measured { runs, stopped, .. } => {
            match stopped.iter().find(|run| of_cell(run.scenario, &run.cell)) {
                Some(run) => Err(&run.reason),
                None => Ok(runs
                    .iter()
                    .filter(|run| of_cell(run.scenario, &run.cell))
                    .collect()),
            }
        }
    }
}

/// The trials of `cell` among `trials`.
fn trials_of<'a>(trials: &'a [Trial], cell: &Cell) -> Vec<&'a Trial> {
    trials
        .iter()
        .filter(|trial| trial.scenario == cell.scenario && trial.cell == cell.name)
        .collect()
}

fn growth_of<'a>(growth: &'a GrowthCell, cell: &Cell) -> Result<Vec<&'a Trial>, &'a str> {
    if cell.scenario.timed_only() {
        return Err(TIMED_ONLY);
    }
    match growth {
        GrowthCell::NotRun { reason } => Err(reason),
        GrowthCell::Measured {
            trials_run,
            stopped,
            ..
        } => match stopped
            .iter()
            .find(|run| run.scenario == cell.scenario && run.cell == cell.name)
        {
            Some(run) => Err(&run.reason),
            None => Ok(trials_of(trials_run, cell)),
        },
    }
}

/// The pages that differ under `check`.
fn differing(extracted: &Extracted, check: OutputCheck) -> &Differing {
    match check {
        OutputCheck::Whole => &extracted.whole,
        OutputCheck::AdAllowed => &extracted.ad_allowed,
    }
}

/// Under each check, the worst of what the output check found of some
/// crawls: the one with the most differing pages. None when it read none.
fn worst<'a>(found: impl Iterator<Item = &'a Extracted>) -> Option<Extracted> {
    found.fold(None, |worst: Option<Extracted>, extracted| {
        let Some(mut worst) = worst else {
            return Some(extracted.clone());
        };
        if extracted.whole.pages > worst.whole.pages {
            worst.whole = extracted.whole.clone();
        }
        if extracted.ad_allowed.pages > worst.ad_allowed.pages {
            worst.ad_allowed = extracted.ad_allowed.clone();
        }
        Some(worst)
    })
}

/// The label of an output that differs under `check` on a site of
/// `site_pages` pages; none when every page read right.
fn output_differs(output: &Extracted, check: OutputCheck, site_pages: u64) -> Option<String> {
    let differing = differing(output, check);
    (differing.pages > 0).then(|| format!("output differs: {} of {site_pages}", differing.pages))
}

/// A cell's label when its output differs under its own check.
fn own_output_differs(cell: &CellReport, scenario: Scenario) -> Option<String> {
    let CellStatus::Measured { site_pages, .. } = &cell.status else {
        return None;
    };
    let check = matrix::cell(scenario, cell.cell)?.output_check()?;
    output_differs(cell.output.as_ref()?, check, *site_pages)
}

/// A cell's runs of the Callgrind cell.
fn counted<'a>(runs: &'a [CountedRun], scenario: Scenario, cell: &str) -> Vec<&'a CountedRun> {
    runs.iter()
        .filter(|run| run.scenario == scenario && run.cell == cell)
        .collect()
}

/// The median instructions per page over a cell's runs.
fn per_page(runs: &[&CountedRun]) -> f64 {
    let counts: Vec<f64> = runs
        .iter()
        .map(|run| float(run.count) / float(run.work.pages))
        .collect();
    median(&counts)
}

fn instructions(cell: &CountedCell) -> Instructions<'_> {
    match cell {
        CountedCell::NotRun { reason } => Instructions::NotRun { reason },
        CountedCell::Measured { runs, .. } => Instructions::Measured {
            ratios: RATIOS
                .iter()
                .filter_map(|row| {
                    let foxhound = counted(runs, row.scenario, row.foxhound);
                    let spider = counted(runs, row.scenario, SPIDER);
                    if foxhound.is_empty() || spider.is_empty() {
                        return None;
                    }
                    let (foxhound_per_page, spider_per_page) =
                        (per_page(&foxhound), per_page(&spider));
                    let spider_output_differs = matrix::cell(row.scenario, row.foxhound)
                        .and_then(Cell::output_check)
                        .and_then(|check| {
                            let output =
                                worst(spider.iter().filter_map(|run| run.extracted.as_ref()))?;
                            output_differs(&output, check, spider.first()?.work.pages)
                        });
                    Some(InstructionRatio {
                        row: row.row,
                        scenario: row.described,
                        foxhound: row.foxhound,
                        foxhound_per_page,
                        spider_per_page,
                        ratio: (foxhound_per_page / spider_per_page * 1000.0).round() / 1000.0,
                        spider_output_differs,
                    })
                })
                .collect(),
        },
    }
}

/// What the output check found of a measured cell's trials, on the article
/// site: under each check, its worst trial's.
fn output_of(trials: &Comparison, cell: &Cell, status: &CellStatus) -> Option<Extracted> {
    match status {
        CellStatus::Measured { .. } if cell.output_check().is_some() => worst(
            trials_of(&trials.trials, cell)
                .into_iter()
                .filter_map(|trial| trial.measured.extracted.as_ref()),
        ),
        CellStatus::Measured { .. }
        | CellStatus::NotSupported { .. }
        | CellStatus::NotMeasured { .. }
        | CellStatus::Ruled { .. }
        | CellStatus::Auto { .. } => None,
    }
}

/// Why `cell`'s timed trial was stopped twice, if it was.
fn stopped_reason(trials: &Comparison, cell: &Cell) -> Option<String> {
    trials
        .stopped
        .iter()
        .find(|run| run.scenario == cell.scenario && run.cell == cell.name)
        .map(|run| run.reason.clone())
}

/// The report of trials that passed the same-work check.
pub(crate) fn of(trials: &Comparison) -> Report<'_> {
    let mut scenarios = Vec::new();
    let mut tools = BTreeMap::new();
    for &scenario in &trials.declared.scenarios {
        let mut cells = Vec::new();
        for cell in CELLS.iter().filter(|cell| cell.scenario == scenario) {
            let status = match cell.status {
                Status::NotSupported(reason) => CellStatus::NotSupported { reason },
                Status::Pending(ticket) => CellStatus::NotMeasured {
                    reason: format!("pending {ticket}"),
                },
                Status::Measured(_) if scenario.reading() == Reading::Rules => {
                    rules::status(trials, cell)
                }
                Status::Measured(_) if scenario.reading() == Reading::Manifest => {
                    auto::status(trials, cell)
                }
                Status::Measured(_) if let Some(reason) = stopped_reason(trials, cell) => {
                    CellStatus::NotMeasured { reason }
                }
                Status::Measured(_) => {
                    let ran = trials_of(&trials.trials, cell);
                    let metrics = metrics(
                        cell,
                        &ran,
                        &runs_of(&trials.syscalls, cell),
                        &growth_of(&trials.growth, cell),
                    );
                    if scenario == Scenario::Raw && cell.row == Row::Main {
                        let peak_bytes =
                            median_bytes(ran.iter().map(|trial| peak(trial)).collect());
                        let rates: Vec<f64> = ran.iter().map(|trial| rate(trial)).collect();
                        tools.insert(
                            cell.tool.name(),
                            Figures {
                                pages_per_second: median(&rates),
                                peak_bytes,
                            },
                        );
                    }
                    let curve = (0..10)
                        .map(|part| {
                            let values: Vec<f64> = ran
                                .iter()
                                .map(|trial| {
                                    trial.measured.curve.get(part).copied().unwrap_or(f64::NAN)
                                })
                                .collect();
                            spread(&values, Unit::PagesPerSecond)
                        })
                        .collect();
                    CellStatus::Measured {
                        trials: ran.len(),
                        pages: ran.first().map_or(0, |trial| trial.measured.work.pages),
                        site_pages: ran.first().map_or(0, |trial| trial.site_pages),
                        bytes: ran.first().map_or(0, |trial| {
                            if scenario.resumes() {
                                trial.measured.work.first_bytes
                            } else {
                                trial.measured.work.bytes
                            }
                        }),
                        metrics,
                        curve,
                    }
                }
            };
            let output = output_of(trials, cell, &status);
            cells.push(CellReport {
                cell: cell.name,
                tool: cell.tool.name(),
                row: cell.row,
                mapping: cell.mapping,
                pending: cell.pending,
                one_trial: cell.one_trial(),
                output,
                status,
            });
        }
        scenarios.push(ScenarioReport {
            scenario,
            title: scenario.title(),
            standings: standing::of(scenario, &cells),
            cells,
        });
    }
    Report {
        label: "local-controlled",
        published: false,
        machine: &trials.machine,
        commit: &trials.commit,
        declared: &trials.declared,
        enough_trials: trials.declared.trials >= TRIALS,
        order_seed: trials.order_seed,
        scenarios,
        tools,
        instructions: instructions(&trials.instructions),
        sizes: sizes(&trials.sizes),
    }
}

const MIB: f64 = 1024.0 * 1024.0;
const GIB: f64 = 1024.0 * MIB;

/// A value in its unit.
fn shown(value: f64, unit: Unit) -> String {
    match unit {
        Unit::Count if value.fract() == 0.0 => format!("{value:.0}"),
        Unit::PagesPerSecond | Unit::Count => format!("{value:.1}"),
        Unit::Seconds if value < 1.0 => format!("{:.1} ms", value * 1000.0),
        Unit::Seconds => format!("{value:.3} s"),
        Unit::Bytes if value >= MIB => format!("{:.1} MiB", value / MIB),
        Unit::Bytes if value >= 1024.0 => format!("{:.1} KiB", value / 1024.0),
        Unit::Bytes => format!("{value:.0} B"),
        Unit::Cores => format!("{value:.2}"),
        Unit::Percent => format!("{value:.1} %"),
        Unit::Share => format!("{:.1} %", value * 100.0),
    }
}

pub(super) fn cell_text(figure: &Figure) -> String {
    match figure {
        Figure::Measured {
            median,
            low,
            high,
            unit,
        } => format!(
            "{} ({}–{})",
            shown(*median, *unit),
            shown(*low, *unit),
            shown(*high, *unit)
        ),
        Figure::NotMeasured { reason } => format!("not measured: {reason}"),
    }
}

/// What the output check found of a measured cell's pages, on the article
/// site: every page read right under the cell's own check, or its label
/// with the first difference; and that no check read them, on a site
/// whose right answers the Proving ground does not serve.
fn output_line(text: &mut String, scenario: Scenario, cell: &CellReport) {
    let CellStatus::Measured { site_pages, .. } = &cell.status else {
        return;
    };
    let Some(check) = matrix::cell(scenario, cell.cell).and_then(Cell::output_check) else {
        return;
    };
    let Some(output) = &cell.output else {
        let _ = writeln!(
            text,
            "  - output not checked: the Proving ground serves the right answers of sites of up to {} pages.",
            super::extracted::ANSWERED_PAGES
        );
        return;
    };
    let allowed = match check {
        OutputCheck::Whole => "",
        OutputCheck::AdAllowed => ", the ad allowed",
    };
    let Some(label) = output_differs(output, check, *site_pages) else {
        let _ = writeln!(
            text,
            "  - output: every page's title and Main content read right{allowed}."
        );
        return;
    };
    let _ = write!(
        text,
        "  - {label}{allowed}; the first: {}.",
        differing(output, check)
            .first
            .as_deref()
            .unwrap_or("not named")
    );
    if check == OutputCheck::Whole {
        let _ = write!(
            text,
            " With the ad allowed: {} of {site_pages}.",
            output.ad_allowed.pages
        );
    }
    text.push('\n');
}

/// A cell's column heading in a scenario's tables: its name, labelled when
/// it is a sensitivity row and when it ran one trial.
fn column(cell: &CellReport) -> String {
    let name = match cell.row {
        Row::Main => cell.cell.to_owned(),
        Row::Sensitivity => format!("{} (sensitivity)", cell.cell),
    };
    match cell.one_trial {
        Some(_) => format!("{name} (one trial)"),
        None => name,
    }
}

/// The metrics in the report's order, each with its group and label. A
/// metric where more is better is also named in `standing`'s
/// `MORE_IS_BETTER`; the standings read every other as a cost.
const ROWS: &[(&str, &str)] = &[
    ("pages_per_second", "Throughput: pages per second"),
    ("first_page_seconds", "Throughput: time to the first page"),
    (
        "first_record_seconds",
        "Throughput: time to the first Record",
    ),
    ("latency_p50_seconds", "Latency per page: p50"),
    ("latency_p95_seconds", "Latency per page: p95"),
    ("latency_p99_seconds", "Latency per page: p99"),
    ("user_seconds", "CPU: user time"),
    ("system_seconds", "CPU: system time"),
    ("cpu_seconds_per_1000_pages", "CPU: seconds per 1,000 pages"),
    ("average_cores", "CPU: average cores"),
    ("peak_cores", "CPU: peak cores"),
    ("peak_rss_bytes", "Memory: peak RSS of the process tree"),
    ("peak_pss_bytes", "Memory: peak PSS of the process tree"),
    ("average_rss_bytes", "Memory: time-weighted mean RSS"),
    (
        "rss_per_1000_pages_at_1000",
        "Memory: RSS per 1,000 pages at 1,000 pages",
    ),
    (
        "rss_per_1000_pages_at_10000",
        "Memory: RSS per 1,000 pages at 10,000 pages",
    ),
    (
        "rss_per_1000_pages_at_100000",
        "Memory: RSS per 1,000 pages at 100,000 pages",
    ),
    ("read_bytes", "Storage: bytes read"),
    ("written_bytes", "Storage: bytes written"),
    ("syncs", "Storage: fsync count"),
    (
        "footprint_bytes_per_1000_pages",
        "Storage: on-disk footprint per 1,000 pages",
    ),
    ("sent_bytes", "Network: bytes sent"),
    ("received_bytes", "Network: bytes received"),
    ("requests_sent", "Network: requests sent"),
    ("tcp_connections", "Network: TCP connections"),
    ("tls_connections", "Network: TLS connections"),
    ("peak_threads", "Process: threads at peak"),
    ("peak_open_files", "Process: open files at peak"),
    ("startup_seconds", "Process: startup to the first request"),
    ("executable_bytes", "Process: executable or install size"),
    ("processes", "Process: processes started"),
    ("system_calls_per_page", "Process: system calls per page"),
    ("pages", "Correctness: pages fetched"),
    ("duplicates", "Correctness: duplicates"),
    ("errors", "Correctness: errors"),
    ("skipped", "Correctness: skipped pages"),
    (
        "sample_share",
        "Tiers: share of the pages that were samples",
    ),
];

/// The report as Markdown, for the owner's reading of the pass (ADR 0134).
#[expect(
    clippy::too_many_lines,
    reason = "the report's sections, in their order"
)]
pub(crate) fn markdown(report: &Report<'_>, trials: &Comparison) -> String {
    let machine = report.machine;
    let declared = report.declared;
    let load = machine.load.map_or_else(String::new, |load| {
        format!(", 1-minute load {load:.2} at the start")
    });
    let name = machine
        .name
        .as_deref()
        .unwrap_or("a machine not named (`--machine`)");
    let memory = machine.memory_bytes.map_or_else(
        || "memory not read".to_owned(),
        |bytes| format!("{:.1} GiB of memory", float(bytes) / GIB),
    );
    let mut text = format!(
        "# Comparison matrix\n\n\
         `local-controlled`, measured on {name}: {} {}, {}, {} threads, {memory}{load}, at commit `{}`. \
         Compare its figures within this report, never with a report measured on another machine.\n\n\
         Rerun it with `{}` at that commit.\n\n\
         Declared before the first trial: {} trials of each measured cell, interleaved in random order \
         (seed {}), each tool started in a fresh process against a fresh Proving ground, {} requests at \
         once, from `{}`, unless a scenario's title names other requests in flight or another \
         site; Spider {}, Crawlee {} (`@crawlee/cheerio`, and `@crawlee/playwright` on Playwright \
         {}) on Node.js, Scrapy {} (and scrapy-playwright {} on Playwright {}) on CPython, \
         crawlee-rs at commit `{}`, pagesift {}, OpenSpider at commit `{}`, fastCRW {} and crawlberg {}, \
         every tool of the headless browser scenario \
         running Chrome for Testing {}; each \
         figure the {}. Every figure is measured from outside the tools: \
         the OS's accounting of the waited-for children (CPU, peak RSS, bytes read and written), `/proc` \
         samples of the process tree every 50 ms, strace's counts, the Proving ground's observations and \
         the data folder left. Every cell passed the same-work check. It gives no false-success \
         count: it reads no tool's own report, only what the Proving ground saw each tool fetch.\n",
        machine.os,
        machine.arch,
        machine.cpu,
        machine.threads,
        report.commit,
        declared.command,
        declared.trials,
        report.order_seed,
        declared.concurrency,
        declared.site,
        declared.spider,
        declared.crawlee,
        declared.playwright,
        declared.scrapy,
        declared.scrapy_playwright,
        declared.playwright,
        declared.crawlee_rs,
        declared.pagesift,
        declared.openspider,
        declared.fastcrw,
        declared.crawlberg,
        declared.browser,
        declared.statistic,
    );
    if !report.enough_trials {
        let _ = writeln!(
            text,
            "\nOnly {} trials of each measured cell, short of the {TRIALS} the matrix asks: a quicker try, not the comparison's figures.",
            declared.trials
        );
    }
    for scenario in &report.scenarios {
        let _ = writeln!(text, "\n## {}\n\nThe mapping:\n", scenario.title);
        for cell in &scenario.cells {
            let _ = writeln!(
                text,
                "- **{}**{}: {}.",
                cell.cell,
                match cell.row {
                    Row::Main => "",
                    Row::Sensitivity => " (sensitivity row)",
                },
                cell.mapping
            );
            for pending in cell.pending {
                let _ = writeln!(text, "  - pending: {pending}.");
            }
            if let Some(why) = cell.one_trial {
                let _ = writeln!(text, "  - one trial: {why}.");
            }
            if let Some(issue) = scenario.scenario.no_bar() {
                let _ = writeln!(
                    text,
                    "  - no bar: none is set on this cell until the owner sets one ({issue})."
                );
            }
            output_line(&mut text, scenario.scenario, cell);
            match &cell.status {
                CellStatus::NotSupported { reason } => {
                    let _ = writeln!(text, "  - not supported: {reason}.");
                }
                CellStatus::NotMeasured { reason } => {
                    let _ = writeln!(text, "  - not measured: {reason}.");
                }
                CellStatus::Measured {
                    trials,
                    pages,
                    bytes,
                    metrics,
                    ..
                } => {
                    // Over TLS a trial's work counts its answers' bodies alone
                    // (`trials::Work::bytes`).
                    let _ = writeln!(
                        text,
                        "  - same work: {trials} trial{}, each {pages} pages served in {bytes} {}bytes.",
                        if *trials == 1 { "" } else { "s" },
                        if scenario.scenario.network().tls() {
                            "body "
                        } else {
                            ""
                        }
                    );
                    published::cpu(&mut text, cell.tool, metrics);
                    published::spider_buffer(&mut text, cell.tool, scenario.scenario);
                }
                CellStatus::Ruled { runs, tasks } => rules::markdown(&mut text, *runs, tasks),
                CellStatus::Auto {
                    runs,
                    pages,
                    rate,
                    rendered,
                    wrong,
                } => auto::markdown(&mut text, *runs, *pages, (rate, rendered, wrong)),
            }
        }
        let measured: Vec<_> = scenario
            .cells
            .iter()
            .filter_map(|cell| match &cell.status {
                CellStatus::Measured {
                    pages,
                    site_pages,
                    metrics,
                    curve,
                    ..
                } => Some((
                    format!(
                        "{}, {pages} of the site's {site_pages} pages{}",
                        column(cell),
                        own_output_differs(cell, scenario.scenario)
                            .map_or_else(String::new, |label| format!(", {label}"))
                    ),
                    metrics,
                    curve,
                )),
                CellStatus::NotSupported { .. }
                | CellStatus::NotMeasured { .. }
                | CellStatus::Ruled { .. }
                | CellStatus::Auto { .. } => None,
            })
            .collect();
        if measured.is_empty() {
            continue;
        }
        let _ = write!(text, "\n| Metric |");
        for (cell, _, _) in &measured {
            let _ = write!(text, " {cell} |");
        }
        let _ = write!(text, "\n|---|");
        for _ in &measured {
            text.push_str("---:|");
        }
        text.push('\n');
        for (name, label) in ROWS {
            let _ = write!(text, "| {label} |");
            for (_, metrics, _) in &measured {
                let figure = metrics.get(name).map_or_else(String::new, cell_text);
                let _ = write!(text, " {figure} |");
            }
            text.push('\n');
        }
        let _ = write!(
            text,
            "\nPages per second over each tenth of the crawl:\n\n| Tenth |"
        );
        for (cell, _, _) in &measured {
            let _ = write!(text, " {cell} |");
        }
        let _ = write!(text, "\n|---:|");
        for _ in &measured {
            text.push_str("---:|");
        }
        text.push('\n');
        for part in 0..10 {
            let _ = write!(text, "| {} |", part + 1);
            for (_, _, curve) in &measured {
                let figure = curve.get(part).map_or_else(String::new, cell_text);
                let _ = write!(text, " {figure} |");
            }
            text.push('\n');
        }
        standing::markdown(&mut text, scenario);
    }
    for row in &RATIOS {
        let ratio = match &report.instructions {
            Instructions::NotRun { reason } => Err(*reason),
            Instructions::Measured { ratios } => ratios
                .iter()
                .find(|ratio| ratio.row == row.row)
                .ok_or("the run asked for no Callgrind cell of its scenario"),
        };
        match ratio {
            Err(reason) => {
                let _ = writeln!(text, "\n`{}`: not run: {reason}.", row.row);
            }
            Ok(InstructionRatio {
                row,
                scenario,
                foxhound_per_page,
                spider_per_page,
                ratio,
                spider_output_differs,
                ..
            }) => {
                let _ = write!(
                    text,
                    "\n`{row}`: {ratio:.3}, Foxhound's {foxhound_per_page:.0} instructions per page over Spider's {spider_per_page:.0}, under Callgrind on {scenario}, the median of each tool's runs, all of the same pages and bytes."
                );
                if let Some(label) = spider_output_differs {
                    let _ = write!(
                        text,
                        " Spider's output differs under this row's check ({label}), so the ratio is of unlike work and no parity Contract reads it."
                    );
                }
                text.push('\n');
            }
        }
    }
    sizes_markdown(&mut text, &report.sizes);
    text.push_str("\n## Trials, in the order run\n\n");
    let unquiet = trials
        .trials
        .iter()
        .filter(|trial| trial.load.is_some_and(|load| load >= QUIET_LOAD))
        .count();
    if unquiet > 0 {
        let _ = writeln!(
            text,
            "{unquiet} of the {} timed trials started above a 1-minute load of {QUIET_LOAD}.\n",
            trials.trials.len()
        );
    }
    if let CountedCell::Measured { runs, .. } = &trials.syscalls {
        let unquiet = runs
            .iter()
            .filter(|run| run.load.is_some_and(|load| load >= STRACE_QUIET_LOAD))
            .count();
        if unquiet > 0 {
            let _ = writeln!(
                text,
                "{unquiet} of the {} runs under strace started above a 1-minute load of {STRACE_QUIET_LOAD}.\n",
                runs.len()
            );
        }
    }
    for redone in &trials.redone {
        let _ = writeln!(
            text,
            "Measured again by a later run at the same commit on the same machine, its trials in their own order (seed {}): {}.\n",
            redone.order_seed,
            redone.cells.join(", ")
        );
    }
    text.push_str(
        "| Scenario | Cell | Trial | Load at start | Seconds | Pages per second | Pages fetched | Peak RSS |\n|---|---|---:|---:|---:|---:|---:|---:|\n",
    );
    for trial in &trials.trials {
        let _ = writeln!(
            text,
            "| {} | {} | {} | {} | {:.3} | {:.1} | {} of {} | {:.1} MiB |",
            trial.scenario.name(),
            trial.cell,
            trial.retried.as_ref().map_or_else(
                || trial.number.to_string(),
                |why| format!("{} (retried: {why})", trial.number)
            ),
            trial
                .load
                .map_or_else(|| "not measured".to_owned(), |load| format!("{load:.2}")),
            trial.measured.seconds,
            rate(trial),
            trial.measured.work.pages,
            trial.site_pages,
            float(peak(trial)) / MIB
        );
    }
    text
}
