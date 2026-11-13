use super::{check, strace_runs_of_other_work_set_aside};
use crate::compare::matrix::{self, Row, Scenario};
use crate::compare::trials::{
    Comparison, CountedCell, CountedRun, Declared, GrowthCell, INSTRUCTION_TOOLS, Machine,
    Measured, Tool, Trial, Work,
};

/// The work of a trial that fetched every page of the 4-page site once,
/// 600 bytes each, plus `again` pages fetched a second time.
fn work(again: u64) -> Work {
    Work {
        pages: 4,
        missing: Vec::new(),
        extra: Vec::new(),
        requests: 4 + again,
        bytes: 600 * (4 + again),
        first_bytes: 2400,
    }
}

/// Three trials of each measured cell over the 4-page site, each with the
/// work `work` gives its scenario and number.
fn comparison(work: impl Fn(Scenario, u32) -> Work) -> Comparison {
    let trials = matrix::measured()
        .flat_map(|(cell, config)| {
            let work = &work;
            (1..=3).map(move |number| Trial {
                scenario: cell.scenario,
                cell: cell.name.to_owned(),
                tool: config.tool(),
                number,
                site_pages: 4,
                load: None,
                retried: None,
                measured: Measured {
                    seconds: 1.0,
                    work: work(cell.scenario, number),
                    ..Measured::default()
                },
            })
        })
        .collect();
    Comparison {
        declared: Declared {
            trials: 3,
            statistic: "median".to_owned(),
            concurrency: 16,
            site: "/sites/4/1".to_owned(),
            spider: "2.53.9".to_owned(),
            crawlee: "3.18.2".to_owned(),
            scrapy: "2.19.0".to_owned(),
            playwright: "1.64.1".to_owned(),
            scrapy_playwright: "0.0.48".to_owned(),
            browser: "156.0.8078.4".to_owned(),
            scenarios: Scenario::ALL.to_vec(),
        },
        machine: Machine {
            class: "local-controlled".to_owned(),
            os: "linux".to_owned(),
            arch: "x86_64".to_owned(),
            cpu: "a CPU".to_owned(),
            threads: 16,
            load: None,
        },
        commit: "0".repeat(40),
        order_seed: 7,
        trials,
        stopped: Vec::new(),
        syscalls: CountedCell::NotRun {
            reason: "a test".to_owned(),
        },
        growth: GrowthCell::NotRun {
            reason: "a test".to_owned(),
        },
        instructions: CountedCell::NotRun {
            reason: "a test".to_owned(),
        },
    }
}

/// Each finding's kind and subject.
fn named(trials: &Comparison) -> Vec<(&'static str, String)> {
    check(trials)
        .into_iter()
        .map(|finding| (finding.kind, finding.subject))
        .collect()
}

#[test]
fn trials_that_did_the_same_work_pass() {
    assert_eq!(named(&comparison(|_, _| work(0))), vec![]);
}

/// The first trial is held to the site's own pages like any other, so
/// when it is the one that did other work, it alone is named.
#[test]
fn a_first_trial_that_fetched_other_pages_is_the_one_named() {
    let mut trials = comparison(|_, _| work(0));
    let first = &mut trials.trials[0].measured.work;
    first.pages = 3;
    first.missing = vec!["/sites/4/4".to_owned()];

    assert_eq!(
        named(&trials),
        vec![("different_pages", "raw foxhound trial 1".to_owned())]
    );
}

/// Bytes are held to what most trials were served, so a first trial
/// served other bytes for the site's pages is the one named.
#[test]
fn a_first_trial_served_other_bytes_is_the_one_named() {
    let mut trials = comparison(|_, _| work(0));
    trials.trials[0].measured.work.bytes += 1;

    assert_eq!(
        named(&trials),
        vec![("different_bytes", "raw foxhound trial 1".to_owned())]
    );
}

/// Every trial that misses the same page is named, though they agree with
/// each other.
#[test]
fn trials_that_all_miss_a_page_are_each_named() {
    let trials = comparison(|scenario, _| match scenario {
        Scenario::Raw => Work {
            pages: 3,
            missing: vec!["/sites/4/4".to_owned()],
            extra: Vec::new(),
            requests: 3,
            bytes: 1800,
            first_bytes: 1800,
        },
        _ => work(0),
    });

    let raw: Vec<(&'static str, String)> = matrix::measured()
        .filter(|(cell, _)| cell.scenario == Scenario::Raw)
        .flat_map(|(cell, _)| {
            (1..=3).map(move |number| {
                (
                    "different_pages",
                    format!("raw {} trial {number}", cell.name),
                )
            })
        })
        .collect();
    assert_eq!(named(&trials), raw);
}

/// A resumed crawl fetches the pages in flight at its kill again: as many
/// as it was killed with, which varies, and is no other work.
#[test]
fn a_resumed_crawls_pages_fetched_again_are_no_other_work() {
    let trials = comparison(|scenario, number| match scenario {
        Scenario::Durable => work(u64::from(number)),
        _ => work(0),
    });

    assert_eq!(named(&trials), vec![]);
}

/// Without a kill, a page fetched twice is other work.
#[test]
fn a_raw_trial_that_fetched_a_page_twice_did_other_work() {
    let trials = comparison(|scenario, number| match (scenario, number) {
        (Scenario::Raw, 2) => work(1),
        _ => work(0),
    });

    assert_eq!(
        named(&trials),
        [
            "foxhound",
            "spider",
            "crawlee",
            "scrapy",
            "spider capacity 4096",
            "spider default features",
            "spider bloom",
            "spider jemalloc",
        ]
        .map(|cell| ("different_bytes", format!("raw {cell} trial 2")))
    );
}

/// The bytes every trial is held to are what most main rows' trials were
/// served: Spider's sensitivity rows, more than the other tools' rows
/// together in scenario 1, add no vote, so when every Spider trial is
/// served other bytes, Spider's are the ones named.
#[test]
fn sensitivity_rows_do_not_vote_for_the_work_trials_are_held_to() {
    let mut trials = comparison(|_, _| work(0));
    for trial in &mut trials.trials {
        if trial.scenario == Scenario::Raw && trial.tool == matrix::Tool::Spider {
            trial.measured.work.bytes += 1;
        }
    }

    let mut cells: Vec<String> = named(&trials)
        .into_iter()
        .map(|(kind, subject)| format!("{kind} {}", &subject[..subject.len() - " trial 1".len()]))
        .collect();
    cells.dedup();
    assert_eq!(
        cells,
        [
            "spider",
            "spider capacity 4096",
            "spider default features",
            "spider bloom",
            "spider jemalloc",
        ]
        .map(|cell| format!("different_bytes raw {cell}"))
    );
}

/// A resumed crawl served other bytes for its pages' first answers did
/// other work.
#[test]
fn a_resumed_trial_whose_pages_were_first_served_other_bytes_did_other_work() {
    let trials = comparison(|scenario, number| {
        let mut work = work(0);
        if (scenario, number) == (Scenario::Durable, 3) {
            work.first_bytes += 1;
        }
        work
    });

    assert_eq!(
        named(&trials),
        vec![
            (
                "different_bytes",
                "durable foxhound full trial 3".to_owned()
            ),
            (
                "different_bytes",
                "durable foxhound periodic trial 3".to_owned()
            ),
        ]
    );
}

/// In the headless browser scenario, a trial that reached the site's
/// first page alone never ran the script that links the others: the check
/// names it for the script-rendered pages it missed.
#[test]
fn a_browser_trial_that_never_rendered_the_links_misses_the_script_rendered_pages() {
    let trials = comparison(|scenario, number| {
        if (scenario, number) == (Scenario::Browser, 2) {
            Work {
                pages: 1,
                missing: (2..=4).map(|page| format!("/sites/4/{page}")).collect(),
                extra: Vec::new(),
                requests: 1,
                bytes: 600,
                first_bytes: 600,
            }
        } else {
            work(0)
        }
    });

    let findings = check(&trials);

    assert_eq!(
        findings
            .iter()
            .map(|finding| (
                finding.kind,
                finding.subject.as_str(),
                finding.message.as_str()
            ))
            .collect::<Vec<_>>(),
        ["foxhound", "spider", "crawlee", "scrapy"]
            .map(|cell| (
                "unrendered_pages",
                format!("browser {cell} trial 2"),
                "3 of the site's 3 pages its script links not reached",
            ))
            .iter()
            .map(|(kind, subject, message)| (*kind, subject.as_str(), *message))
            .collect::<Vec<_>>()
    );
}

/// Three runs of each cell `counts` keeps, each of the work `work` gives
/// its scenario, cell and number: a counted cell over the 4-page site.
fn counted(
    counts: impl Fn(Scenario, Row, Tool) -> bool,
    work: impl Fn(Scenario, &str, u32) -> Work,
) -> CountedCell {
    let runs = matrix::measured()
        .filter(|(cell, config)| counts(cell.scenario, cell.row, config.tool()))
        .flat_map(|(cell, config)| {
            let work = &work;
            (1..=3).map(move |number| CountedRun {
                scenario: cell.scenario,
                cell: cell.name.to_owned(),
                tool: config.tool(),
                number,
                count: 9,
                syncs: 0,
                work: work(cell.scenario, cell.name, number),
            })
        })
        .collect();
    CountedCell::Measured {
        trials: 3,
        runs,
        stopped: Vec::new(),
    }
}

/// The work of a run that reached 3 of the 4-page site's pages.
fn short() -> Work {
    Work {
        pages: 3,
        missing: vec!["/sites/4/4".to_owned()],
        extra: Vec::new(),
        requests: 3,
        bytes: 1800,
        first_bytes: 1800,
    }
}

/// A stopped run's scenario, cell, number and reason.
type Stopped<'a> = (Scenario, &'a str, u32, &'a str);

/// The cells a counted cell's runs are of, and its stopped runs.
fn cells_of(cell: &CountedCell) -> (Vec<(Scenario, &str)>, Vec<Stopped<'_>>) {
    let CountedCell::Measured { runs, stopped, .. } = cell else {
        panic!("the cell ran");
    };
    let mut cells: Vec<_> = runs
        .iter()
        .map(|run| (run.scenario, run.cell.as_str()))
        .collect();
    cells.dedup();
    let stopped = stopped
        .iter()
        .map(|run| {
            (
                run.scenario,
                run.cell.as_str(),
                run.number,
                run.reason.as_str(),
            )
        })
        .collect();
    (cells, stopped)
}

/// strace slows a headless browser crawl enough for it to miss pages: the
/// run's system calls are of other work, so its cell's are not measured,
/// with the pages it reached, and the timed figures still count.
#[test]
fn a_strace_run_that_missed_pages_leaves_its_cells_system_calls_not_measured() {
    let mut trials = comparison(|_, _| work(0));
    trials.syscalls = counted(
        |_, _, _| true,
        |scenario, cell, number| {
            if (scenario, cell, number) == (Scenario::Browser, "spider", 2) {
                short()
            } else {
                work(0)
            }
        },
    );

    strace_runs_of_other_work_set_aside(&mut trials);

    assert_eq!(named(&trials), vec![]);
    let (cells, stopped) = cells_of(&trials.syscalls);
    assert!(!cells.contains(&(Scenario::Browser, "spider")), "{cells:?}");
    assert!(cells.contains(&(Scenario::Browser, "crawlee")), "{cells:?}");
    assert_eq!(
        stopped,
        vec![(
            Scenario::Browser,
            "spider",
            2,
            "under `strace -f`, run 2 did other work, reaching 3 of the site's 4 pages: 1 of the site's 3 pages its script links not reached"
        )]
    );
}

/// A strace run served other bytes for the site's pages counted other
/// work's system calls too.
#[test]
fn a_strace_run_served_other_bytes_leaves_its_cells_system_calls_not_measured() {
    let mut trials = comparison(|_, _| work(0));
    trials.syscalls = counted(
        |_, _, _| true,
        |scenario, cell, number| {
            work(u64::from(
                (scenario, cell, number) == (Scenario::Raw, "crawlee", 1),
            ))
        },
    );

    strace_runs_of_other_work_set_aside(&mut trials);

    assert_eq!(named(&trials), vec![]);
    let (cells, stopped) = cells_of(&trials.syscalls);
    assert!(!cells.contains(&(Scenario::Raw, "crawlee")), "{cells:?}");
    assert_eq!(stopped.len(), 1, "{stopped:?}");
    let (scenario, cell, number, reason) = stopped[0];
    assert_eq!((scenario, cell, number), (Scenario::Raw, "crawlee", 1));
    assert!(
        reason.starts_with(
            "under `strace -f`, run 1 did other work, reaching 4 of the site's 4 pages: 5 requests served 3000 bytes, where "
        ),
        "{reason}"
    );
}

/// Only strace's cell is set aside: a timed trial and a Callgrind run that
/// missed a page still keep every figure from counting.
#[test]
fn a_timed_trial_and_a_callgrind_run_of_other_work_still_fail_the_check() {
    let mut trials = comparison(|scenario, number| {
        if (scenario, number) == (Scenario::Polite, 1) {
            short()
        } else {
            work(0)
        }
    });
    trials.syscalls = counted(|_, _, _| true, |_, _, _| work(0));
    trials.instructions = counted(
        |scenario, row, tool| {
            scenario == Scenario::Raw && row == Row::Main && INSTRUCTION_TOOLS.contains(&tool)
        },
        |_, cell, number| {
            if (cell, number) == ("spider", 3) {
                short()
            } else {
                work(0)
            }
        },
    );

    strace_runs_of_other_work_set_aside(&mut trials);

    assert_eq!(
        named(&trials),
        vec![
            ("different_pages", "polite foxhound trial 1".to_owned()),
            ("different_pages", "polite crawlee trial 1".to_owned()),
            ("different_pages", "polite scrapy trial 1".to_owned()),
            ("different_pages", "raw spider callgrind trial 3".to_owned()),
        ]
    );
    let (_, stopped) = cells_of(&trials.syscalls);
    assert_eq!(stopped, vec![]);
}
