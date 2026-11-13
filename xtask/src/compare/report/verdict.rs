//! The verdicts: in each scenario, each of the Product's main rows against
//! each other tool's main row, on every metric both give a lowest and a
//! highest trial of. `ahead` when every trial of the Product's is better
//! than every trial of the competitor's, `behind` when every one is worse,
//! otherwise `overlapping`: two ranges that cross are no ranking. A cell
//! declared as not doing the Product's work gets none, and neither does a
//! scenario whose content no check read.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use serde::Serialize;

use super::{CellReport, CellStatus, Figure, ROWS, ScenarioReport, cell_text};
use crate::compare::matrix::{Row, Scenario, Tool};

/// One metric's verdict on the Product against a competitor.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Verdict {
    Ahead,
    Behind,
    Overlapping,
}

/// The verdicts on one of the Product's cells against one competitor's.
#[derive(Serialize)]
pub(crate) struct Verdicts {
    pub(crate) product: &'static str,
    pub(crate) competitor: &'static str,
    pub(crate) metrics: BTreeMap<&'static str, Verdict>,
}

/// The metrics where more is better: every other metric of `ROWS` is a
/// cost, where less is, so a metric added there where more is better is
/// named here too.
const MORE_IS_BETTER: &[&str] = &["pages_per_second", "pages"];

/// The verdict on `metric` from the Product's figure and the competitor's,
/// none unless both were measured.
fn judge(metric: &str, product: &Figure, competitor: &Figure) -> Option<Verdict> {
    let (
        Figure::Measured {
            low: product_low,
            high: product_high,
            ..
        },
        Figure::Measured {
            low: competitor_low,
            high: competitor_high,
            ..
        },
    ) = (product, competitor)
    else {
        return None;
    };
    let (better, worse) = if MORE_IS_BETTER.contains(&metric) {
        (product_low > competitor_high, product_high < competitor_low)
    } else {
        (product_high < competitor_low, product_low > competitor_high)
    };
    Some(if better {
        Verdict::Ahead
    } else if worse {
        Verdict::Behind
    } else {
        Verdict::Overlapping
    })
}

/// A cell's metrics, when it is a main row whose figures a verdict may
/// read.
fn judged(cell: &CellReport) -> Option<&BTreeMap<&'static str, Figure>> {
    match &cell.status {
        CellStatus::Measured { metrics, .. }
            if cell.row == Row::Main && cell.unlike_work.is_none() =>
        {
            Some(metrics)
        }
        CellStatus::Measured { .. }
        | CellStatus::NotSupported { .. }
        | CellStatus::NotMeasured { .. } => None,
    }
}

/// The verdicts of `scenario`'s cells: none when no check read the content
/// they wrote.
pub(super) fn of(scenario: Scenario, cells: &[CellReport]) -> Vec<Verdicts> {
    let product = Tool::Foxhound.name();
    let mut verdicts = Vec::new();
    if scenario.content_unverified() {
        return verdicts;
    }
    for (product_cell, product_metrics) in cells
        .iter()
        .filter(|cell| cell.tool == product)
        .filter_map(|cell| Some((cell, judged(cell)?)))
    {
        for (competitor_cell, competitor_metrics) in cells
            .iter()
            .filter(|cell| cell.tool != product)
            .filter_map(|cell| Some((cell, judged(cell)?)))
        {
            let metrics = product_metrics
                .iter()
                .filter_map(|(name, figure)| {
                    Some((*name, judge(name, figure, competitor_metrics.get(name)?)?))
                })
                .collect();
            verdicts.push(Verdicts {
                product: product_cell.cell,
                competitor: competitor_cell.cell,
                metrics,
            });
        }
    }
    verdicts
}

/// A cell's pages, as a verdict's heading names them.
fn pages(cell: Option<&CellReport>) -> String {
    match cell.map(|cell| &cell.status) {
        Some(CellStatus::Measured {
            pages, site_pages, ..
        }) => format!("{pages} of {site_pages} pages"),
        _ => "no pages".to_owned(),
    }
}

/// A metric's figure in `cell`.
fn figure<'a>(cell: Option<&'a CellReport>, metric: &str) -> Option<&'a Figure> {
    match &cell?.status {
        CellStatus::Measured { metrics, .. } => metrics.get(metric),
        CellStatus::NotSupported { .. } | CellStatus::NotMeasured { .. } => None,
    }
}

/// The scenario's verdict table, when it gives any verdict.
pub(super) fn markdown(text: &mut String, scenario: &ScenarioReport) {
    if scenario.verdicts.is_empty() {
        return;
    }
    let cell = |name: &str| scenario.cells.iter().find(|cell| cell.cell == name);
    text.push_str(
        "\nThe verdicts of Foxhound against each other tool's main row, on every metric both \
         measured: `ahead` when every trial of Foxhound's is better than every trial of the \
         other's, `behind` when every one is worse, otherwise `overlapping`, with both medians \
         and both ranges. Better is more pages per second and more pages fetched, and less of \
         every other metric.\n\n| Verdict |",
    );
    for verdicts in &scenario.verdicts {
        let _ = write!(
            text,
            " {} ({}) against {} ({}) |",
            verdicts.product,
            pages(cell(verdicts.product)),
            verdicts.competitor,
            pages(cell(verdicts.competitor)),
        );
    }
    text.push_str("\n|---|");
    for _ in &scenario.verdicts {
        text.push_str("---|");
    }
    text.push('\n');
    for (name, label) in ROWS {
        if !scenario
            .verdicts
            .iter()
            .any(|verdicts| verdicts.metrics.contains_key(name))
        {
            continue;
        }
        let _ = write!(text, "| {label} |");
        for verdicts in &scenario.verdicts {
            let shown = match verdicts.metrics.get(name) {
                None => "no verdict: a side was not measured".to_owned(),
                Some(Verdict::Ahead) => "ahead".to_owned(),
                Some(Verdict::Behind) => "behind".to_owned(),
                Some(Verdict::Overlapping) => {
                    let side = |side: &str| {
                        figure(cell(side), name).map_or_else(String::new, |figure| {
                            format!("{side} {}", cell_text(figure))
                        })
                    };
                    format!(
                        "overlapping: {}, {}",
                        side(verdicts.product),
                        side(verdicts.competitor)
                    )
                }
            };
            let _ = write!(text, " {shown} |");
        }
        text.push('\n');
    }
}
