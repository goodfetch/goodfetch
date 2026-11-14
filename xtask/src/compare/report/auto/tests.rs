use crate::compare::auto::Judged;
use crate::compare::matrix::{Scenario, Tool};
use crate::compare::report::{markdown, of};
use crate::compare::trials::{AutoRun, Measured, fixture};

/// One run of the auto cell `cell` over the 37 pages of the rungs site's
/// manifest, every page requested.
fn run(
    cell: &str,
    tool: Tool,
    number: u32,
    (seconds, rendered, wrong): (f64, u64, u64),
) -> AutoRun {
    AutoRun {
        scenario: Scenario::Auto,
        cell: cell.to_owned(),
        tool,
        number,
        judged: Judged {
            seconds,
            pages: 37,
            requested: 37,
            rendered,
            wrong,
        },
    }
}

/// The lines under `cell` in the auto scenario's mapping.
fn lines<'a>(section: &'a str, cell: &str) -> Vec<&'a str> {
    section
        .lines()
        .skip_while(|line| !line.starts_with(&format!("- **{cell}**")))
        .skip(1)
        .take_while(|line| line.starts_with("  "))
        .collect()
}

/// Each auto cell gives, over its runs, its rate, the pages it rendered and
/// the pages whose Records were wrong, each the median and range of its
/// runs, beside the same work its runs did; it carries no bar (G-04 Q11),
/// and gets no standing and no metric table. A cell no run of which is in
/// the trials file is not measured.
#[test]
fn an_auto_cell_gives_its_rate_pages_rendered_and_pages_wrong_with_no_bar() {
    let mut trials = fixture::comparison(1000, |_, _| Measured::default());
    trials.auto = vec![
        run("spider", Tool::Spider, 1, (3.7, 10, 0)),
        run("spider", Tool::Spider, 2, (1.85, 12, 2)),
        run("spider", Tool::Spider, 3, (7.4, 11, 1)),
        run("foxhound", Tool::Foxhound, 1, (1.0, 9, 0)),
    ];

    let report = of(&trials);
    let markdown = markdown(&report, &trials);

    let section = super::super::tests::section(&markdown, Scenario::Auto.title());
    assert_eq!(
        (lines(section, "spider"), lines(section, "foxhound")),
        (
            vec![
                "  - no bar: none is set on this cell until the owner sets one (#2411, G-04 Q11).",
                "  - same work: 3 runs, each requested all 37 pages of the rungs site's manifest.",
                "  - rate 10.0 (5.0–20.0) pages/s, pages rendered 11 (10–12) and pages wrong 1 \
                 (0–2) of the 37, each the median (lowest–highest) of its runs.",
            ],
            vec![
                "  - no bar: none is set on this cell until the owner sets one (#2411, G-04 Q11).",
                "  - same work: 1 run, each requested all 37 pages of the rungs site's manifest.",
                "  - rate 37.0 (37.0–37.0) pages/s, pages rendered 9 (9–9) and pages wrong 0 \
                 (0–0) of the 37, each the median (lowest–highest) of its runs.",
            ],
        )
    );
    let auto = report
        .scenarios
        .iter()
        .find(|scenario| scenario.scenario == Scenario::Auto)
        .expect("invariant: the fixture declares every scenario");
    assert_eq!(
        (
            auto.standings.len(),
            section.contains("| Metric |"),
            lines(section, "crawlee")
                .contains(&"  - not measured: no run of it is in the trials file.")
        ),
        (0, false, true)
    );
}
