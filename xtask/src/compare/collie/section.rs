//! The Collie scenario's section of the report (issue 2282): each cell's
//! median figures per action and per tool, the `actionability.test` pages
//! each tool got right, and `collie.playwright_command_ratio` beside the
//! register row's Today.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use serde::Serialize;

use crate::register::document;

use super::super::report::{Figure, Unit, cell_text, median, spread};
use super::super::trials::{CollieCell, CollieTrial, Driver, Exercise, PageDone};
use super::{ACTION_TIMEOUT_MILLISECONDS, EXERCISES, RATIO_CELLS, RATIO_ROW};

/// A page some trial did not get right.
#[derive(Serialize)]
pub(crate) struct Missed {
    pub(crate) task: String,
    /// The oracle's outcome in the first trial that did not get it right.
    pub(crate) verdict: Option<String>,
    pub(crate) error: Option<String>,
}

/// The `actionability.test` pages a tool got right in every trial, of
/// those it ran.
#[derive(Serialize)]
pub(crate) struct Tally {
    pub(crate) right: usize,
    pub(crate) ran: usize,
    pub(crate) not_right: Vec<Missed>,
}

/// One tool's figures in a cell.
#[derive(Serialize)]
pub(crate) struct ToolFigures {
    pub(crate) commands_per_action: Figure,
    pub(crate) cpu_seconds_per_action: Figure,
    pub(crate) wall_seconds_per_action: Figure,
    pub(crate) peak_bytes: Figure,
    /// None in a cell of no `actionability.test` page.
    pub(crate) pages_right: Option<Tally>,
}

/// One cell's figures.
#[derive(Serialize)]
pub(crate) struct CellFigures {
    pub(crate) exercise: Exercise,
    pub(crate) actions: u64,
    pub(crate) tools: BTreeMap<&'static str, ToolFigures>,
}

/// A cell's command ratio.
#[derive(Serialize)]
pub(crate) struct CommandRatio {
    pub(crate) exercise: Exercise,
    /// Each tool's median commands per action.
    pub(crate) collie: f64,
    pub(crate) playwright: f64,
    /// Collie's over Playwright's, to three decimals, as the register's
    /// figures are.
    pub(crate) ratio: f64,
    /// Whether it is over the row's Today; none while the row has no
    /// Today figure.
    pub(crate) over_today: Option<bool>,
}

/// The register row's Today, as TESTING.md gives it.
#[derive(Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub(crate) enum Today {
    Figure {
        today: f64,
    },
    NoFigure {
        text: String,
    },
    /// TESTING.md's register has no such row, or could not be read.
    NoRow,
}

/// The Collie scenario's part of the report.
#[derive(Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub(crate) enum Section {
    NotRun {
        reason: String,
    },
    Measured {
        trials: u32,
        cells: Vec<CellFigures>,
        /// Over every cell, each `actionability.test` page once.
        pages_right: BTreeMap<&'static str, Tally>,
        ratios: Vec<CommandRatio>,
        today: Today,
    },
}

/// `count` as a float, for a figure.
#[expect(
    clippy::cast_precision_loss,
    reason = "actions, commands and bytes here stay far below 2^52, where a float is exact"
)]
fn float(count: u64) -> f64 {
    count as f64
}

/// The `actionability.test` pages of `trials` right in every one of them.
fn right(trials: &[&CollieTrial]) -> Option<Tally> {
    let mut tasks: BTreeMap<&str, Option<&PageDone>> = BTreeMap::new();
    for trial in trials {
        for page in &trial.pages {
            if !page.task.starts_with("actionability/") {
                continue;
            }
            let missed = tasks.entry(&page.task).or_default();
            if missed.is_none() && page.verdict.as_deref() != Some("verified") {
                *missed = Some(page);
            }
        }
    }
    if tasks.is_empty() {
        return None;
    }
    let mut ordered: Vec<(&str, Option<&PageDone>)> = tasks.into_iter().collect();
    ordered.sort_by_key(|(task, _)| {
        task.rsplit_once('/')
            .and_then(|(_, number)| number.parse::<u32>().ok())
    });
    let ran = ordered.len();
    let not_right: Vec<Missed> = ordered
        .into_iter()
        .filter_map(|(task, missed)| {
            missed.map(|page| Missed {
                task: task.to_owned(),
                verdict: page.verdict.clone(),
                error: page.error.clone(),
            })
        })
        .collect();
    Some(Tally {
        right: ran - not_right.len(),
        ran,
        not_right,
    })
}

/// The row's Today in TESTING.md's `register`.
fn today(register: Option<&str>) -> Today {
    let Some(register) = register.and_then(document::Register::read) else {
        return Today::NoRow;
    };
    let rows = register.rows();
    let Some(row) = rows.iter().find(|row| row.name() == RATIO_ROW) else {
        return Today::NoRow;
    };
    let text = row.cell("Today");
    document::figure(text).map_or_else(
        || Today::NoFigure {
            text: text.to_owned(),
        },
        |today| Today::Figure { today },
    )
}

/// The Collie scenario's section of the report, its ratio held to the
/// Today of TESTING.md's `register`.
pub(crate) fn section(cell: &CollieCell, register: Option<&str>) -> Section {
    let CollieCell::Measured { trials, trials_run } = cell else {
        let CollieCell::NotRun { reason } = cell else {
            unreachable!("invariant: a Collie cell is run or not")
        };
        return Section::NotRun {
            reason: reason.clone(),
        };
    };
    let today = today(register);
    let mut cells = Vec::new();
    let mut ratios = Vec::new();
    for exercise in EXERCISES {
        let actions = super::cell(exercise).actions();
        let mut tools = BTreeMap::new();
        let mut medians = BTreeMap::new();
        for tool in Driver::ALL {
            let ran: Vec<&CollieTrial> = trials_run
                .iter()
                .filter(|trial| trial.exercise == exercise && trial.tool == tool)
                .collect();
            let per_action = |value: &dyn Fn(&CollieTrial) -> f64| -> Vec<f64> {
                ran.iter()
                    .map(|trial| value(trial) / float(trial.actions))
                    .collect()
            };
            let commands = per_action(&|trial| float(trial.commands));
            medians.insert(tool, median(&commands));
            tools.insert(
                tool.name(),
                ToolFigures {
                    commands_per_action: spread(&commands, Unit::Count),
                    cpu_seconds_per_action: spread(
                        &per_action(&|trial| trial.cpu_seconds),
                        Unit::Seconds,
                    ),
                    wall_seconds_per_action: spread(
                        &per_action(&|trial| trial.wall_seconds),
                        Unit::Seconds,
                    ),
                    peak_bytes: spread(
                        &ran.iter()
                            .map(|trial| float(trial.peak_bytes))
                            .collect::<Vec<_>>(),
                        Unit::Bytes,
                    ),
                    pages_right: right(&ran),
                },
            );
        }
        if RATIO_CELLS.contains(&exercise) {
            let (collie, playwright) = (medians[&Driver::Collie], medians[&Driver::Playwright]);
            let ratio = (collie / playwright * 1000.0).round() / 1000.0;
            ratios.push(CommandRatio {
                exercise,
                collie,
                playwright,
                ratio,
                over_today: match today {
                    Today::Figure { today } => Some(ratio > today),
                    Today::NoFigure { .. } | Today::NoRow => None,
                },
            });
        }
        cells.push(CellFigures {
            exercise,
            actions,
            tools,
        });
    }
    let pages_right = Driver::ALL
        .into_iter()
        .filter_map(|tool| {
            let ran: Vec<&CollieTrial> = trials_run
                .iter()
                .filter(|trial| trial.tool == tool)
                .collect();
            Some((tool.name(), right(&ran)?))
        })
        .collect();
    Section::Measured {
        trials: *trials,
        cells,
        pages_right,
        ratios,
        today,
    }
}

/// The pages right as the report writes them: `12 of 13`.
fn right_text(right: Option<&Tally>) -> String {
    right.map_or_else(String::new, |right| {
        format!("{} of {}", right.right, right.ran)
    })
}

/// The ratios of the click and form cells, and how they stand to the row's
/// Today, appended to `text`.
fn ratios_text(text: &mut String, ratios: &[CommandRatio], today: &Today) {
    let _ = writeln!(
        text,
        "\n`{RATIO_ROW}`, Collie's CDP commands per action over Playwright's, Guard on:\n"
    );
    for ratio in ratios {
        let _ = writeln!(
            text,
            "- {}: {:.3}, Collie's {:.2} over Playwright's {:.2}.",
            ratio.exercise.name(),
            ratio.ratio,
            ratio.collie,
            ratio.playwright
        );
    }
    match today {
        Today::Figure { today } => {
            let over: Vec<String> = ratios
                .iter()
                .filter(|ratio| ratio.over_today == Some(true))
                .map(|ratio| format!("{} ({:.3})", ratio.exercise.name(), ratio.ratio))
                .collect();
            if over.is_empty() {
                let _ = writeln!(text, "\nNo ratio is over the row's Today, {today:.3}.");
            } else {
                let _ = writeln!(
                    text,
                    "\nThe ratio is over the row's Today, {today:.3}, in: {}.",
                    over.join(", ")
                );
            }
        }
        Today::NoFigure { text: cell } => {
            let _ = writeln!(
                text,
                "\nThe row has no Today figure yet ({cell}): no ratio is held to it."
            );
        }
        Today::NoRow => {
            let _ = writeln!(
                text,
                "\nTESTING.md's register has no row `{RATIO_ROW}`: no ratio is held to it."
            );
        }
    }
}

/// The Collie scenario's section in Markdown.
pub(crate) fn markdown(section: &Section) -> String {
    let mut text = String::from("\n## The Collie scenario: Collie against Playwright\n\n");
    let Section::Measured {
        trials,
        cells,
        pages_right,
        ratios,
        today,
    } = section
    else {
        let Section::NotRun { reason } = section else {
            unreachable!("invariant: a section is run or not")
        };
        let _ = writeln!(text, "The Collie scenario was not run: {reason}.");
        return text;
    };
    let _ = writeln!(
        text,
        "Collie's Attached engine, through its TypeScript SDK, against Playwright with its \
         actionability checks on (no `force`), both attached over CDP to one Chrome for Testing \
         {} that xtask launched, through the counting relay. {trials} trials of each tool in each \
         cell, interleaved in random order, each in a fresh process against a fresh Proving ground \
         and browser; actions wait {ACTION_TIMEOUT_MILLISECONDS} ms for their target. Each figure \
         is the median (lowest–highest) per action, between the marks the driver asks for once \
         attached; peak memory is the tool's process tree, the browser's processes not in it, \
         and the browser's launch is in no figure. Pages right are the `actionability.test` pages \
         the oracle verified in every trial.\n",
        crate::test_facts::CHROME_FOR_TESTING_BUILD
    );
    text.push_str(
        "| Cell | Actions | Tool | CDP commands | CPU | Wall | Pages right | Peak memory |\n\
         |---|---:|---|---:|---:|---:|---:|---:|\n",
    );
    for cell in cells {
        for (tool, figures) in &cell.tools {
            let _ = writeln!(
                text,
                "| {} | {} | {tool} | {} | {} | {} | {} | {} |",
                cell.exercise.name(),
                cell.actions,
                cell_text(&figures.commands_per_action),
                cell_text(&figures.cpu_seconds_per_action),
                cell_text(&figures.wall_seconds_per_action),
                right_text(figures.pages_right.as_ref()),
                cell_text(&figures.peak_bytes),
            );
        }
    }
    text.push_str("\nPages of `actionability.test` right, over every cell:\n\n");
    for (tool, right) in pages_right {
        let _ = write!(text, "- {tool}: {} of {}", right.right, right.ran);
        let missed: Vec<String> = right
            .not_right
            .iter()
            .map(|missed| {
                format!(
                    "{} ({}{})",
                    missed.task,
                    missed.verdict.as_deref().unwrap_or("no verdict"),
                    missed
                        .error
                        .as_ref()
                        .map_or_else(String::new, |error| format!(": {error}"))
                )
            })
            .collect();
        if missed.is_empty() {
            text.push_str(".\n");
        } else {
            let _ = writeln!(text, "; not right: {}.", missed.join(", "));
        }
    }
    ratios_text(&mut text, ratios, today);
    text
}
