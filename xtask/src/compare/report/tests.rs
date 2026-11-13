use super::verdict::Verdict;
use super::{Report, markdown, of};
use crate::compare::matrix::{self, Scenario};
use crate::compare::trials::{
    Comparison, CountedCell, Declared, GrowthCell, Machine, Measured, Trial, Work,
};

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
    }
}

/// Three trials of each measured cell over the 1,000-page site, each at the
/// pages per second `rates` gives its cell, 1,000 for a cell it does not
/// name.
fn comparison(rates: &[(Scenario, &str, [f64; 3])]) -> Comparison {
    let trials = matrix::measured()
        .flat_map(|(cell, config)| {
            let rates = rates
                .iter()
                .find(|(scenario, name, _)| *scenario == cell.scenario && *name == cell.name)
                .map_or([1000.0; 3], |(_, _, rates)| *rates);
            (1..=3).zip(rates).map(move |(number, rate)| Trial {
                scenario: cell.scenario,
                cell: cell.name.to_owned(),
                tool: config.tool(),
                number,
                site_pages: PAGES,
                load: None,
                retried: None,
                measured: Measured {
                    seconds: 1000.0 / rate,
                    work: work(),
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
            site: "/sites/1000/1".to_owned(),
            spider: "2.53.9".to_owned(),
            crawlee: "3.18.2".to_owned(),
            scrapy: "2.19.0".to_owned(),
            playwright: "1.64.1".to_owned(),
            scrapy_playwright: "0.0.48".to_owned(),
            browser: "156.0.8078.4".to_owned(),
            scenarios: Scenario::ALL.to_vec(),
            command: "just compare release".to_owned(),
        },
        machine: Machine {
            class: "local-controlled".to_owned(),
            name: None,
            os: "linux 6.8.0".to_owned(),
            arch: "x86_64".to_owned(),
            cpu: "AMD EPYC-Milan Processor".to_owned(),
            threads: 16,
            memory_bytes: None,
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

/// The verdict on `metric` of the `product` cell of `scenario` against its
/// `competitor` cell.
fn verdict(
    report: &Report<'_>,
    scenario: Scenario,
    product: &str,
    competitor: &str,
    metric: &str,
) -> Option<Verdict> {
    report
        .scenarios
        .iter()
        .find(|report| report.scenario == scenario)?
        .verdicts
        .iter()
        .find(|verdicts| verdicts.product == product && verdicts.competitor == competitor)?
        .metrics
        .get(metric)
        .copied()
}

/// The section of `title` in the Markdown, to the next section.
fn section<'a>(markdown: &'a str, title: &str) -> &'a str {
    let start = markdown
        .find(&format!("\n## {title}"))
        .unwrap_or_else(|| panic!("no section {title} in:\n{markdown}"));
    let rest = &markdown[start + 1..];
    rest.find("\n## ").map_or(rest, |end| &rest[..end])
}

/// The rows of a section's verdict table: each metric's label and its
/// verdict against each competitor, in the columns' order.
fn verdict_rows(section: &str) -> Vec<Vec<&str>> {
    section
        .lines()
        .skip_while(|line| !line.starts_with("| Verdict |"))
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
            verdict(&report, Scenario::Raw, "foxhound", "spider", rate),
            verdict(&report, Scenario::Raw, "foxhound", "crawlee", rate),
            verdict(&report, Scenario::Browser, "foxhound", "spider", rate),
        ),
        (
            Some(Verdict::Ahead),
            Some(Verdict::Behind),
            Some(Verdict::Overlapping)
        )
    );
    let raw = verdict_rows(section(&markdown, "1. "));
    let browser = verdict_rows(section(&markdown, "5. "));
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
                "behind"
            ]),
            Some(&vec![
                "Throughput: pages per second",
                "overlapping: foxhound 11.5 (11.0–12.0), spider 15.0 (6.8–28.9)",
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
            .all(|word| ["ahead", "behind", "overlapping", "no verdict"].contains(word)),
        "{words:?}"
    );
}

/// Sensitivity rows are no competitor of their own: the verdicts are
/// against each other tool's main row.
#[test]
fn verdicts_are_against_each_competitors_main_row() {
    let trials = comparison(&[]);

    let report = of(&trials);

    let raw = report
        .scenarios
        .iter()
        .find(|report| report.scenario == Scenario::Raw)
        .expect("invariant: the comparison runs scenario 1");
    assert_eq!(
        raw.verdicts
            .iter()
            .map(|verdicts| (verdicts.product, verdicts.competitor))
            .collect::<Vec<_>>(),
        [
            ("foxhound", "spider"),
            ("foxhound", "crawlee"),
            ("foxhound", "scrapy")
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

/// Every cell of the extraction scenario writes content no check reads yet
/// (#2136): each says so, and the scenario gives no verdict.
#[test]
fn extraction_cells_say_content_not_verified_and_get_no_verdict() {
    let trials = comparison(&[
        (Scenario::Extraction, "foxhound", [20.0, 21.0, 22.0]),
        (Scenario::Extraction, "crawlee", [10.0, 11.0, 12.0]),
    ]);

    let report = of(&trials);
    let markdown = markdown(&report, &trials);

    let extraction = section(&markdown, "4. ");
    let unverified = "  - content not verified: no check reads what it wrote, until #2136's \
                      output check lands; no verdict.";
    assert_eq!(
        (
            report
                .scenarios
                .iter()
                .find(|report| report.scenario == Scenario::Extraction)
                .map(|report| report.verdicts.len()),
            cells_saying(extraction, unverified),
            verdict_rows(extraction),
        ),
        (
            Some(0),
            vec!["foxhound", "spider", "crawlee", "scrapy", "spider mimalloc"],
            Vec::<Vec<&str>>::new()
        )
    );
}

/// The extraction scenario's Spider cells are declared as not doing the
/// Product's work: each says why, with no verdict and no ratio.
#[test]
fn spiders_extraction_cells_are_not_the_same_work() {
    let trials = comparison(&[]);

    let markdown = markdown(&of(&trials), &trials);

    let unlike = "  - not the same work: it writes each page's raw HTML, where the others write \
                  the text of its main content (until #2136); no verdict and no ratio.";
    assert_eq!(
        cells_saying(section(&markdown, "4. "), unlike),
        ["spider", "spider mimalloc"]
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
        trials_table.contains(
            "| Scenario | Cell | Trial | Load at start | Seconds | Pages per second | Pages fetched | Peak RSS |"
        ) && trials_table.contains("| 1.000 | 1000.0 | 1000 of 1000 |"),
        "{trials_table}"
    );
}
