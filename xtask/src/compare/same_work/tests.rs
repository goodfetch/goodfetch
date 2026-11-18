use super::{check, strace_runs_of_other_work_set_aside};
use crate::compare::auto::Judged;
use crate::compare::extracted::{Differing, Extracted};
use crate::compare::matrix::{self, Row, Scenario};
use crate::compare::trials::{
    AutoRun, Comparison, CountedCell, CountedRun, INSTRUCTION_SCENARIOS, INSTRUCTION_TOOLS,
    Measured, Redone, SIZE_CONCURRENCIES, SIZES, SizesCell, Tool, Trial, Work, fixture,
    size_not_run, size_tools,
};

/// The work of a trial that fetched every page of the site of `pages`
/// pages once, 600 bytes each.
fn whole(pages: u64) -> Work {
    Work {
        pages,
        missing: Vec::new(),
        extra: Vec::new(),
        requests: pages,
        bytes: 600 * pages,
        first_bytes: 600 * pages,
        other_protocol: 0,
    }
}

/// The work of a trial that fetched every page of the 4-page site once,
/// 600 bytes each, plus `again` pages fetched a second time.
fn work(again: u64) -> Work {
    Work {
        requests: 4 + again,
        bytes: 600 * (4 + again),
        ..whole(4)
    }
}

/// `work` on the 4-page site, on `scenario`'s site where the comparison's
/// has 4 pages: on the multi-domain site, its other pages each fetched once
/// besides.
fn on_site(scenario: Scenario, work: Work) -> Work {
    let more = scenario.site_pages(4) - 4;
    Work {
        pages: work.pages + more,
        requests: work.requests + more,
        bytes: work.bytes + 600 * more,
        first_bytes: work.first_bytes + 600 * more,
        ..work
    }
}

/// Three trials of each measured cell over the 4-page site, one of a cell
/// that runs one ([`matrix::Cell::one_trial`]), each with the
/// work `work` gives its scenario and number; the long scenario's trials
/// crawl its own site, and each did all of its work.
fn comparison(work: impl Fn(Scenario, u32) -> Work) -> Comparison {
    fixture::comparison(4, |cell, number| Measured {
        seconds: 1.0,
        work: if cell.scenario == Scenario::Long {
            whole(matrix::LONG_PAGES)
        } else {
            on_site(cell.scenario, work(cell.scenario, number))
        },
        ..Measured::default()
    })
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

/// An auto cell's run did the same work when the Proving ground saw every
/// page of the rungs site's manifest requested: its Records are held to the
/// manifest as figures, pages wrong, never as other work (#2411).
#[test]
fn an_auto_run_that_left_a_page_of_the_manifest_unrequested_did_other_work() {
    let run = |cell: &str, number, requested, wrong| AutoRun {
        scenario: Scenario::Auto,
        cell: cell.to_owned(),
        tool: Tool::Spider,
        number,
        judged: Judged {
            seconds: 1.0,
            pages: 37,
            requested,
            rendered: 0,
            wrong,
        },
    };
    let mut trials = comparison(|_, _| work(0));
    trials.auto = vec![run("spider", 1, 37, 5), run("spider", 2, 36, 0)];

    assert_eq!(
        named(&trials),
        vec![("unrequested_pages", "auto spider run 2".to_owned())]
    );
}

/// pagesift's polite cell is held to one trial of each pass, timed and
/// under strace, the owner's decision of 2026-11-14 (#2215): one is the
/// whole cell, and a second is past it.
#[test]
fn pagesifts_polite_cell_is_held_to_one_trial_of_each_pass() {
    let mut trials = comparison(|_, _| work(0));
    trials.syscalls = counted(|_, _, _| true, |_, _, _| work(0));
    let one = named(&trials);
    let mut again = trials
        .trials
        .iter()
        .find(|trial| trial.scenario == Scenario::Polite && trial.cell == "pagesift")
        .cloned()
        .expect("invariant: the fixture holds a trial of pagesift's polite cell");
    again.number = 2;
    trials.trials.push(again);

    assert_eq!(
        (one, named(&trials)),
        (vec![], vec![("trial_count", "polite pagesift".to_owned())])
    );
}

/// The trials of `trials` of the cells `cells` names alone, each
/// `<scenario>/<cell>`: what a run that named them measured.
fn of_cells(trials: &mut Comparison, cells: &[&str]) {
    trials.trials.retain(|trial| {
        cells.contains(&format!("{}/{}", trial.scenario.name(), trial.cell).as_str())
    });
}

/// A run that names cells measures them alone (#2200), so the check counts
/// the trials of those alone: a cell it did not name ran none and is short
/// of nothing, while a named cell a trial short is still named (#2742).
#[test]
fn a_run_that_names_one_cell_counts_the_trials_of_that_cell_alone() {
    let mut trials = comparison(|_, _| work(0));
    trials.declared.cells = vec!["raw/foxhound".to_owned()];
    of_cells(&mut trials, &["raw/foxhound"]);
    let whole = named(&trials);
    trials.trials.pop();

    assert_eq!(
        (whole, named(&trials)),
        (vec![], vec![("trial_count", "raw foxhound".to_owned())])
    );
}

/// A run that names no cell measures every cell of its scenarios, and the
/// check counts every one: each cell with no trial is short of its
/// declared trials.
#[test]
fn a_run_that_names_no_cell_counts_every_measured_cell() {
    let mut trials = comparison(|_, _| work(0));
    of_cells(&mut trials, &["raw/foxhound"]);

    let findings = named(&trials);

    assert_eq!(
        (
            findings.len(),
            findings.iter().all(|(kind, _)| *kind == "trial_count")
        ),
        (matrix::measured().count() - 1, true)
    );
}

/// A redo of a run that named cells may measure others too: the check then
/// counts the cells the run named and those the redo measured again.
#[test]
fn a_redo_of_a_run_that_named_cells_counts_the_cells_it_measured_again() {
    let mut trials = comparison(|_, _| work(0));
    trials.declared.cells = vec!["raw/foxhound".to_owned()];
    trials.redone = vec![Redone {
        cells: vec!["raw/spider".to_owned()],
        order_seed: 11,
    }];
    of_cells(&mut trials, &["raw/foxhound", "raw/spider"]);
    let whole = named(&trials);
    trials.trials.pop();

    assert_eq!(
        (whole, named(&trials)),
        (vec![], vec![("trial_count", "raw spider".to_owned())])
    );
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

/// A crawl-like trial (#2284) that sent some of its requests for the site's
/// pages over anything but TLS and HTTP/2 did other work, though it
/// fetched the site's pages and its bytes match: it is named, and the
/// other trials are held to the work they share.
#[test]
fn a_crawl_like_trial_that_left_tls_and_http2_did_other_work() {
    let mut trials = comparison(|_, _| work(0));
    let trial = trials
        .trials
        .iter_mut()
        .find(|trial| {
            trial.scenario == Scenario::CrawlLike && trial.cell == "scrapy" && trial.number == 2
        })
        .expect("invariant: the fixture holds the crawl-like scenario's scrapy trials");
    trial.measured.work.other_protocol = 3;

    assert_eq!(
        named(&trials),
        vec![("other_protocol", "crawl-like scrapy trial 2".to_owned())]
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
            other_protocol: 0,
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
            "crawlee-rs",
            "pagesift",
            "openspider",
            "fastcrw",
            "crawlberg",
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
            ("different_bytes", "durable crawlee-rs trial 3".to_owned()),
            ("different_bytes", "durable pagesift trial 3".to_owned()),
        ]
    );
}

/// Foxhound's `auto` row fetches each sample's page twice, on `http` and in
/// a browser, so its trials are held to the pages first served and their
/// bytes (#2283): its extra requests are no other work, but other bytes for
/// a page's first answer are.
#[test]
fn an_auto_rows_samples_fetched_twice_are_no_other_work_but_other_first_bytes_are() {
    let mut trials = comparison(|_, _| work(0));
    for trial in &mut trials.trials {
        if trial.cell == "foxhound auto" {
            let work = &mut trial.measured.work;
            work.requests += 3000;
            work.bytes += 600 * 3000;
            if trial.number == 2 {
                work.first_bytes += 1;
            }
        }
    }

    assert_eq!(
        named(&trials),
        vec![("different_bytes", "polite foxhound auto trial 2".to_owned())]
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
                other_protocol: 0,
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
        [
            "foxhound",
            "spider",
            "crawlee",
            "scrapy",
            "pagesift",
            "openspider",
            "fastcrw",
            "crawlberg"
        ]
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

/// The handled crawl holds every tool to its pages' fields (#2317): a
/// competitor's trial whose Records differ from the answers is refused as
/// Foxhound's is, where the extraction scenario only labels it.
#[test]
fn a_handled_trial_of_any_tool_whose_fields_differ_is_refused() {
    let mut trials = comparison(|_, _| work(0));
    for trial in &mut trials.trials {
        if trial.scenario == Scenario::Handled && trial.number == 2 {
            trial.measured.extracted = Some(Extracted {
                whole: Differing {
                    pages: 1,
                    first: Some("/sites/4/3: name null, not \"Page 3\"".to_owned()),
                },
                ..Extracted::default()
            });
        }
    }

    assert_eq!(
        named(&trials),
        ["foxhound", "spider", "crawlee", "scrapy", "crawlee-rs"]
            .map(|cell| ("output_differs", format!("handled {cell} trial 2")))
    );
}

/// Three runs of each cell `counts` keeps, one of a cell that runs one
/// ([`matrix::Cell::one_trial`]), each of the work `work` gives
/// its scenario, cell and number: a counted cell over the 4-page site,
/// which a scenario that runs only its timed trials has no run in.
fn counted(
    counts: impl Fn(Scenario, Row, Tool) -> bool,
    work: impl Fn(Scenario, &str, u32) -> Work,
) -> CountedCell {
    let runs = matrix::measured()
        .filter(|(cell, config)| {
            !cell.scenario.timed_only() && counts(cell.scenario, cell.row, config.tool())
        })
        .flat_map(|(cell, config)| {
            let work = &work;
            (1..=cell.runs(3)).map(move |number| CountedRun {
                scenario: cell.scenario,
                cell: cell.name.to_owned(),
                tool: config.tool(),
                number,
                load: None,
                count: 9,
                syncs: 0,
                work: on_site(cell.scenario, work(cell.scenario, cell.name, number)),
                extracted: cell.output_check().map(|_| Extracted::default()),
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
        other_protocol: 0,
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
            INSTRUCTION_SCENARIOS.contains(&scenario)
                && row == Row::Main
                && INSTRUCTION_TOOLS.contains(&tool)
        },
        |scenario, cell, number| {
            if (scenario, cell, number) == (Scenario::Raw, "spider", 3) {
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
            ("different_pages", "polite foxhound auto trial 1".to_owned()),
            ("different_pages", "polite crawlee trial 1".to_owned()),
            ("different_pages", "polite scrapy trial 1".to_owned()),
            ("different_pages", "polite crawlee-rs trial 1".to_owned()),
            ("different_pages", "polite pagesift trial 1".to_owned()),
            ("different_pages", "raw spider callgrind trial 3".to_owned()),
        ]
    );
    let (_, stopped) = cells_of(&trials.syscalls);
    assert_eq!(stopped, vec![]);
}

/// Three trials of each scenario 1 main cell the sizes cell runs at each
/// size and concurrency, every tool's at 16 in flight and Foxhound's and
/// Spider's at 1 and 64, Spider's at 1 aside, which it does not run; each
/// fetching its site's pages once, the first at 100 pages and 64 in flight
/// short of a page.
fn sizes() -> SizesCell {
    let trials_run = matrix::measured()
        .filter(|(cell, _)| cell.scenario == Scenario::Raw && cell.row == Row::Main)
        .flat_map(|(cell, config)| {
            SIZES.into_iter().flat_map(move |pages| {
                SIZE_CONCURRENCIES
                    .into_iter()
                    .filter(move |&concurrency| {
                        size_tools(concurrency).contains(&config.tool())
                            && size_not_run(config.tool(), concurrency).is_none()
                    })
                    .flat_map(move |concurrency| {
                        (1..=3).map(move |number| {
                            let short = (pages, concurrency, number) == (100, 64, 1);
                            let fetched = if short { pages - 1 } else { pages };
                            Trial {
                                scenario: cell.scenario,
                                cell: cell.name.to_owned(),
                                tool: config.tool(),
                                number,
                                site_pages: pages,
                                concurrency,
                                load: None,
                                retried: None,
                                measured: Measured {
                                    seconds: 1.0,
                                    work: Work {
                                        pages: fetched,
                                        missing: if short {
                                            vec!["/sites/100/100".to_owned()]
                                        } else {
                                            Vec::new()
                                        },
                                        extra: Vec::new(),
                                        requests: fetched,
                                        bytes: 600 * fetched,
                                        first_bytes: 600 * fetched,
                                        other_protocol: 0,
                                    },
                                    ..Measured::default()
                                },
                            }
                        })
                    })
            })
        })
        .collect();
    SizesCell::Measured {
        trials: 3,
        trials_run,
        stopped: Vec::new(),
    }
}

/// The sizes cell's trials are held to their own size's site, each size
/// and concurrency apart: the trial short of a page at 100 pages and 64 in
/// flight is named there, and no point is short of its trials, every tool
/// at 16 in flight and Spider at 1, which runs none, included.
#[test]
fn a_sizes_trial_is_held_to_its_own_sizes_site() {
    let mut trials = comparison(|_, _| work(0));
    trials.sizes = sizes();

    assert_eq!(
        named(&trials),
        vec![
            (
                "different_pages",
                "raw foxhound sizes 100 pages at 64 trial 1".to_owned()
            ),
            (
                "different_pages",
                "raw spider sizes 100 pages at 64 trial 1".to_owned()
            ),
        ]
    );
}
