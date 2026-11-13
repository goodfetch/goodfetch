use super::check;
use crate::compare::matrix::{self, Scenario};
use crate::compare::trials::{Comparison, Measured, Work, fixture};

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
    fixture::comparison(4, |cell, number| Measured {
        seconds: 1.0,
        work: work(cell.scenario, number),
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
