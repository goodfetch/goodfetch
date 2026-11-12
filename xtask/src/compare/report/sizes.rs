//! The sizes cell's part of the report: each tool's peak memory at each
//! size and concurrency, with the bar at [`CONCURRENCY`] in flight
//! (spec #2165).

use std::collections::BTreeMap;
use std::fmt::Write as _;

use serde::Serialize;

use super::super::trials::{
    CONCURRENCY, SIZE_CONCURRENCIES, SIZE_TOOLS, SIZES, SizesCell, Tool, Trial,
};
use super::{Unit, float, median_bytes, peak, shown};

/// One tool's peak memory at one size and concurrency of the sizes cell,
/// each the median of its trials.
#[derive(Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub(crate) enum SizeFigures {
    Measured {
        trials: usize,
        /// The process tree's peak RSS ([`super::peak`]).
        peak_bytes: u64,
        /// The tree's peak resident memory no file backs (`RssAnon`), where
        /// every trial sampled it.
        private_bytes: Option<u64>,
        /// The peak of the tool's executable's own resident pages, where
        /// every trial read them.
        executable_bytes: Option<u64>,
    },
    /// Its trial at this point was stopped twice.
    NotMeasured { reason: String },
}

/// The sizes cell at one size and concurrency.
#[derive(Serialize)]
pub(crate) struct SizePoint {
    pub(crate) site_pages: u64,
    pub(crate) concurrency: u32,
    /// Each tool's figures, by its name, which `just targets` reads.
    pub(crate) tools: BTreeMap<&'static str, SizeFigures>,
    /// At [`CONCURRENCY`] in flight, whether Foxhound's peak RSS is at most
    /// Spider's (spec #2165); none at another concurrency, or when either
    /// is not measured.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) bar_met: Option<bool>,
}

/// The sizes cell's report.
#[derive(Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub(crate) enum Sizes<'a> {
    NotRun { reason: &'a str },
    Measured { points: Vec<SizePoint> },
}

/// Each tool's figures over `trials`, its trials at one size and
/// concurrency, or why it has none.
fn size_figures(trials: &[&Trial]) -> SizeFigures {
    // Each figure only where every trial gave it.
    let every = |figure: fn(&Trial) -> Option<u64>| {
        trials
            .iter()
            .map(|trial| figure(trial))
            .collect::<Option<Vec<u64>>>()
            .map(median_bytes)
    };
    SizeFigures::Measured {
        trials: trials.len(),
        peak_bytes: median_bytes(trials.iter().map(|trial| peak(trial)).collect()),
        private_bytes: every(|trial| {
            trial
                .measured
                .sampled
                .as_ref()
                .map(|sampled| sampled.peak_private_bytes)
        }),
        executable_bytes: every(|trial| {
            trial
                .measured
                .sampled
                .as_ref()
                .and_then(|sampled| sampled.peak_executable_bytes)
        }),
    }
}

/// The sizes cell's report: each size and concurrency, Foxhound's and
/// Spider's figures, and at [`CONCURRENCY`] in flight the bar.
pub(super) fn sizes(cell: &SizesCell) -> Sizes<'_> {
    let (trials_run, stopped) = match cell {
        SizesCell::NotRun { reason } => return Sizes::NotRun { reason },
        SizesCell::Measured {
            trials_run,
            stopped,
            ..
        } => (trials_run, stopped),
    };
    let mut points = Vec::new();
    for concurrency in SIZE_CONCURRENCIES {
        for site_pages in SIZES {
            let at = |pages, at_concurrency| pages == site_pages && at_concurrency == concurrency;
            let mut tools = BTreeMap::new();
            for tool in SIZE_TOOLS {
                let ran: Vec<&Trial> = trials_run
                    .iter()
                    .filter(|trial| trial.tool == tool && at(trial.site_pages, trial.concurrency))
                    .collect();
                let stop = stopped.iter().find(|point| {
                    point.run.tool == tool && at(point.site_pages, point.concurrency)
                });
                let figures = match stop {
                    Some(point) => SizeFigures::NotMeasured {
                        reason: point.run.reason.clone(),
                    },
                    None if ran.is_empty() => continue,
                    None => size_figures(&ran),
                };
                tools.insert(tool.name(), figures);
            }
            let peak_of = |tool: Tool| match tools.get(tool.name()) {
                Some(SizeFigures::Measured { peak_bytes, .. }) => Some(*peak_bytes),
                Some(SizeFigures::NotMeasured { .. }) | None => None,
            };
            let bar_met = (concurrency == CONCURRENCY)
                .then(|| Some(peak_of(Tool::Foxhound)? <= peak_of(Tool::Spider)?))
                .flatten();
            points.push(SizePoint {
                site_pages,
                concurrency,
                tools,
                bar_met,
            });
        }
    }
    Sizes::Measured { points }
}

/// A size's figure in MiB, or what it lacks.
fn mib(bytes: Option<u64>) -> String {
    bytes.map_or_else(
        || "not measured".to_owned(),
        |bytes| shown(float(bytes), Unit::Bytes),
    )
}

/// The sizes cell's section: at [`CONCURRENCY`] in flight first, with the
/// bar, then at each other concurrency, without one.
pub(super) fn sizes_markdown(text: &mut String, sizes: &Sizes<'_>) {
    text.push_str("\n## Peak memory at three sizes\n\n");
    let points = match sizes {
        Sizes::NotRun { reason } => {
            let _ = writeln!(text, "Not run: {reason}.");
            return;
        }
        Sizes::Measured { points } => points,
    };
    let _ = writeln!(
        text,
        "Scenario 1's main rows of Foxhound and Spider, crawling the site of each size the table lists \
         at each number of requests in flight it lists, each trial started once every executable it runs \
         was dropped from the page cache, so it pages them in as a first start does (spec #2165). The \
         bar, at {CONCURRENCY} in flight alone: Foxhound's peak RSS at most Spider's. Private memory is \
         the tree's resident memory no file backs; the executable's pages, the resident pages of the \
         tool's own executable file. Each figure is the median of the tool's trials.\n"
    );
    text.push_str(
        "| Pages | In flight | Tool | Trials | Peak RSS | Private | Executable's pages | Bar |\n\
         |---:|---:|---|---:|---:|---:|---:|---|\n",
    );
    for point in points {
        for (tool, figures) in &point.tools {
            let bar = match point.bar_met {
                Some(met) if *tool == Tool::Foxhound.name() => {
                    if met {
                        "met"
                    } else {
                        "missed"
                    }
                }
                _ => "",
            };
            let (trials, peak, private, executable) = match figures {
                SizeFigures::Measured {
                    trials,
                    peak_bytes,
                    private_bytes,
                    executable_bytes,
                } => (
                    trials.to_string(),
                    mib(Some(*peak_bytes)),
                    mib(*private_bytes),
                    mib(*executable_bytes),
                ),
                SizeFigures::NotMeasured { reason } => (
                    format!("not measured: {reason}"),
                    String::new(),
                    String::new(),
                    String::new(),
                ),
            };
            let _ = writeln!(
                text,
                "| {} | {} | {tool} | {trials} | {peak} | {private} | {executable} | {bar} |",
                point.site_pages, point.concurrency
            );
        }
    }
}
