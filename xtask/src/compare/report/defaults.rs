//! The default costs' part of the report (#2438): the instructions each
//! Foxhound default adds per page to scenario 1's first cell, each the
//! median of the Callgrind cell's runs of the first cell with that one
//! default on, minus the first cell's median. No line has a limit of its
//! own: the defaults' total is held by the at-defaults row,
//! `foxhound.spider_instruction_ratio_at_defaults`.

use std::fmt::Write as _;

use serde::Serialize;

use super::super::matrix::{DEFAULT_COST_MAPPING, DEFAULT_COSTS, DEFAULT_COSTS_BASE, Scenario};
use super::super::trials::{CountedRun, INSTRUCTION_RUNS};
use super::{counted, float, per_page};

/// The thousandths of a count read under Valgrind that it may vary by,
/// 0.5 %: the Quality register's tolerance for its instruction rows,
/// which `perf`'s `with_valgrind_tolerance` applies to `xtask counts`.
const VALGRIND_TOLERANCE_THOUSANDTHS: u64 = 5;

/// The instructions each default adds, read against [`DEFAULT_COSTS_BASE`].
#[derive(Serialize)]
pub(crate) struct DefaultCosts {
    /// The cell each line is read against: scenario 1's first cell.
    pub(crate) cell: &'static str,
    /// Its median instructions per page, where the Callgrind cell counted
    /// it.
    pub(crate) per_page: Option<f64>,
    /// One line per default, in [`DEFAULT_COSTS`]' order.
    pub(crate) lines: Vec<DefaultCostLine>,
}

/// One default's line.
#[derive(Serialize)]
pub(crate) struct DefaultCostLine {
    pub(crate) default: &'static str,
    /// The default-cost cell that counted it.
    pub(crate) cell: &'static str,
    /// The setting the cell turns on, as the report writes it.
    pub(crate) setting: String,
    #[serde(flatten)]
    pub(crate) figure: DefaultCostFigure,
}

/// What one default's line gives.
#[derive(Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub(crate) enum DefaultCostFigure {
    Measured {
        /// The cell's median instructions per page.
        per_page: f64,
        /// `per_page` less the first cell's.
        added: f64,
        /// Whether `added` is under 0.5 % of the first cell's count, the
        /// variation a figure read under Valgrind may show.
        within_tolerance: bool,
    },
    NotRun {
        reason: &'static str,
    },
}

/// Each default's line over the Callgrind cell's `runs`.
pub(super) fn default_costs(runs: &[CountedRun]) -> DefaultCosts {
    let base = counted(runs, Scenario::Raw, DEFAULT_COSTS_BASE);
    let base = (!base.is_empty()).then(|| per_page(&base));
    DefaultCosts {
        cell: DEFAULT_COSTS_BASE,
        per_page: base,
        lines: DEFAULT_COSTS
            .iter()
            .map(|cost| {
                let own = counted(runs, Scenario::Raw, cost.cell.name);
                let figure = match (base, own.is_empty()) {
                    (_, true) => DefaultCostFigure::NotRun {
                        reason: "the run asked for no run of this cell",
                    },
                    (None, false) => DefaultCostFigure::NotRun {
                        reason: "the run counted no run of scenario 1's first cell",
                    },
                    (Some(base), false) => {
                        let per_page = per_page(&own);
                        let added = per_page - base;
                        DefaultCostFigure::Measured {
                            per_page,
                            added,
                            within_tolerance: added.abs() * 1_000.0
                                < base * float(VALGRIND_TOLERANCE_THOUSANDTHS),
                        }
                    }
                };
                DefaultCostLine {
                    default: cost.default,
                    cell: cost.cell.name,
                    setting: cost.setting(),
                    figure,
                }
            })
            .collect(),
    }
}

/// The table of the instructions each default adds, or why the Callgrind
/// cell gave none: `costs` is `Err` with its reason when it did not run.
pub(super) fn default_costs_markdown(text: &mut String, costs: Result<&DefaultCosts, &str>) {
    text.push_str("\n## Instructions each Foxhound default adds\n\n");
    let costs = match costs {
        Err(reason) => {
            let _ = writeln!(text, "Not run: {reason}.");
            return;
        }
        Ok(costs) => costs,
    };
    let base = costs
        .per_page
        .map_or_else(|| "not run".to_owned(), |base| format!("{base:.0}"));
    let _ = writeln!(
        text,
        "Each line is the median instructions per page of {INSTRUCTION_RUNS} Callgrind runs of \
         {DEFAULT_COST_MAPPING}, minus the first cell's median, {base}; every run did the same \
         pages and bytes. No line has a limit of its own: the defaults' total is held by \
         `foxhound.spider_instruction_ratio_at_defaults`. A line under 0.5 % of the first cell's \
         count is within the counter's tolerance, the Quality register's for a figure read under \
         Valgrind (#2438).\n",
    );
    text.push_str(
        "| Default | Setting | Instructions per page | Added |\n\
         |---|---|---:|---:|\n",
    );
    for line in &costs.lines {
        let (per_page, added) = match &line.figure {
            DefaultCostFigure::NotRun { reason } => (format!("not run: {reason}"), String::new()),
            DefaultCostFigure::Measured {
                per_page,
                added,
                within_tolerance,
            } => {
                // `per_page - added` is the first cell's median, which a
                // measured line is read against.
                let share = added / (per_page - added) * 100.0;
                let tolerance = if *within_tolerance {
                    ", within the counter's tolerance"
                } else {
                    ""
                };
                (
                    format!("{per_page:.0}"),
                    format!("{added:+.0} ({share:+.2} %{tolerance})"),
                )
            }
        };
        let _ = writeln!(
            text,
            "| {} | {} | {per_page} | {added} |",
            line.default, line.setting
        );
    }
}
