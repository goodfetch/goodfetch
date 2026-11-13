//! The hostile pages' cells in the report (#2439): for each task of a
//! cell's pass, each count of the rules its tool broke, over the cell's
//! runs, beside its coverage and, where the family records it, its waste.
//! They carry no bar until the owner sets one (#2439, Q3), and give no
//! standing.

use std::fmt::Write as _;

use serde::Serialize;

use super::{CellStatus, Figure, Unit, cell_text, float, not, shown, spread};
use crate::compare::hostile::Broken;
use crate::compare::matrix::Cell;
use crate::compare::trials::Comparison;

/// One task's counts over a cell's runs.
#[derive(Serialize)]
pub(crate) struct RuledTask {
    pub(crate) task: String,
    pub(crate) requests: Figure,
    pub(crate) retry_after_ignored: Figure,
    pub(crate) over_in_flight_cap: Figure,
    pub(crate) honeypot: Figure,
    /// The runs whose address the family's rule had flagged by its end.
    pub(crate) flagged: usize,
    /// The task's items fetched over its items, as a percentage.
    pub(crate) coverage: Figure,
    pub(crate) wanted: u64,
    /// Duplicates and trap requests over every request, as a percentage,
    /// where the family records them.
    pub(crate) waste: Option<Figure>,
    /// Each run the comparison stopped before its tool ended its crawl:
    /// its number and why.
    pub(crate) stopped: Vec<(u32, String)>,
}

/// `part` of `whole`, as a percentage; not a number when `whole` is none.
fn percent(part: u64, whole: u64) -> f64 {
    if whole == 0 {
        return f64::NAN;
    }
    float(part) * 100.0 / float(whole)
}

/// The status of the hostile pages' `cell`: its runs' counts, task by task
/// in its pass's order.
pub(super) fn status(trials: &Comparison, cell: &Cell) -> CellStatus {
    let runs: Vec<_> = trials
        .rules
        .iter()
        .filter(|run| run.scenario == cell.scenario && run.cell == cell.name)
        .collect();
    if runs.is_empty() {
        return CellStatus::NotMeasured {
            reason: "no run of it is in the trials file".to_owned(),
        };
    }
    let tasks = cell
        .scenario
        .hostile_tasks()
        .iter()
        .map(|task| {
            let counts: Vec<(u32, &Broken)> = runs
                .iter()
                .filter_map(|run| {
                    let counts = run.tasks.iter().find(|counts| counts.task == task.task)?;
                    Some((run.number, counts))
                })
                .collect();
            let each = |count: fn(&Broken) -> f64, unit: Unit| {
                if counts.len() < runs.len() {
                    return not("a run gave no count of this task");
                }
                let values: Vec<f64> = counts.iter().map(|(_, counts)| count(counts)).collect();
                spread(&values, unit)
            };
            RuledTask {
                task: task.task.to_owned(),
                requests: each(|counts| float(counts.requests), Unit::Count),
                retry_after_ignored: each(|counts| float(counts.retry_after_ignored), Unit::Count),
                over_in_flight_cap: each(|counts| float(counts.over_in_flight_cap), Unit::Count),
                honeypot: each(|counts| float(counts.honeypot), Unit::Count),
                flagged: counts.iter().filter(|(_, counts)| counts.flagged).count(),
                coverage: each(
                    |counts| percent(counts.wanted_fetched, counts.wanted),
                    Unit::Percent,
                ),
                wanted: task.wanted,
                waste: task.records_waste.then(|| {
                    each(
                        |counts| percent(counts.duplicate + counts.trap, counts.requests),
                        Unit::Percent,
                    )
                }),
                stopped: counts
                    .iter()
                    .filter_map(|(number, counts)| Some((*number, counts.stopped.clone()?)))
                    .collect(),
            }
        })
        .collect();
    CellStatus::Ruled {
        runs: runs.len(),
        tasks,
    }
}

/// `figure` after `label`: its median, then its range.
fn counted(label: &str, figure: &Figure) -> String {
    match figure {
        Figure::Measured {
            median,
            low,
            high,
            unit,
        } => format!(
            "{label} {} ({}–{})",
            shown(*median, *unit),
            shown(*low, *unit),
            shown(*high, *unit)
        ),
        Figure::NotMeasured { reason } => format!("{label} not measured: {reason}"),
    }
}

/// A hostile pages' cell's lines in its scenario's mapping.
pub(super) fn markdown(text: &mut String, runs: usize, tasks: &[RuledTask]) {
    let _ = writeln!(
        text,
        "  - rules broken over {runs} runs, each figure the median (lowest–highest) of its runs:"
    );
    for task in tasks {
        // Every count of a task is measured, or none is.
        let (median, low, high) = match &task.requests {
            Figure::Measured {
                median, low, high, ..
            } => (median, low, high),
            Figure::NotMeasured { reason } => {
                let _ = writeln!(text, "    - `{}`: not measured: {reason}.", task.task);
                continue;
            }
        };
        let _ = write!(
            text,
            "    - `{}`: {} requests ({}–{}), {}, {}, {}; the address ended flagged in {} of \
             {runs} runs; coverage {} of its {} wanted pages",
            task.task,
            shown(*median, Unit::Count),
            shown(*low, Unit::Count),
            shown(*high, Unit::Count),
            counted("`retry_after_ignored`", &task.retry_after_ignored),
            counted("`over_in_flight_cap`", &task.over_in_flight_cap),
            counted("`honeypot`", &task.honeypot),
            task.flagged,
            cell_text(&task.coverage),
            task.wanted,
        );
        if let Some(waste) = &task.waste {
            let _ = write!(
                text,
                "; waste {} of its requests, duplicates and trap requests",
                cell_text(waste)
            );
        }
        if !task.stopped.is_empty() {
            let stopped: Vec<String> = task
                .stopped
                .iter()
                .map(|(number, reason)| format!("run {number}, {reason}"))
                .collect();
            let _ = write!(
                text,
                "; stopped in {} of {runs} runs: {}",
                task.stopped.len(),
                stopped.join("; ")
            );
        }
        text.push_str(".\n");
    }
}

#[cfg(test)]
mod tests;
