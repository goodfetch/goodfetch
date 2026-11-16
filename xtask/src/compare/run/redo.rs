//! A redo: the cells a trials file reads as not measured, or the cells
//! named, measured again into that file (#2521), so one lost cell costs one
//! cell's runs rather than a hole in the report or a whole run.

use anyhow::{Context as _, Result, bail};

use super::super::matrix::{self, Scenario};
use super::super::same_work;
use super::super::trials::{
    AutoRun, Comparison, CountedCell, CountedRun, GrowthCell, RuledRun, SizesCell, StoppedPoint,
    StoppedRun, Trial,
};
use super::CellName;

/// A record of one cell's: a trial, a run or a stopped run.
pub(super) trait OfCell {
    /// The record's scenario and cell.
    fn of_cell(&self) -> (Scenario, &str);
}

macro_rules! of_cell {
    ($($record:ty),*) => {$(
        impl OfCell for $record {
            fn of_cell(&self) -> (Scenario, &str) {
                (self.scenario, &self.cell)
            }
        }
    )*};
}

of_cell!(Trial, CountedRun, StoppedRun, RuledRun, AutoRun);

impl OfCell for StoppedPoint {
    fn of_cell(&self) -> (Scenario, &str) {
        self.run.of_cell()
    }
}

/// Whether `only` asks for `cell`: every cell when it names none.
pub(super) fn asks(only: &[CellName], (scenario, cell): (Scenario, &str)) -> bool {
    only.is_empty()
        || only
            .iter()
            .any(|&(asked, name)| asked == scenario && name == cell)
}

/// `again` in place of `earlier`'s records of the cells `only` names, the
/// other cells' records kept; `again` alone when `only` names none, as a
/// whole run measures every cell.
pub(super) fn merge<T: OfCell>(earlier: &mut Vec<T>, again: Vec<T>, only: &[CellName]) {
    if only.is_empty() {
        *earlier = again;
        return;
    }
    earlier.retain(|record| !asks(only, record.of_cell()));
    earlier.extend(again);
}

/// A counted cell measured again for the cells `only` names: their runs in
/// place of the earlier ones. A redo keeps a cell the earlier run did not
/// measure as it was.
pub(super) fn merge_counted(earlier: &mut CountedCell, again: CountedCell, only: &[CellName]) {
    if only.is_empty() {
        *earlier = again;
        return;
    }
    if let (
        CountedCell::Measured { runs, stopped, .. },
        CountedCell::Measured {
            runs: runs_again,
            stopped: stopped_again,
            ..
        },
    ) = (earlier, again)
    {
        merge(runs, runs_again, only);
        merge(stopped, stopped_again, only);
    }
}

/// The growth cell measured again for the cells `only` names, as
/// [`merge_counted`] merges a counted cell.
pub(super) fn merge_growth(earlier: &mut GrowthCell, again: GrowthCell, only: &[CellName]) {
    if only.is_empty() {
        *earlier = again;
        return;
    }
    if let (
        GrowthCell::Measured {
            trials_run,
            stopped,
            ..
        },
        GrowthCell::Measured {
            trials_run: trials_again,
            stopped: stopped_again,
            ..
        },
    ) = (earlier, again)
    {
        merge(trials_run, trials_again, only);
        merge(stopped, stopped_again, only);
    }
}

/// The sizes cell measured again for the cells `only` names, as
/// [`merge_counted`] merges a counted cell.
pub(super) fn merge_sizes(earlier: &mut SizesCell, again: SizesCell, only: &[CellName]) {
    if only.is_empty() {
        *earlier = again;
        return;
    }
    if let (
        SizesCell::Measured {
            trials_run,
            stopped,
            ..
        },
        SizesCell::Measured {
            trials_run: trials_again,
            stopped: stopped_again,
            ..
        },
    ) = (earlier, again)
    {
        merge(trials_run, trials_again, only);
        merge(stopped, stopped_again, only);
    }
}

/// The cell `name` of `scenario` as the matrix names it.
fn of_matrix(scenario: Scenario, name: &str) -> Result<CellName> {
    let cell = matrix::cell(scenario, name)
        .with_context(|| format!("no cell `{name}` in scenario {}", scenario.name()))?;
    Ok((scenario, cell.name))
}

/// The stopped runs of a counted cell: none when it did not run.
fn counted_stopped(cell: &CountedCell) -> &[StoppedRun] {
    match cell {
        CountedCell::Measured { stopped, .. } => stopped,
        CountedCell::NotRun { .. } => &[],
    }
}

/// The cells `file` reads as not measured, each once, in the order its
/// parts ran: a timed trial's, a sizes point's, a strace run's (the
/// report's own reading, a run of other work set aside included), a growth
/// trial's or a Callgrind run's stopped.
pub(crate) fn not_measured(mut file: Comparison) -> Result<Vec<CellName>> {
    same_work::strace_runs_of_other_work_set_aside(&mut file);
    let sizes = match &file.sizes {
        SizesCell::Measured { stopped, .. } => stopped.as_slice(),
        SizesCell::NotRun { .. } => &[],
    };
    let growth = match &file.growth {
        GrowthCell::Measured { stopped, .. } => stopped.as_slice(),
        GrowthCell::NotRun { .. } => &[],
    };
    let mut cells: Vec<CellName> = Vec::new();
    for run in file
        .stopped
        .iter()
        .chain(sizes.iter().map(|point| &point.run))
        .chain(counted_stopped(&file.syscalls))
        .chain(growth)
        .chain(counted_stopped(&file.instructions))
    {
        let cell = of_matrix(run.scenario, &run.cell)?;
        if !cells.contains(&cell) {
            cells.push(cell);
        }
    }
    Ok(cells)
}

/// The cell `named` names, `<scenario>/<cell>`, such as `browser/foxhound`,
/// of one of `scenarios`, the scenarios the run asks for or, in a redo,
/// those the trials file's run declared.
pub(crate) fn named(named: &str, scenarios: &[Scenario]) -> Result<CellName> {
    let Some((scenario, cell)) = named.split_once('/') else {
        bail!("`{named}` names no cell: name one as <scenario>/<cell>, such as browser/foxhound");
    };
    let Some(&scenario) = scenarios.iter().find(|asked| asked.name() == scenario) else {
        bail!("the run asked for no scenario `{scenario}`");
    };
    of_matrix(scenario, cell)
}

#[cfg(test)]
mod tests;
