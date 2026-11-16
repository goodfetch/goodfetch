use super::standing::Standing;
use super::{CellStatus, Figure, Report, markdown, of};
use crate::compare::matrix::Scenario;
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
/// #2439, the crawl-like scenario's for #2284 and the auto cells for
/// #2411 (their own tests). The cell at the catalog's defaults says it is
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
    let pending = "  - not measured: pending #2175, which brings the cell at the catalog's \
                   defaults.";

    assert_eq!(
        (
            saying,
            unmeasured,
            cells_saying(section(&markdown, Scenario::Raw256.title()), pending),
        ),
        (
            Scenario::ALL
                .into_iter()
                .map(|scenario| (
                    scenario,
                    if scenario.no_bar().is_some()
                        && !scenario.hostile()
                        && scenario != Scenario::CrawlLike
                        && scenario != Scenario::Auto
                    {
                        every(scenario)
                    } else {
                        Vec::new()
                    }
                ))
                .collect::<Vec<_>>(),
            Vec::new(),
            vec!["foxhound defaults"],
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
