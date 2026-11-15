//! The auto cells in the report (#2411): over a cell's runs on the rungs
//! site, its rate, the pages it rendered and the pages whose Records
//! differ from the site's manifest, beside the same work its runs did.
//! They carry no bar (G-04 Q11) and give no standing.

use std::fmt::Write as _;

use super::{CellStatus, Figure, Unit, cell_text, float, spread};
use crate::compare::auto::Judged;
use crate::compare::matrix::Cell;
use crate::compare::trials::Comparison;

/// The status of the auto `cell`: its runs' figures.
pub(super) fn status(trials: &Comparison, cell: &Cell) -> CellStatus {
    let runs: Vec<_> = trials
        .auto
        .iter()
        .filter(|run| run.scenario == cell.scenario && run.cell == cell.name)
        .map(|run| &run.judged)
        .collect();
    let Some(first) = runs.first() else {
        return CellStatus::NotMeasured {
            reason: "no run of it is in the trials file".to_owned(),
        };
    };
    let each = |value: fn(&Judged) -> f64, unit: Unit| {
        let values: Vec<f64> = runs.iter().map(|judged| value(judged)).collect();
        spread(&values, unit)
    };
    CellStatus::Auto {
        runs: runs.len(),
        pages: first.pages,
        rate: each(Judged::rate, Unit::PagesPerSecond),
        rendered: each(|judged| float(judged.rendered), Unit::Count),
        wrong: each(|judged| float(judged.wrong), Unit::Count),
    }
}

/// An auto cell's lines in its scenario's mapping.
pub(super) fn markdown(
    text: &mut String,
    runs: usize,
    pages: u64,
    (rate, rendered, wrong): (&Figure, &Figure, &Figure),
) {
    let _ = writeln!(
        text,
        "  - same work: {runs} run{}, each requested all {pages} pages of the rungs site's \
         manifest.",
        if runs == 1 { "" } else { "s" },
    );
    let _ = writeln!(
        text,
        "  - rate {} pages/s, pages rendered {} and pages wrong {} of the {pages}, each the \
         median (lowest–highest) of its runs.",
        cell_text(rate),
        cell_text(rendered),
        cell_text(wrong),
    );
}

#[cfg(test)]
mod tests;
