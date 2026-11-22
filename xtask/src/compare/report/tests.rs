use super::standing::Standing;
use super::{CellStatus, Figure, Report, markdown, of};
use crate::compare::matrix::{HANDLED_GETTERS_PENDING, HANDLED_RUST_PENDING, Scenario};
use crate::compare::trials::{Comparison, Measured, Work, fixture};

/// The pages of the site every trial crawls.
const PAGES: u64 = 1000;

/// The work of a trial that fetched every page of the site once.
fn work() -> Work {
    Work {
        pages: PAGES,
        missing: Vec::new(),
        extra: Vec::new(),
        requests: PAGES,
        bytes: 600 * PAGES,
        first_bytes: 600 * PAGES,
        other_protocol: 0,
    }
}

/// Three trials of each measured cell over the 1,000-page site, each at the
/// pages per second `rates` gives its cell, 1,000 for a cell it does not
/// name.
fn comparison(rates: &[(Scenario, &str, [f64; 3])]) -> Comparison {
    fixture::comparison(PAGES, |cell, number| {
        let rate = rates
            .iter()
            .find(|(scenario, name, _)| *scenario == cell.scenario && *name == cell.name)
            .map_or(1000.0, |(_, _, rates)| rates[number as usize - 1]);
        Measured {
            seconds: 1000.0 / rate,
            work: work(),
            ..Measured::default()
        }
    })
}

/// The standing on `metric` of the `product` cell of `scenario` against its
/// `competitor` cell.
fn standing(
    report: &Report<'_>,
    scenario: Scenario,
    product: &str,
    competitor: &str,
    metric: &str,
) -> Option<Standing> {
    report
        .scenarios
        .iter()
        .find(|report| report.scenario == scenario)?
        .standings
        .iter()
        .find(|standings| standings.product == product && standings.competitor == competitor)?
        .metrics
        .get(metric)
        .copied()
}

/// The section of `title` in the Markdown, to the next section.
pub(super) fn section<'a>(markdown: &'a str, title: &str) -> &'a str {
    let start = markdown
        .find(&format!("\n## {title}"))
        .unwrap_or_else(|| panic!("no section {title} in:\n{markdown}"));
    let rest = &markdown[start + 1..];
    rest.find("\n## ").map_or(rest, |end| &rest[..end])
}

/// The rows of a section's standing table: each metric's label and its
/// standing against each competitor, in the columns' order.
fn standing_rows(section: &str) -> Vec<Vec<&str>> {
    section
        .lines()
        .skip_while(|line| !line.starts_with("| Standing |"))
        .skip(2)
        .take_while(|line| line.starts_with('|'))
        .map(|line| {
            line.trim_matches('|')
                .split(" | ")
                .map(str::trim)
                .collect::<Vec<_>>()
        })
        .collect()
}

/// The cells of a section's mapping whose lines include `line`.
fn cells_saying<'a>(section: &'a str, line: &str) -> Vec<&'a str> {
    let mut cells = Vec::new();
    let mut current = None;
    for text in section.lines() {
        if let Some(rest) = text.strip_prefix("- **") {
            current = rest.split("**").next();
        } else if text == line
            && let Some(cell) = current
        {
            cells.push(cell);
        }
    }
    cells
}

/// `ahead` when every trial of the Product is better than every trial of the
/// competitor, `behind` when every trial is worse, otherwise `overlapping`
/// with both medians and both ranges: scenario 1's Foxhound against a slower
/// Spider and a faster Crawlee, and the headless browser cell of
/// 2026-11-06, whose ranges cross.
#[test]
fn three_sets_of_trials_read_ahead_behind_and_overlapping() {
    let trials = comparison(&[
        (Scenario::Raw, "foxhound", [20.0, 21.0, 22.0]),
        (Scenario::Raw, "spider", [10.0, 11.0, 12.0]),
        (Scenario::Raw, "crawlee", [30.0, 31.0, 32.0]),
        (Scenario::Browser, "foxhound", [11.0, 11.5, 12.0]),
        (Scenario::Browser, "spider", [6.8, 15.0, 28.9]),
    ]);

    let report = of(&trials);
    let markdown = markdown(&report, &trials);

    let rate = "pages_per_second";
    assert_eq!(
        (
            standing(&report, Scenario::Raw, "foxhound", "spider", rate),
            standing(&report, Scenario::Raw, "foxhound", "crawlee", rate),
            standing(&report, Scenario::Browser, "foxhound", "spider", rate),
        ),
        (
            Some(Standing::Ahead),
            Some(Standing::Behind),
            Some(Standing::Overlapping)
        )
    );
    let raw = standing_rows(section(&markdown, "1. "));
    let browser = standing_rows(section(&markdown, "5. "));
    assert_eq!(
        (
            raw.iter()
                .find(|row| row[0] == "Throughput: pages per second"),
            browser
                .iter()
                .find(|row| row[0] == "Throughput: pages per second"),
        ),
        (
            Some(&vec![
                "Throughput: pages per second",
                "ahead",
                "behind",
                "behind",
                "behind",
                "behind",
                "behind",
                "behind",
                "behind"
            ]),
            Some(&vec![
                "Throughput: pages per second",
                "overlapping: foxhound 11.5 (11.0–12.0), spider 15.0 (6.8–28.9)",
                "behind",
                "behind",
                "behind",
                "behind",
                "behind",
                "behind"
            ]),
        )
    );
    let words: Vec<&str> = [raw, browser]
        .iter()
        .flatten()
        .flat_map(|row| &row[1..])
        .map(|text| text.split(':').next().unwrap_or_default())
        .collect();
    assert!(
        words
            .iter()
            .all(|word| ["ahead", "behind", "overlapping", "no standing"].contains(word)),
        "{words:?}"
    );
}

/// Sensitivity rows are no competitor of their own: the standings are
/// against each other tool's main row.
#[test]
fn standings_are_against_each_competitors_main_row() {
    let trials = comparison(&[]);

    let report = of(&trials);

    let raw = report
        .scenarios
        .iter()
        .find(|report| report.scenario == Scenario::Raw)
        .expect("invariant: the comparison runs scenario 1");
    assert_eq!(
        raw.standings
            .iter()
            .map(|standings| (standings.product, standings.competitor))
            .collect::<Vec<_>>(),
        [
            ("foxhound", "spider"),
            ("foxhound", "crawlee"),
            ("foxhound", "scrapy"),
            ("foxhound", "crawlee-rs"),
            ("foxhound", "pagesift"),
            ("foxhound", "openspider"),
            ("foxhound", "fastcrw"),
            ("foxhound", "crawlberg")
        ]
    );
}

/// The header names the machine as given to the run, with its memory
/// beside its OS, CPU and threads, tells the reader to compare its figures
/// within the report only, gives the command that reruns it, and says once
/// that it counts no false success.
#[test]
fn the_header_names_the_machine_and_the_command_that_reruns_it() {
    let mut trials = comparison(&[]);
    trials.machine.name = Some("Hetzner Cloud CCX43, 16 dedicated vCPUs".to_owned());
    trials.machine.memory_bytes = Some(64 * 1024 * 1024 * 1024);
    trials.declared.command =
        "just compare release --machine 'Hetzner Cloud CCX43, 16 dedicated vCPUs'".to_owned();

    let markdown = markdown(&of(&trials), &trials);

    let lines: Vec<&str> = markdown.lines().take(5).collect();
    assert_eq!(
        lines,
        [
            "# Comparison matrix",
            "",
            "`local-controlled`, measured on Hetzner Cloud CCX43, 16 dedicated vCPUs: linux 6.8.0 \
             x86_64, AMD EPYC-Milan Processor, 16 threads, 64.0 GiB of memory, at commit \
             `0000000000000000000000000000000000000000`. Compare its figures within this report, \
             never with a report measured on another machine.",
            "",
            "Rerun it with `just compare release --machine 'Hetzner Cloud CCX43, 16 dedicated \
             vCPUs'` at that commit.",
        ]
    );
    assert_eq!(
        (
            markdown
                .matches("It gives no false-success count: it reads no tool's own report")
                .count(),
            markdown.contains("never published"),
        ),
        (1, false)
    );
}

/// A run given no name, on an OS that gave no memory, says so.
#[test]
fn a_run_given_no_name_says_machine_not_named() {
    let trials = comparison(&[]);

    let markdown = markdown(&of(&trials), &trials);

    assert_eq!(
        markdown.lines().nth(2),
        Some(
            "`local-controlled`, measured on a machine not named (`--machine`): linux 6.8.0 \
             x86_64, AMD EPYC-Milan Processor, 16 threads, memory not read, at commit \
             `0000000000000000000000000000000000000000`. Compare its figures within this report, \
             never with a report measured on another machine."
        )
    );
}

/// Each cell of #2316's scenarios says it carries no bar, and each measured
/// one gives its rate, latency percentiles and peak memory; no cell of the
/// matrix's own five says so, and the hostile pages' cells say it for
/// #2439, the crawl-like scenario's for #2284, the auto cells for #2411
/// and the handled cell's for #2317 (their own tests). The cell at the catalog's defaults says it is
/// pending #2175 and gives no figure.
#[test]
fn the_new_scenarios_cells_carry_no_bar_and_give_their_figures() {
    let trials = comparison(&[]);

    let report = of(&trials);
    let markdown = markdown(&report, &trials);

    let no_bar = "  - no bar: none is set on this cell until the owner sets one (#2316).";
    let saying: Vec<(Scenario, Vec<&str>)> = report
        .scenarios
        .iter()
        .map(|scenario| {
            (
                scenario.scenario,
                cells_saying(section(&markdown, scenario.title), no_bar),
            )
        })
        .collect();
    let every = |scenario: Scenario| -> Vec<&str> {
        report
            .scenarios
            .iter()
            .find(|report| report.scenario == scenario)
            .map(|report| report.cells.iter().map(|cell| cell.cell).collect())
            .unwrap_or_default()
    };
    let figures = [
        "pages_per_second",
        "latency_p50_seconds",
        "latency_p95_seconds",
        "latency_p99_seconds",
        "peak_rss_bytes",
    ];
    let unmeasured: Vec<(Scenario, &str, &str)> = report
        .scenarios
        .iter()
        .filter(|scenario| scenario.scenario.no_bar().is_some())
        .flat_map(|scenario| {
            scenario
                .cells
                .iter()
                .flat_map(move |cell| match &cell.status {
                    CellStatus::Measured { metrics, .. } => figures
                        .iter()
                        .filter(|name| !matches!(metrics.get(*name), Some(Figure::Measured { .. })))
                        .map(|name| (scenario.scenario, cell.cell, *name))
                        .collect::<Vec<_>>(),
                    CellStatus::NotSupported { .. }
                    | CellStatus::NotMeasured { .. }
                    | CellStatus::Ruled { .. }
                    | CellStatus::Auto { .. } => Vec::new(),
                })
        })
        .collect();
    let pending: Vec<(Scenario, &str)> = report
        .scenarios
        .iter()
        .flat_map(|scenario| {
            scenario
                .cells
                .iter()
                .filter_map(move |cell| match cell.status {
                    CellStatus::NotMeasured { .. }
                        if cell.cell.starts_with("foxhound defaults") =>
                    {
                        Some((scenario.scenario, cell.cell))
                    }
                    _ => None,
                })
        })
        .collect();

    assert_eq!(
        (saying, unmeasured, pending),
        (
            Scenario::ALL
                .into_iter()
                .map(|scenario| (
                    scenario,
                    if scenario.no_bar().is_some()
                        && !scenario.hostile()
                        && !matches!(
                            scenario,
                            Scenario::CrawlLike | Scenario::Auto | Scenario::Handled
                        )
                    {
                        every(scenario)
                    } else {
                        Vec::new()
                    }
                ))
                .collect::<Vec<_>>(),
            Vec::new(),
            Vec::new(),
        )
    );
}

/// Each cell of the crawl-like scenario says it carries no bar, for
/// #2284.
#[test]
fn the_crawl_like_scenarios_cells_carry_no_bar() {
    let trials = comparison(&[]);

    let report = of(&trials);
    let markdown = markdown(&report, &trials);
    let crawl_like = section(&markdown, Scenario::CrawlLike.title());

    assert_eq!(
        cells_saying(
            crawl_like,
            "  - no bar: none is set on this cell until the owner sets one (#2284)."
        ),
        [
            "foxhound",
            "spider",
            "crawlee",
            "scrapy",
            "crawlee-rs",
            "pagesift",
            "openspider",
            "fastcrw",
            "crawlberg"
        ]
    );
}

/// Each cell of the handled crawl says it carries no bar, for #2317; its
/// Rust and getters rows say what they are pending, and Foxhound's mapping
/// says it sets no tier, so the engine chooses one (#1602).
#[test]
fn the_handled_cells_carry_no_bar_and_name_what_two_wait_on() {
    let trials = comparison(&[]);

    let markdown = markdown(&of(&trials), &trials);
    let handled = section(&markdown, Scenario::Handled.title());

    assert_eq!(
        (
            cells_saying(
                handled,
                "  - no bar: none is set on this cell until the owner sets one (#2317)."
            ),
            [HANDLED_RUST_PENDING, HANDLED_GETTERS_PENDING]
                .map(|why| { cells_saying(handled, &format!("  - not measured: pending {why}.")) }),
            handled.contains("`fetch.tier` not set, so the engine's own tier choice"),
        ),
        (
            vec![
                "foxhound",
                "foxhound rust",
                "foxhound getters",
                "spider",
                "crawlee",
                "scrapy",
                "crawlee-rs"
            ],
            [vec!["foxhound rust"], vec!["foxhound getters"]],
            true,
        )
    );
}

/// Wherever a cell's pages per second are printed, the pages the Proving
/// ground saw it fetch, of the site's, are printed beside them.
#[test]
fn the_pages_fetched_are_beside_each_rate() {
    let trials = comparison(&[]);

    let markdown = markdown(&of(&trials), &trials);

    let raw = section(&markdown, "1. ");
    let headings: Vec<&str> = raw
        .lines()
        .filter(|line| line.starts_with("| Metric |") || line.starts_with("| Tenth |"))
        .collect();
    assert_eq!(headings.len(), 2, "{raw}");
    for heading in headings {
        assert!(
            heading.contains(" foxhound, 1000 of the site's 1000 pages |"),
            "{heading}"
        );
    }
    let trials_table = &markdown[markdown
        .find("## Trials, in the order run")
        .expect("invariant: the report lists its trials")..];
    assert!(
        trials_table.contains("| 1.000 | 1000.0 | 1000 of 1000 |"),
        "{trials_table}"
    );
}

/// crawlee-rs's published CPU per page is written beside the median
/// measured here, on each of its measured cells and no other tool's.
#[test]
fn crawlee_rss_published_cpu_per_page_is_beside_its_measured_figure() {
    let trials = fixture::comparison(PAGES, |_, _| Measured {
        seconds: 1.0,
        user_seconds: 0.5,
        system_seconds: 0.25,
        work: work(),
        ..Measured::default()
    });

    let markdown = markdown(&of(&trials), &trials);

    let line = format!(
        "  - CPU per page: 0.75 ms measured here, the median of its trials; {}.",
        super::published::CRAWLEE_RS_CPU
    );
    assert_eq!(
        (
            cells_saying(section(&markdown, "1. "), &line),
            cells_saying(section(&markdown, "2. "), &line),
            cells_saying(section(&markdown, "5. "), &line),
        ),
        (vec!["crawlee-rs"], vec!["crawlee-rs"], Vec::<&str>::new())
    );
}

/// pagesift's own crawl benchmark reads Spider's pages with a 256-page
/// buffer on each site (#2215): each of its measured cells says so beside
/// the capacity the scenario's spider row reads them at, where Spider has
/// a measured main row, and no other tool's cell does.
#[test]
fn pagesifts_cells_note_the_buffer_its_own_spider_benchmark_reads_with() {
    let trials = fixture::comparison(PAGES, |_, _| Measured {
        seconds: 1.0,
        work: work(),
        ..Measured::default()
    });

    let markdown = markdown(&of(&trials), &trials);

    let line = |capacity: u32| {
        format!(
            "  - Spider beside it: pagesift's own crawl benchmark reads Spider's pages with \
             `subscribe(256)` on each site (`benchmarks/crawl/src/bin/spider_crawl.rs` at its \
             release 1.3.0); the spider row here reads them with `subscribe({capacity})`, as \
             its mapping says. Its published figures against Spider are that benchmark's, \
             never compared with these."
        )
    };
    assert_eq!(
        (
            cells_saying(section(&markdown, "1. "), &line(64)),
            cells_saying(section(&markdown, "3. "), &line(64)),
            cells_saying(section(&markdown, "4. "), &line(64)),
        ),
        (vec!["pagesift"], Vec::<&str>::new(), vec!["pagesift"])
    );
}

/// pagesift's polite cell runs one trial of each pass (#2215): its mapping
/// says so, with the owner's decision and why, and its same-work line and
/// its column in each table say one trial; no other cell's do.
#[test]
fn pagesifts_polite_cell_is_labelled_as_one_trial() {
    let trials = comparison(&[]);

    let markdown = markdown(&of(&trials), &trials);

    let one = "  - one trial: the owner's decision of 2026-11-14 (#2215): a trial of this cell \
               takes about 50 minutes, since its frontier takes a domain's pages after its first \
               four last, at 1.5 to 3 pages a second, so one timed trial and one run under strace \
               stand for it where the run asks for more; its figures are that trial's, with no \
               spread.";
    let same = "  - same work: 1 trial, each 1000 pages served in 600000 bytes.";
    let polite = section(&markdown, "3. ");
    let headed: Vec<&str> = markdown
        .lines()
        .filter(|line| line.starts_with("| Metric |") && line.contains("(one trial)"))
        .collect();
    assert_eq!(
        (
            cells_saying(&markdown, one),
            cells_saying(polite, same),
            headed.len(),
            headed
                .iter()
                .all(|line| line.contains("| pagesift (one trial), ")),
        ),
        (vec!["pagesift"], vec!["pagesift"], 1, true)
    );
}

/// The trials' section says how many runs under strace started above
/// their quiet load (#2521), as it says of the timed trials, and which
/// cells a later run measured again, with that run's order seed.
#[test]
fn the_trials_section_says_which_strace_runs_started_loaded_and_which_cells_were_redone() {
    use crate::compare::trials::{CountedCell, CountedRun, Redone, Tool};
    let mut trials = comparison(&[]);
    let run = |number, load| CountedRun {
        scenario: Scenario::Browser,
        cell: "foxhound".to_owned(),
        tool: Tool::Foxhound,
        number,
        load,
        count: 9,
        syncs: 0,
        work: work(),
        extracted: None,
    };
    trials.syscalls = CountedCell::Measured {
        trials: 3,
        runs: vec![run(1, Some(19.3)), run(2, Some(2.0)), run(3, None)],
        stopped: Vec::new(),
    };
    trials.redone = vec![Redone {
        cells: vec!["browser/foxhound".to_owned()],
        order_seed: 11,
    }];

    let markdown = markdown(&of(&trials), &trials);

    let section = &markdown[markdown
        .find("## Trials, in the order run")
        .expect("invariant: the report lists its trials")..];
    let notes: Vec<&str> = section
        .lines()
        .filter(|line| line.contains("strace") || line.starts_with("Measured again"))
        .collect();
    assert_eq!(
        notes,
        vec![
            "1 of the 3 runs under strace started above a 1-minute load of 8.",
            "Measured again by a later run at the same commit on the same machine, its trials in their own order (seed 11): browser/foxhound.",
        ]
    );
}

/// With scenario 1's Foxhound cells counted under Callgrind, each ratio is
/// the median of its own cell's runs, never a blend of both (#2175), and
/// the Markdown names the cell each ratio reads.
#[test]
fn each_callgrind_ratio_reads_its_own_cells_runs() {
    use super::Instructions;
    use crate::compare::trials::{CountedCell, CountedRun, Tool};
    let mut trials = comparison(&[]);
    let run = |cell: &str, tool, number, per_page: u64| CountedRun {
        scenario: Scenario::Raw,
        cell: cell.to_owned(),
        tool,
        number,
        load: None,
        count: per_page * PAGES,
        syncs: 0,
        work: work(),
        extracted: None,
    };
    trials.instructions = CountedCell::Measured {
        trials: 3,
        runs: vec![
            run("foxhound", Tool::Foxhound, 1, 2000),
            run("foxhound", Tool::Foxhound, 2, 2200),
            run("foxhound", Tool::Foxhound, 3, 2100),
            run("foxhound defaults", Tool::Foxhound, 1, 3000),
            run("foxhound defaults", Tool::Foxhound, 2, 3000),
            run("spider", Tool::Spider, 1, 1000),
            run("spider", Tool::Spider, 2, 1000),
            run("spider", Tool::Spider, 3, 1000),
        ],
        stopped: Vec::new(),
    };

    let report = of(&trials);
    let markdown = markdown(&report, &trials);

    let ratios: Vec<(&str, &str, f64)> = match &report.instructions {
        Instructions::Measured { ratios, .. } => ratios
            .iter()
            .map(|ratio| (ratio.row, ratio.foxhound, ratio.ratio))
            .collect(),
        Instructions::NotRun { reason } => panic!("the Callgrind cell ran: {reason}"),
    };
    let lines: Vec<&str> = markdown
        .lines()
        .filter(|line| line.starts_with("`foxhound.spider_instruction_ratio"))
        .collect();
    assert_eq!(
        (ratios, lines),
        (
            vec![
                ("foxhound.spider_instruction_ratio", "foxhound", 2.1),
                (
                    "foxhound.spider_instruction_ratio_at_defaults",
                    "foxhound defaults",
                    3.0
                ),
            ],
            vec![
                "`foxhound.spider_instruction_ratio`: 2.100, Foxhound's `foxhound` cell's 2100 instructions per page over Spider's `spider` cell's 1000, under Callgrind on scenario 1 of ADR 0138's matrix, the median of each cell's runs, all of the same pages and bytes.",
                "`foxhound.spider_instruction_ratio_at_defaults`: 3.000, Foxhound's `foxhound defaults` cell's 3000 instructions per page over Spider's `spider` cell's 1000, under Callgrind on scenario 1 of ADR 0138's matrix, Foxhound at the catalog's defaults at the scenario's pace, the median of each cell's runs, all of the same pages and bytes.",
            ]
        )
    );
}

/// A default-cost cell (#2438) the run did not count says it was not run,
/// beside the line of one it counted.
#[test]
fn a_default_the_run_did_not_count_is_not_run() {
    let lines = default_cost_lines(&[("foxhound", 2000), ("foxhound records on", 2500)]);

    assert_eq!(
        lines,
        [
            "| the Provenance log | `provenance.log: true` | not run: the run asked for no run of this cell |  |",
            "| Records | `dataset.records: true` | 2500 | +500 (+25.00 %) |",
        ]
    );
}

/// A run that counted no run of scenario 1's first cell gives no default's
/// difference, even of a default-cost cell it counted (#2438).
#[test]
fn without_the_first_cell_no_default_is_read() {
    let lines = default_cost_lines(&[("foxhound records on", 2500)]);

    assert_eq!(
        lines,
        [
            "| the Provenance log | `provenance.log: true` | not run: the run asked for no run of this cell |  |",
            "| Records | `dataset.records: true` | not run: the run counted no run of scenario 1's first cell |  |",
        ]
    );
}

/// The Provenance log's and Records' lines of the report of a Callgrind
/// cell that counted each of `cells`, a scenario 1 cell of Foxhound and its
/// instructions per page.
fn default_cost_lines(cells: &[(&str, u64)]) -> Vec<String> {
    use crate::compare::trials::{CountedCell, CountedRun, Tool};
    let mut trials = comparison(&[]);
    trials.instructions = CountedCell::Measured {
        trials: 1,
        runs: cells
            .iter()
            .map(|&(cell, per_page)| CountedRun {
                scenario: Scenario::Raw,
                cell: cell.to_owned(),
                tool: Tool::Foxhound,
                number: 1,
                load: None,
                count: per_page * PAGES,
                syncs: 0,
                work: work(),
                extracted: None,
            })
            .collect(),
        stopped: Vec::new(),
    };
    markdown(&of(&trials), &trials)
        .lines()
        .filter(|line| {
            line.starts_with("| Records |") || line.starts_with("| the Provenance log |")
        })
        .map(str::to_owned)
        .collect()
}

/// Every cell measured with `curve`, Foxhound's cell at the catalog's
/// defaults having sent 600 pages over HTTP, 300 from the browser's
/// context and 100 from the browser.
fn curve_and_tiers(curve: &[f64]) -> Comparison {
    use std::collections::BTreeMap;
    fixture::comparison(PAGES, |cell, _| Measured {
        seconds: 1.0,
        work: work(),
        curve: curve.to_vec(),
        tiers: (cell.name == "foxhound defaults").then(|| {
            BTreeMap::from([
                ("http".to_owned(), 600),
                ("browser_fetch".to_owned(), 300),
                ("browser".to_owned(), 100),
            ])
        }),
        ..Measured::default()
    })
}

/// The figures `metrics` give scenario 1's cell `name` in `report`, to a
/// thousandth; `None` for each it does not give.
fn scenario_1_figures(report: &Report, name: &str, metrics: &[&str]) -> Vec<Option<f64>> {
    let cell = report
        .scenarios
        .iter()
        .find(|report| report.scenario == Scenario::Raw)
        .and_then(|report| report.cells.iter().find(|cell| cell.cell == name))
        .expect("scenario 1 has the cell");
    let CellStatus::Measured { metrics: given, .. } = &cell.status else {
        panic!("{name} is measured");
    };
    metrics
        .iter()
        .map(|metric| match given.get(metric) {
            Some(Figure::Measured { median, .. }) => Some((median * 1000.0).round() / 1000.0),
            _ => None,
        })
        .collect()
}

/// Each cell gives its steady rate, the mean of its curve between the
/// first and the last tenth (#2175).
#[test]
fn cells_give_their_steady_rate() {
    let trials = curve_and_tiers(&[1.0, 4.0, 4.0, 5.0, 5.0, 5.0, 5.0, 6.0, 6.0, 100.0]);

    let report = of(&trials);

    let steady = ["steady_pages_per_second"];
    assert_eq!(
        (
            scenario_1_figures(&report, "foxhound defaults", &steady),
            scenario_1_figures(&report, "foxhound", &steady),
        ),
        (vec![Some(5.0)], vec![Some(5.0)])
    );
}

/// A cell gives the share of its pages each tier sent where its row reads
/// them from Provenance, and none where it does not (#2175).
#[test]
fn cells_give_their_tiers_where_their_row_reads_them() {
    let trials = curve_and_tiers(&[1.0; 10]);

    let report = of(&trials);

    let tiers = [
        "tier_http_share",
        "tier_browser_fetch_share",
        "tier_browser_share",
        "tier_other_share",
    ];
    assert_eq!(
        (
            scenario_1_figures(&report, "foxhound defaults", &tiers),
            scenario_1_figures(&report, "foxhound", &tiers),
        ),
        (
            vec![Some(0.6), Some(0.3), Some(0.1), Some(0.0)],
            vec![None, None, None, None],
        )
    );
}
