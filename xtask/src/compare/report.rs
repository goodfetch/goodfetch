//! The report: for each scenario, each cell's mapping and every metric of
//! the owner's list, each the median of the cell's trials with its spread
//! (the lowest and the highest trial), or why the cell or the metric gives
//! no figure; and `foxhound.spider_instruction_ratio` from the Callgrind
//! cell. Labelled `local-controlled` with the machine and never published
//! (ADR 0076), and written only once the same-work check passed.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use serde::Serialize;

use super::matrix::{CELLS, Cell, Row, Scenario, Status};
use super::sampler::{CORE_WINDOW_SAMPLES, INTERVAL_MILLISECONDS};
use super::trials::{
    Comparison, CountedCell, CountedRun, Declared, GrowthCell, LatencyEnd, MILESTONES, Machine,
    QUIET_LOAD, TRIALS, Trial,
};

/// The span of one window of the core sampling.
const CORE_WINDOW_MILLISECONDS: u64 = CORE_WINDOW_SAMPLES as u64 * INTERVAL_MILLISECONDS;

/// The Quality register's row the Callgrind cell gives (ADR 0140).
const RATIO_ROW: &str = "foxhound.spider_instruction_ratio";

/// A metric's unit, as the Markdown writes it.
#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Unit {
    PagesPerSecond,
    Seconds,
    Bytes,
    Cores,
    Count,
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
    #[serde(flatten)]
    pub(crate) status: CellStatus,
}

#[derive(Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub(crate) enum CellStatus {
    Measured {
        trials: usize,
        /// What the same-work check found every trial did.
        pages: u64,
        bytes: u64,
        metrics: BTreeMap<&'static str, Figure>,
        /// Pages per second in each tenth of the crawl.
        curve: Vec<Figure>,
    },
    NotSupported {
        reason: &'static str,
    },
    /// Its timed trial was stopped twice (see `run::stopped`).
    NotMeasured {
        reason: String,
    },
}

/// One scenario's report.
#[derive(Serialize)]
pub(crate) struct ScenarioReport {
    pub(crate) scenario: Scenario,
    pub(crate) title: &'static str,
    pub(crate) cells: Vec<CellReport>,
}

/// Scenario 1's figures of a tool, its main row's, which `just targets`
/// reads (ADR 0138: the Spider target is read on scenario 1).
#[derive(Serialize)]
pub(crate) struct Figures {
    pub(crate) pages_per_second: f64,
    pub(crate) peak_bytes: u64,
}

/// The Callgrind cell's figure.
#[derive(Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub(crate) enum Instructions<'a> {
    NotRun {
        reason: &'a str,
    },
    Measured {
        row: &'static str,
        scenario: &'static str,
        foxhound_per_page: f64,
        spider_per_page: f64,
        /// Foxhound's per page over Spider's, to three decimals, as the
        /// register's figures are.
        ratio: f64,
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
}

/// The median of `values`, the mean of the middle two for an even count.
fn median(values: &[f64]) -> f64 {
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
fn float(count: u64) -> f64 {
    count as f64
}

fn spread(values: &[f64], unit: Unit) -> Figure {
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
    metrics
}

/// The runs of `cell` in a counted cell, or why there are none: the cell
/// did not run, or a run of `cell` was stopped.
fn runs_of<'a>(counted: &'a CountedCell, cell: &Cell) -> Result<Vec<&'a CountedRun>, &'a str> {
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

fn growth_of<'a>(growth: &'a GrowthCell, cell: &Cell) -> Result<Vec<&'a Trial>, &'a str> {
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
            None => Ok(trials_run
                .iter()
                .filter(|trial| trial.scenario == cell.scenario && trial.cell == cell.name)
                .collect()),
        },
    }
}

/// `tool`'s median instructions per page over its scenario 1 runs.
fn per_page(runs: &[CountedRun], tool: super::trials::Tool) -> f64 {
    let counts: Vec<f64> = runs
        .iter()
        .filter(|run| run.tool == tool)
        .map(|run| float(run.count) / float(run.work.pages))
        .collect();
    median(&counts)
}

fn instructions(cell: &CountedCell) -> Instructions<'_> {
    match cell {
        CountedCell::NotRun { reason } => Instructions::NotRun { reason },
        CountedCell::Measured { runs, .. } => {
            let foxhound_per_page = per_page(runs, super::trials::Tool::Foxhound);
            let spider_per_page = per_page(runs, super::trials::Tool::Spider);
            Instructions::Measured {
                row: RATIO_ROW,
                scenario: super::trials::INSTRUCTION_SCENARIO,
                foxhound_per_page,
                spider_per_page,
                ratio: (foxhound_per_page / spider_per_page * 1000.0).round() / 1000.0,
            }
        }
    }
}

/// The report of trials that passed the same-work check.
/// Why `cell`'s timed trial was stopped twice, if it was.
fn stopped_reason(trials: &Comparison, cell: &Cell) -> Option<String> {
    trials
        .stopped
        .iter()
        .find(|run| run.scenario == cell.scenario && run.cell == cell.name)
        .map(|run| run.reason.clone())
}

pub(crate) fn of(trials: &Comparison) -> Report<'_> {
    let mut scenarios = Vec::new();
    let mut tools = BTreeMap::new();
    for &scenario in &trials.declared.scenarios {
        let mut cells = Vec::new();
        for cell in CELLS.iter().filter(|cell| cell.scenario == scenario) {
            let status = match cell.status {
                Status::NotSupported(reason) => CellStatus::NotSupported { reason },
                Status::Measured(_) if let Some(reason) = stopped_reason(trials, cell) => {
                    CellStatus::NotMeasured { reason }
                }
                Status::Measured(_) => {
                    let ran: Vec<&Trial> = trials
                        .trials
                        .iter()
                        .filter(|trial| trial.scenario == scenario && trial.cell == cell.name)
                        .collect();
                    let metrics = metrics(
                        cell,
                        &ran,
                        &runs_of(&trials.syscalls, cell),
                        &growth_of(&trials.growth, cell),
                    );
                    if scenario == Scenario::Raw && cell.row == Row::Main {
                        let mut peaks: Vec<u64> = ran.iter().map(|trial| peak(trial)).collect();
                        peaks.sort_unstable();
                        let middle = peaks.len() / 2;
                        let peak_bytes = match peaks.len() {
                            0 => 0,
                            // Whole bytes: the mean of the middle two,
                            // rounded down.
                            length if length % 2 == 0 => {
                                u64::midpoint(peaks[middle - 1], peaks[middle])
                            }
                            _ => peaks[middle],
                        };
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
            cells.push(CellReport {
                cell: cell.name,
                tool: cell.tool.name(),
                row: cell.row,
                mapping: cell.mapping,
                pending: cell.pending,
                status,
            });
        }
        scenarios.push(ScenarioReport {
            scenario,
            title: scenario.title(),
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
    }
}

const MIB: f64 = 1024.0 * 1024.0;

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
    }
}

fn cell_text(figure: &Figure) -> String {
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

/// A cell's column heading in a scenario's tables: its name, labelled when
/// it is a sensitivity row.
fn column(cell: &CellReport) -> String {
    match cell.row {
        Row::Main => cell.cell.to_owned(),
        Row::Sensitivity => format!("{} (sensitivity)", cell.cell),
    }
}

/// The metrics in the report's order, each with its group and label.
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
    let mut text = format!(
        "# Comparison matrix\n\n\
         `local-controlled`, never published (ADR 0076): {} {} on {}, {} threads{load}, at commit `{}`.\n\n\
         Declared before the first trial: {} trials of each measured cell, interleaved in random order \
         (seed {}), each tool started in a fresh process against a fresh Proving ground, {} requests at \
         once, from `{}`; Spider {}, Crawlee {} (`@crawlee/cheerio`, and `@crawlee/playwright` on \
         Playwright {}) on Node.js and Scrapy {} (and scrapy-playwright {} on Playwright {}) on \
         CPython, every tool of the headless browser scenario running Chrome for Testing {}; each \
         figure the {}. Every figure is measured from outside the tools: \
         the OS's accounting of the waited-for children (CPU, peak RSS, bytes read and written), `/proc` \
         samples of the process tree every 50 ms, strace's counts, the Proving ground's observations and \
         the data folder left. Every cell passed the same-work check.\n",
        machine.os,
        machine.arch,
        machine.cpu,
        machine.threads,
        report.commit,
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
                    ..
                } => {
                    let _ = writeln!(
                        text,
                        "  - same work: {trials} trials, each {pages} pages served in {bytes} bytes."
                    );
                }
            }
        }
        let measured: Vec<_> = scenario
            .cells
            .iter()
            .filter_map(|cell| match &cell.status {
                CellStatus::Measured { metrics, curve, .. } => Some((column(cell), metrics, curve)),
                CellStatus::NotSupported { .. } | CellStatus::NotMeasured { .. } => None,
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
    }
    match &report.instructions {
        Instructions::NotRun { reason } => {
            let _ = writeln!(text, "\n`{RATIO_ROW}`: not run: {reason}.");
        }
        Instructions::Measured {
            row,
            scenario,
            foxhound_per_page,
            spider_per_page,
            ratio,
        } => {
            let _ = writeln!(
                text,
                "\n`{row}`: {ratio:.3}, Foxhound's {foxhound_per_page:.0} instructions per page over Spider's {spider_per_page:.0}, under Callgrind on {scenario}, the median of each tool's runs, all of the same work."
            );
        }
    }
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
    text.push_str(
        "| Scenario | Cell | Trial | Load at start | Seconds | Pages per second | Peak RSS |\n|---|---|---:|---:|---:|---:|---:|\n",
    );
    for trial in &trials.trials {
        let _ = writeln!(
            text,
            "| {} | {} | {} | {} | {:.3} | {:.1} | {:.1} MiB |",
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
            float(peak(trial)) / MIB
        );
    }
    text
}
