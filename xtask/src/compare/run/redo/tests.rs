use std::path::Path;

use super::super::{Executables, Options, redo};
use super::{merge, named, not_measured};
use crate::compare::matrix::Scenario;
use crate::compare::report::{self, CellStatus};
use crate::compare::trials::{
    Comparison, CountedCell, CountedRun, Measured, StoppedRun, Tool, Work, fixture,
};

/// The pages of the site every trial crawls.
const PAGES: u64 = 1000;

/// The work of a run that fetched every page of `scenario`'s site once.
fn whole(scenario: Scenario) -> Work {
    let pages = scenario.site_pages(PAGES);
    Work {
        pages,
        requests: pages,
        bytes: 600 * pages,
        first_bytes: 600 * pages,
        ..Work::default()
    }
}

/// A run's trials, each of which fetched its scenario's site.
fn file() -> Comparison {
    fixture::comparison(PAGES, |cell, _| Measured {
        seconds: 1.0,
        work: whole(cell.scenario),
        ..Measured::default()
    })
}

/// A stopped run of `scenario`'s `cell`.
fn stopped(scenario: Scenario, cell: &str, tool: Tool) -> StoppedRun {
    StoppedRun {
        scenario,
        cell: cell.to_owned(),
        tool,
        number: 2,
        reason: "its crawl stalled".to_owned(),
    }
}

/// A run under strace of `scenario`'s `cell` that did `work`.
fn counted(scenario: Scenario, cell: &str, tool: Tool, number: u32, work: Work) -> CountedRun {
    CountedRun {
        scenario,
        cell: cell.to_owned(),
        tool,
        number,
        load: None,
        count: 9,
        syncs: 0,
        work,
        extracted: None,
    }
}

/// The report's reading of `scenario`'s `cell`: its trials when measured,
/// none when not.
fn measured_trials(file: &Comparison, scenario: Scenario, cell: &str) -> Option<usize> {
    let report = report::of(file);
    let status = &report
        .scenarios
        .iter()
        .find(|report| report.scenario == scenario)?
        .cells
        .iter()
        .find(|report| report.cell == cell)?
        .status;
    match status {
        CellStatus::Measured { trials, .. } => Some(*trials),
        CellStatus::NotSupported { .. }
        | CellStatus::NotMeasured { .. }
        | CellStatus::Ruled { .. }
        | CellStatus::Auto { .. } => None,
    }
}

/// A redo measures again each cell the file reads as not measured, once:
/// one a timed trial's stop left out, one a strace run stopped, and one
/// whose strace run missed a page, which the report sets aside (#2509).
#[test]
fn a_redo_measures_again_each_cell_the_file_reads_as_not_measured() {
    let mut earlier = file();
    earlier.stopped = vec![stopped(Scenario::Raw, "spider", Tool::Spider)];
    let mut short = whole(Scenario::Browser);
    short.pages -= 1;
    short.missing = vec![format!("/sites/{PAGES}/{PAGES}")];
    earlier.syscalls = CountedCell::Measured {
        trials: 3,
        runs: (1..=3)
            .map(|number| {
                let work = if number == 2 {
                    short.clone()
                } else {
                    whole(Scenario::Browser)
                };
                counted(Scenario::Browser, "foxhound", Tool::Foxhound, number, work)
            })
            .collect(),
        stopped: vec![
            stopped(Scenario::Raw, "spider", Tool::Spider),
            stopped(Scenario::Raw, "crawlee", Tool::Crawlee),
        ],
    };

    assert_eq!(
        not_measured(earlier).expect("invariant: the file's cells are the matrix's"),
        vec![
            (Scenario::Raw, "spider"),
            (Scenario::Raw, "crawlee"),
            (Scenario::Browser, "foxhound"),
        ]
    );
}

/// Once a redo's trials of the one cell a run left not measured replace
/// its earlier ones, the report holds every cell, and every other cell's
/// trials are the earlier run's.
#[test]
fn a_cell_measured_again_completes_the_report() {
    let only = [(Scenario::Browser, "foxhound")];
    let mut earlier = file();
    let all = earlier.trials.len();
    let (again, kept): (Vec<_>, Vec<_>) = earlier
        .trials
        .drain(..)
        .partition(|trial| trial.scenario == Scenario::Browser && trial.cell == "foxhound");
    earlier.trials = kept;
    earlier.stopped = vec![stopped(Scenario::Browser, "foxhound", Tool::Foxhound)];
    let before = measured_trials(&earlier, Scenario::Browser, "foxhound");

    merge(&mut earlier.trials, again, &only);
    merge(&mut earlier.stopped, Vec::new(), &only);

    assert_eq!(
        (
            before,
            measured_trials(&earlier, Scenario::Browser, "foxhound"),
            earlier.trials.len(),
            earlier.stopped.len(),
        ),
        (None, Some(3), all, 0)
    );
}

/// A cell is named `<scenario>/<cell>`, of a scenario the file's run asked
/// for; another name is refused with how to give one.
#[test]
fn a_redo_names_its_cells_by_scenario_and_cell() {
    let asked = [Scenario::Raw, Scenario::Browser];

    assert_eq!(
        named("browser/spider capacity 4096", &asked).ok(),
        None,
        "the browser scenario has no such cell"
    );
    assert_eq!(
        named("browser/foxhound", &asked).ok(),
        Some((Scenario::Browser, "foxhound"))
    );
    assert_eq!(
        (
            named("foxhound", &asked).map_err(|error| error.to_string()),
            named("long/foxhound", &asked).map_err(|error| error.to_string()),
        ),
        (
            Err(
                "`foxhound` names no cell: name one as <scenario>/<cell>, such as browser/foxhound"
                    .to_owned()
            ),
            Err("the run asked for no scenario `long`".to_owned()),
        )
    );
}

/// A redo of a file of another commit, or of another machine, is refused
/// before it runs a tool: its runs would not compare with the earlier's.
#[test]
fn a_redo_is_refused_at_another_commit_or_on_another_machine() {
    let nowhere = Path::new("/nowhere");
    let executables = Executables {
        proving_ground: nowhere,
        foxhound: nowhere,
        spider_builds: nowhere,
        profile_folder: "release",
        crawlee: nowhere,
        scrapy: nowhere,
        foxhound_program: nowhere,
        browser: nowhere,
        collie: None,
        foxhound_settings: &[],
    };
    let options = Options {
        only: vec![(Scenario::Raw, "foxhound")],
        ..crate::compare::options(
            &crate::compare::tests::recipe_arguments(&[]),
            &crate::environment::Environment::of(&[]),
        )
        .expect("invariant: the recipe's arguments name no cell")
    };
    let refused = |earlier: Comparison, commit: &str| {
        redo(&executables, commit, &options, earlier, &mut |_| Ok(()))
            .map(|_| ())
            .map_err(|error| error.to_string())
    };
    let mut elsewhere = file();
    elsewhere.machine.cpu = "a CPU no host has".to_owned();
    let at = "0".repeat(40);

    assert_eq!(
        (refused(file(), &"1".repeat(40)), refused(elsewhere, &at)),
        (
            Err(format!(
                "the trials file's run was at commit {at}, not at this checkout's {}: a redo measures at the same commit",
                "1".repeat(40)
            )),
            Err("the trials file's run was on another machine (16 threads of a CPU no host has): a redo measures on the same machine".to_owned()),
        )
    );
}
