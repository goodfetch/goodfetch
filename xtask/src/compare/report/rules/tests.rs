use crate::compare::hostile::Broken;
use crate::compare::matrix::{Scenario, Tool};
use crate::compare::report::{markdown, of};
use crate::compare::trials::{Measured, RuledRun, fixture};

/// `traps/1`'s counts in one run: its requests and those refused, honeypot
/// requests, items fetched, duplicates and trap requests, and why it was
/// stopped, if it was.
fn traps(
    (requests, refused): (u64, u64),
    honeypot: u64,
    wanted_fetched: u64,
    (duplicate, trap): (u64, u64),
    stopped: Option<&str>,
) -> Broken {
    Broken {
        task: "traps/1".to_owned(),
        requests,
        retry_after_ignored: 0,
        over_in_flight_cap: 0,
        honeypot,
        flagged: honeypot > 0,
        refused,
        wanted_fetched,
        wanted: 30,
        duplicate,
        trap,
        stopped: stopped.map(str::to_owned),
    }
}

/// `limits/1`'s counts in one run: requests inside a `Retry-After` pause.
fn limits(retry_after_ignored: u64) -> Broken {
    Broken {
        task: "limits/1".to_owned(),
        requests: 40,
        retry_after_ignored,
        over_in_flight_cap: 0,
        honeypot: 0,
        flagged: retry_after_ignored >= 3,
        refused: 0,
        wanted_fetched: 24,
        wanted: 24,
        duplicate: 0,
        trap: 0,
        stopped: None,
    }
}

/// Each hostile pages' cell gives, for each task of its pass, the median
/// and range of each count over its runs, in how many runs the address
/// ended flagged, the requests refused, its coverage and, where the family
/// records it, its waste over the requests answered,
/// and the runs the comparison stopped, and says a task some run gave no
/// counts of is not measured; it carries no bar until the owner sets one
/// (#2439, Q3), and gets no standing and no metric table.
#[test]
fn a_hostile_cell_gives_each_tasks_counts_over_its_runs_with_no_bar() {
    let mut trials = fixture::comparison(1000, |_, _| Measured::default());
    trials.rules = [
        (1, traps((100, 0), 0, 30, (5, 5), None), limits(0)),
        (2, traps((120, 0), 1, 30, (6, 6), None), limits(2)),
        (
            3,
            traps(
                (1000, 500),
                2,
                27,
                (200, 250),
                Some("stopped after 1000 requests"),
            ),
            limits(5),
        ),
    ]
    .into_iter()
    .map(|(number, traps, limits)| RuledRun {
        scenario: Scenario::Hostile,
        cell: "spider".to_owned(),
        tool: Tool::Spider,
        number,
        tasks: vec![traps, limits],
    })
    .collect();

    let report = of(&trials);
    let markdown = markdown(&report, &trials);

    let section = super::super::tests::section(&markdown, Scenario::Hostile.title());
    let spider: Vec<&str> = section
        .lines()
        .skip_while(|line| !line.starts_with("- **spider**"))
        .skip(1)
        .take_while(|line| line.starts_with("  "))
        .collect();
    assert_eq!(
        spider,
        [
            "  - no bar: none is set on this cell until the owner sets one (#2439, Q3).",
            "  - rules broken over 3 runs, each figure the median (lowest–highest) of its runs:",
            "    - `traps/1`: 120 requests (100–1000), `retry_after_ignored` 0 (0–0), \
             `over_in_flight_cap` 0 (0–0), `honeypot` 1 (0–2); the address ended flagged in 2 of \
             3 runs, refused 0 (0–500) requests; coverage 100.0 % (90.0 %–100.0 %) of its 30 \
             wanted pages; waste 10.0 % (10.0 %–90.0 %) of the requests answered, duplicates and \
             trap requests; stopped in 1 of 3 runs: run 3, stopped after 1000 requests.",
            "    - `limits/1`: 40 requests (40–40), `retry_after_ignored` 2 (0–5), \
             `over_in_flight_cap` 0 (0–0), `honeypot` 0 (0–0); the address ended flagged in 1 of \
             3 runs, refused 0 (0–0) requests; coverage 100.0 % (100.0 %–100.0 %) of its 24 \
             wanted pages.",
            "    - `limits/3`: not measured: a run gave no count of this task.",
        ]
    );
    let hostile = report
        .scenarios
        .iter()
        .find(|scenario| scenario.scenario == Scenario::Hostile)
        .expect("invariant: the fixture declares every scenario");
    assert_eq!(
        (
            hostile.standings.len(),
            section.contains("| Metric |"),
            section.contains("  - not measured: no run of it is in the trials file.")
        ),
        (0, false, true)
    );
}
