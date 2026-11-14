//! The Collie scenario's section of the report (issue 2282): each cell's
//! median figures per action and per tool, the `actionability.test` pages
//! each tool got right, `collie.playwright_command_ratio` beside the
//! register row's Today, the Collie row each arm is read against (issues
//! 2443 and 2441), the opening cell's wall time beside the commands
//! awaited one after another, and the page-load cell's rows (issue 2319).

use std::collections::BTreeMap;
use std::fmt::Write as _;

use goodfetch_protocol::result_document::OracleOutcome;
use serde::Serialize;

use crate::register::document;

use super::super::report::{Figure, Unit, cell_text, float, median, spread};
use super::super::trials::{
    AGENT_BROWSER_VERSION, CollieScenario, CollieStopped, CollieTrial, Contender, Exercise,
    PLAYWRIGHT_VERSION, PUPPETEER_VERSION, PageDone, STAGEHAND_VERSION,
};
use super::page_load::{self, LoadSection};
use super::{ACTION_TIMEOUT_MILLISECONDS, EXERCISES, RATIO_CELLS, RATIO_ROW};

/// A page some trial did not get right.
#[derive(Serialize)]
pub(crate) struct Missed {
    pub(crate) task: String,
    /// The oracle's outcome in the first trial that did not get it right.
    pub(crate) outcome: Option<OracleOutcome>,
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
    /// The commands awaited one after another, those sent together or
    /// while others were in flight counting once.
    pub(crate) rounds_per_action: Figure,
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
    /// TESTING.md has no Quality register, or its register no such row.
    NoRow,
}

/// The Collie row an arm is read against, by the `actionability.test`
/// pages it got right (owner, #2443 and #2441).
#[derive(Serialize)]
#[serde(tag = "row", rename_all = "snake_case")]
pub(crate) enum Placement {
    /// It got right every page Playwright got right.
    GuardOn,
    /// It did not get right the pages `missed`, which Playwright got right:
    /// read against the Guard-off row, which #2440 builds.
    GuardOff { missed: Vec<String> },
    /// The pages right of it or of Playwright are not all known.
    NotPlaced { reason: String },
}

/// The issue that builds Collie's Guard-off row.
const GUARD_OFF_ISSUE: &str = "#2440";

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
        /// Each arm's Collie row.
        placements: BTreeMap<&'static str, Placement>,
        ratios: Vec<CommandRatio>,
        today: Today,
        page_load: LoadSection,
    },
}

/// Each `actionability.test` page `trials` ran, in the order of its
/// number, with the first of them that did not get it right.
fn verified<'a>(trials: &[&'a CollieTrial]) -> Vec<(&'a str, Option<&'a PageDone>)> {
    let mut tasks: BTreeMap<&str, Option<&PageDone>> = BTreeMap::new();
    for trial in trials {
        for page in &trial.pages {
            if !page.task.starts_with("actionability/") {
                continue;
            }
            let missed = tasks.entry(&page.task).or_default();
            if missed.is_none() && page.outcome != Some(OracleOutcome::Verified) {
                *missed = Some(page);
            }
        }
    }
    let mut ordered: Vec<(&str, Option<&PageDone>)> = tasks.into_iter().collect();
    ordered.sort_by_key(|(task, _)| {
        task.rsplit_once('/')
            .and_then(|(_, number)| number.parse::<u32>().ok())
    });
    ordered
}

/// The `actionability.test` pages of `trials` right in every one of them.
fn right(trials: &[&CollieTrial]) -> Option<Tally> {
    let ordered = verified(trials);
    if ordered.is_empty() {
        return None;
    }
    let ran = ordered.len();
    let not_right: Vec<Missed> = ordered
        .into_iter()
        .filter_map(|(task, missed)| {
            missed.map(|page| Missed {
                task: task.to_owned(),
                outcome: page.outcome,
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
fn today(register: &str) -> Today {
    let Some(register) = document::Register::read(register) else {
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

/// A tool's figures in a cell where its trial `stopped` was stopped twice.
fn not_measured(stopped: &CollieStopped) -> ToolFigures {
    none_measured(&format!(
        "trial {} was stopped twice: {}",
        stopped.number, stopped.reason
    ))
}

/// A tool's figures in a cell it has none in, for `reason`.
fn none_measured(reason: &str) -> ToolFigures {
    let figure = || Figure::NotMeasured {
        reason: reason.to_owned(),
    };
    ToolFigures {
        commands_per_action: figure(),
        rounds_per_action: figure(),
        cpu_seconds_per_action: figure(),
        wall_seconds_per_action: figure(),
        peak_bytes: figure(),
        pages_right: None,
    }
}

/// A tool's figures over its trials `ran` of one cell, each per action it
/// took, and the median of its commands per action.
fn figures(ran: &[&CollieTrial]) -> (ToolFigures, f64) {
    let per_action = |value: &dyn Fn(&CollieTrial) -> f64| -> Vec<f64> {
        ran.iter()
            .map(|trial| value(trial) / float(trial.actions))
            .collect()
    };
    let commands = per_action(&|trial| float(trial.commands));
    let figures = ToolFigures {
        commands_per_action: spread(&commands, Unit::Count),
        rounds_per_action: spread(&per_action(&|trial| float(trial.rounds)), Unit::Count),
        cpu_seconds_per_action: spread(&per_action(&|trial| trial.cpu_seconds), Unit::Seconds),
        wall_seconds_per_action: spread(&per_action(&|trial| trial.wall_seconds), Unit::Seconds),
        peak_bytes: spread(
            &ran.iter()
                .map(|trial| float(trial.peak_bytes))
                .collect::<Vec<_>>(),
            Unit::Bytes,
        ),
        pages_right: right(ran),
    };
    (figures, median(&commands))
}

/// The Collie scenario's section of the report, its ratio held to the
/// Today of TESTING.md's `register`.
pub(crate) fn section(scenario: &CollieScenario, register: &str) -> Section {
    let (trials, tools_run, trials_run, stopped, loads, loads_stopped) = match scenario {
        CollieScenario::NotRun { reason } => {
            return Section::NotRun {
                reason: reason.clone(),
            };
        }
        CollieScenario::Measured {
            trials,
            tools,
            trials_run,
            stopped,
            loads,
            loads_stopped,
        } => (trials, tools, trials_run, stopped, loads, loads_stopped),
    };
    let today = today(register);
    let mut cells = Vec::new();
    let mut ratios = Vec::new();
    for exercise in EXERCISES {
        let actions = super::cell(exercise).actions();
        let mut tools = BTreeMap::new();
        let mut medians = BTreeMap::new();
        for &tool in tools_run {
            if let Some(reason) = super::skipped(tool, exercise) {
                tools.insert(tool.name(), none_measured(reason));
                continue;
            }
            if let Some(stopped) = stopped
                .iter()
                .find(|run| run.exercise == exercise && run.tool == tool)
            {
                tools.insert(tool.name(), not_measured(stopped));
                continue;
            }
            let ran: Vec<&CollieTrial> = trials_run
                .iter()
                .filter(|trial| trial.exercise == exercise && trial.tool == tool)
                .collect();
            let (figures, commands) = figures(&ran);
            medians.insert(tool, commands);
            tools.insert(tool.name(), figures);
        }
        if RATIO_CELLS.contains(&exercise)
            && let (Some(&collie), Some(&playwright)) = (
                medians.get(&Contender::Collie),
                medians.get(&Contender::Playwright),
            )
        {
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
    let pages_right = tools_run
        .iter()
        .filter_map(|&tool| {
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
        placements: tools_run
            .iter()
            .filter(|tool| Contender::ARMS.contains(tool))
            .map(|&arm| (arm.name(), placement(arm, trials_run, stopped)))
            .collect(),
        ratios,
        today,
        page_load: page_load::section(loads, loads_stopped),
    }
}

/// The Collie row `arm` is read against: Guard on when it got right every
/// `actionability.test` page Playwright got right, over every cell; Guard
/// off otherwise; and not placed while a cell of such pages has either
/// tool not measured, or either ran none.
fn placement(arm: Contender, trials_run: &[CollieTrial], stopped: &[CollieStopped]) -> Placement {
    let not_measured = |tool: Contender| {
        stopped.iter().find(|run| {
            run.tool == tool
                && super::cell(run.exercise)
                    .pages()
                    .iter()
                    .any(|page| page.task.starts_with("actionability/"))
        })
    };
    if let Some(run) = not_measured(arm) {
        return Placement::NotPlaced {
            reason: format!("its {} cell was not measured", run.exercise.name()),
        };
    }
    if let Some(run) = not_measured(Contender::Playwright) {
        return Placement::NotPlaced {
            reason: format!("Playwright's {} cell was not measured", run.exercise.name()),
        };
    }
    let pages = |tool: Contender| {
        let ran: Vec<&CollieTrial> = trials_run
            .iter()
            .filter(|trial| trial.tool == tool)
            .collect();
        verified(&ran)
            .into_iter()
            .map(|(task, missed)| (task.to_owned(), missed.is_none()))
            .collect::<Vec<_>>()
    };
    let (own, playwright) = (pages(arm), pages(Contender::Playwright));
    if own.is_empty() {
        return Placement::NotPlaced {
            reason: "it ran no `actionability.test` page".to_owned(),
        };
    }
    if playwright.is_empty() {
        return Placement::NotPlaced {
            reason: "Playwright ran no `actionability.test` page".to_owned(),
        };
    }
    let missed: Vec<String> = playwright
        .into_iter()
        .filter(|(task, right)| *right && !own.contains(&(task.clone(), true)))
        .map(|(task, _)| task)
        .collect();
    if missed.is_empty() {
        Placement::GuardOn
    } else {
        Placement::GuardOff { missed }
    }
}

/// The calls each tool's steps map to, as the drivers make them.
fn mapping(tool: Contender) -> String {
    match tool {
        Contender::Collie => {
            "Collie's TypeScript SDK, `locator(…).click()`, `.fill()` and `.press()`, the Guard on"
                .to_owned()
        }
        Contender::Playwright => format!(
            "Playwright {PLAYWRIGHT_VERSION}, `locator(…).click()`, `.fill()` and `.press()`, \
             its actionability checks on"
        ),
        Contender::PuppeteerLocator => format!(
            "Puppeteer {PUPPETEER_VERSION}'s locators, `Locator.click()` and `Locator.fill()`, \
             and `waitHandle()` then `ElementHandle.press()`; its click waits for the viewport, \
             a stable box and enabled, not for visible or uncovered"
        ),
        Contender::PuppeteerPage => format!(
            "Puppeteer {PUPPETEER_VERSION}'s page methods, `page.click()`, `page.type()`, and \
             `page.focus()` then `page.keyboard.press()`; its click checks nothing"
        ),
        Contender::Stagehand => format!(
            "Stagehand {STAGEHAND_VERSION} with no model, `page.locator(…).click()` and \
             `.fill()`, and a focus then `page.keyPress()`; its click neither waits nor checks"
        ),
        Contender::AgentBrowser => format!(
            "agent-browser {AGENT_BROWSER_VERSION}'s command line, `agent-browser click`, \
             `fill`, and `focus` then `press`, each a process that hands the command to its \
             daemon, with an `eval` after each `open` for the status `open` does not answer; its \
             click checks once that its target is visible, enabled, in the viewport and not \
             covered, and does not wait"
        ),
    }
}

/// Each tool's calls and each arm's Collie row, appended to `text`.
fn arms_text(
    text: &mut String,
    cells: &[CellFigures],
    placements: &BTreeMap<&'static str, Placement>,
) {
    if placements.is_empty() {
        return;
    }
    text.push_str("\nWhat each tool's steps call:\n\n");
    for tool in Contender::ALL {
        if cells
            .iter()
            .any(|cell| cell.tools.contains_key(tool.name()))
        {
            let _ = writeln!(text, "- {}: {}.", tool.name(), mapping(tool));
        }
    }
    text.push_str(
        "\nThe Collie row each arm is read against: Guard on when it got right every \
         `actionability.test` page Playwright got right, Guard off otherwise:\n\n",
    );
    for (arm, placement) in placements {
        let _ = match placement {
            Placement::GuardOn => writeln!(text, "- {arm}: Collie's Guard-on row."),
            Placement::GuardOff { missed } => writeln!(
                text,
                "- {arm}: Collie's Guard-off row ({GUARD_OFF_ISSUE}), still to come: it did not \
                 get {} right, which Playwright got right.",
                missed.join(", ")
            ),
            Placement::NotPlaced { reason } => writeln!(text, "- {arm}: not placed, {reason}."),
        };
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
    for exercise in RATIO_CELLS {
        let Some(ratio) = ratios.iter().find(|ratio| ratio.exercise == exercise) else {
            let _ = writeln!(
                text,
                "- {}: not measured, a tool's trial having been stopped twice.",
                exercise.name()
            );
            continue;
        };
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
    let mut text =
        String::from("\n## The Collie scenario: Collie against Playwright and the arms\n\n");
    let (trials, cells, pages_right, placements, ratios, today, loads) = match section {
        Section::NotRun { reason } => {
            let _ = writeln!(text, "The Collie scenario was not run: {reason}.");
            return text;
        }
        Section::Measured {
            trials,
            cells,
            pages_right,
            placements,
            ratios,
            today,
            page_load,
        } => (
            trials,
            cells,
            pages_right,
            placements,
            ratios,
            today,
            page_load,
        ),
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
    if !placements.is_empty() {
        text.push_str(
            "Beside them the arms, each through its own library attached the same way: \
             Puppeteer's locators and its page methods, agent-browser's command line, and \
             Stagehand with no model, whose browser is launched with \
             `--enable-unsafe-extension-debugging` and `--remote-allow-origins=*` for the \
             extension it drives the browser through. That \
             extension runs in the browser's processes, so Stagehand's CPU and memory leave its \
             work out, while its CDP commands pass the relay and are counted. Stagehand drives \
             the browser's one context, so it is not measured in the Sessions cell. agent-browser \
             runs each step as a command, a process of its own that hands it to a daemon the \
             first one started; the daemon is in agent-browser's process tree, and each \
             command's CPU counts once the driver waited for it. Its daemon opens a context it \
             cannot close, so it is not measured in the Sessions cell either.\n\n",
        );
    }
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
    opening_text(&mut text, cells);
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
                    missed.outcome.map_or("no outcome", OracleOutcome::as_str),
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
    arms_text(&mut text, cells, placements);
    ratios_text(&mut text, ratios, today);
    page_load::markdown(&mut text, loads);
    text
}

/// The opening cell's wall time per page opened beside the CDP commands
/// each tool awaited one after another, appended to `text`; it has no bar.
fn opening_text(text: &mut String, cells: &[CellFigures]) {
    let Some(cell) = cells.iter().find(|cell| cell.exercise == Exercise::Opening) else {
        return;
    };
    let _ = writeln!(
        text,
        "\nPages opened one after another in one Session, each kept open ({} of them), \
         per page opened, with no bar:\n",
        cell.actions
    );
    for (tool, figures) in &cell.tools {
        let (
            Figure::Measured { median: wall, .. },
            Figure::Measured { median: rounds, .. },
            Figure::Measured {
                median: commands, ..
            },
        ) = (
            &figures.wall_seconds_per_action,
            &figures.rounds_per_action,
            &figures.commands_per_action,
        )
        else {
            let _ = writeln!(text, "- {tool}: not measured.");
            continue;
        };
        let _ = writeln!(
            text,
            "- {tool}: {:.1} ms per page opened, {rounds:.2} CDP commands awaited one after \
             another, of {commands:.2} sent.",
            wall * 1000.0
        );
    }
}
