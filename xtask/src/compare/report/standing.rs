//! The standings: in each scenario, each of the Product's main rows against
//! each other tool's main row, on every metric both give a lowest and a
//! highest trial of. `ahead` when every trial of the Product's is better
//! than every trial of the competitor's, `behind` when every one is worse,
//! otherwise `overlapping`: two ranges that cross are no ranking. On the
//! article site, a standing against a competitor whose output differs
//! under the check the Product's row is held to is labelled so (#2136):
//! a timed bar reads only against a competitor whose output passes (the
//! owner's decision of 2026-11-10, #2252 Q8).

use std::collections::BTreeMap;
use std::fmt::Write as _;

use serde::Serialize;

use super::{CellReport, CellStatus, Figure, ROWS, ScenarioReport, cell_text, output_differs};
use crate::compare::matrix::{self, Row, Scenario, Tool};

/// The Product's standing against a competitor on one metric.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Standing {
    Ahead,
    Behind,
    Overlapping,
}

/// The standings on one of the Product's cells against one competitor's.
#[derive(Serialize)]
pub(crate) struct Standings {
    pub(crate) product: &'static str,
    pub(crate) competitor: &'static str,
    /// The competitor's label when its output differs under the check the
    /// Product's row is held to: no bar reads such a standing.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) output_differs: Option<String>,
    pub(crate) metrics: BTreeMap<&'static str, Standing>,
}

/// The metrics where more is better: every other metric of `ROWS` is a
/// cost, where less is, so a metric added there where more is better is
/// named here too.
const MORE_IS_BETTER: &[&str] = &["pages_per_second", "steady_pages_per_second", "pages"];

/// The standing on `metric` from the Product's figure and the competitor's,
/// none unless both were measured.
fn judge(metric: &str, product: &Figure, competitor: &Figure) -> Option<Standing> {
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
        Standing::Ahead
    } else if worse {
        Standing::Behind
    } else {
        Standing::Overlapping
    })
}

/// A cell's metrics, when it is a main row whose figures a standing may
/// read: not Foxhound's cell at the catalog's defaults, which carries its
/// own target and leaves the counted criteria to the other (#2175).
fn judged(scenario: Scenario, cell: &CellReport) -> Option<&BTreeMap<&'static str, Figure>> {
    let counted = matrix::cell(scenario, cell.cell).is_some_and(matrix::Cell::counted_criteria);
    match &cell.status {
        CellStatus::Measured { metrics, .. } if cell.row == Row::Main && counted => Some(metrics),
        CellStatus::Measured { .. }
        | CellStatus::NotSupported { .. }
        | CellStatus::NotMeasured { .. }
        | CellStatus::Ruled { .. }
        | CellStatus::Auto { .. } => None,
    }
}

/// The competitor's label against a row of the Product's held to the
/// output check, when its pages differ under that row's check.
fn differs(scenario: Scenario, product: &CellReport, competitor: &CellReport) -> Option<String> {
    let check = matrix::cell(scenario, product.cell)?.output_check()?;
    let CellStatus::Measured { site_pages, .. } = &competitor.status else {
        return None;
    };
    output_differs(competitor.output.as_ref()?, check, *site_pages)
}

/// The standings of `scenario`'s cells.
pub(super) fn of(scenario: Scenario, cells: &[CellReport]) -> Vec<Standings> {
    let product = Tool::Foxhound.name();
    let mut standings = Vec::new();
    for (product_cell, product_metrics) in cells
        .iter()
        .filter(|cell| cell.tool == product)
        .filter_map(|cell| Some((cell, judged(scenario, cell)?)))
    {
        for (competitor_cell, competitor_metrics) in cells
            .iter()
            .filter(|cell| cell.tool != product)
            .filter_map(|cell| Some((cell, judged(scenario, cell)?)))
        {
            let metrics = product_metrics
                .iter()
                .filter_map(|(name, figure)| {
                    Some((*name, judge(name, figure, competitor_metrics.get(name)?)?))
                })
                .collect();
            standings.push(Standings {
                product: product_cell.cell,
                competitor: competitor_cell.cell,
                output_differs: differs(scenario, product_cell, competitor_cell),
                metrics,
            });
        }
    }
    standings
}

/// A cell's pages, as a standing's heading names them.
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
        CellStatus::NotSupported { .. }
        | CellStatus::NotMeasured { .. }
        | CellStatus::Ruled { .. }
        | CellStatus::Auto { .. } => None,
    }
}

/// The scenario's standing table, when it gives any standing.
pub(super) fn markdown(text: &mut String, scenario: &ScenarioReport) {
    if scenario.standings.is_empty() {
        return;
    }
    let cell = |name: &str| scenario.cells.iter().find(|cell| cell.cell == name);
    text.push_str(
        "\nThe standings of Foxhound against each other tool's main row, on every metric both \
         measured: `ahead` when every trial of Foxhound's is better than every trial of the \
         other's, `behind` when every one is worse, otherwise `overlapping`, with both medians \
         and both ranges. Better is more pages per second and more pages fetched, and less of \
         every other metric.",
    );
    if scenario
        .standings
        .iter()
        .any(|standings| standings.output_differs.is_some())
    {
        text.push_str(
            " A standing against a tool labelled `output differs` is against other work than \
             the Foxhound row's: its pages differ under the check that row is held to, and no \
             bar reads it.",
        );
    }
    text.push_str("\n\n| Standing |");
    for standings in &scenario.standings {
        let _ = write!(
            text,
            " {} ({}) against {} ({}{}) |",
            standings.product,
            pages(cell(standings.product)),
            standings.competitor,
            pages(cell(standings.competitor)),
            standings
                .output_differs
                .as_ref()
                .map_or_else(String::new, |label| format!(", {label}")),
        );
    }
    text.push_str("\n|---|");
    for _ in &scenario.standings {
        text.push_str("---|");
    }
    text.push('\n');
    for (name, label) in ROWS {
        if !scenario
            .standings
            .iter()
            .any(|standings| standings.metrics.contains_key(name))
        {
            continue;
        }
        let _ = write!(text, "| {label} |");
        for standings in &scenario.standings {
            let shown = match standings.metrics.get(name) {
                None => "no standing: a side was not measured".to_owned(),
                Some(Standing::Ahead) => "ahead".to_owned(),
                Some(Standing::Behind) => "behind".to_owned(),
                Some(Standing::Overlapping) => {
                    let side = |side: &str| {
                        figure(cell(side), name).map_or_else(String::new, |figure| {
                            format!("{side} {}", cell_text(figure))
                        })
                    };
                    format!(
                        "overlapping: {}, {}",
                        side(standings.product),
                        side(standings.competitor)
                    )
                }
            };
            let _ = write!(text, " {shown} |");
        }
        text.push('\n');
    }
}
