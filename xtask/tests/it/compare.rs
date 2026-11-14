//! The comparison matrix (ADRs 0134 and 0138): `xtask compare report` reads
//! the trials `xtask compare run` wrote and gives, for each scenario, each
//! cell's mapping and every metric of the owner's list as the median of its
//! trials with their spread, labelled `local-controlled` with the machine,
//! only once every trial of every measured cell fetched the same pages and
//! was served the same bytes; otherwise it fails naming the trial that did
//! other work, and writes no figure. A cell a tool cannot run says so, a
//! Foxhound value that does not exist yet is pending, and the Callgrind
//! cell gives `foxhound.spider_instruction_ratio` and the extraction
//! scenario's two ratios under the same check (ADR 0140). On the extraction
//! scenario's article site, each trial's pages are also held to the right
//! answers (#2136): a Foxhound trial that differs is refused, and a
//! competitor's keeps its figures, labelled. The Collie scenario (issue
//! 2282) gives Collie's CDP commands per action over Playwright's, held to
//! its register row's Today, under a same-work check of its own. Trials
//! are fixtures here.

use serde_json::{Value, json};

use crate::workspace::{Workspace, finding};

const TRIALS: &str = "target/compare/trials.json";
const REPORT: &str = "target/compare/report.json";
const MARKDOWN: &str = "target/compare/report.md";
const MIB: u64 = 1024 * 1024;

/// The measured cells: each tool's main rows by scenario, then Spider's
/// sensitivity rows and Foxhound's `auto` row.
const CELLS: [(&str, &str, &str); 33] = [
    ("raw", "foxhound", "foxhound"),
    ("raw", "spider", "spider"),
    ("raw", "crawlee", "crawlee"),
    ("raw", "scrapy", "scrapy"),
    ("raw", "crawlee-rs", "crawlee-rs"),
    ("raw", "pagesift", "pagesift"),
    ("durable", "foxhound full", "foxhound"),
    ("durable", "foxhound periodic", "foxhound"),
    ("durable", "crawlee-rs", "crawlee-rs"),
    ("durable", "pagesift", "pagesift"),
    ("polite", "foxhound", "foxhound"),
    ("polite", "crawlee", "crawlee"),
    ("polite", "scrapy", "scrapy"),
    ("polite", "crawlee-rs", "crawlee-rs"),
    ("polite", "pagesift", "pagesift"),
    ("extraction", "foxhound", "foxhound"),
    ("extraction", "foxhound filter lists off", "foxhound"),
    ("extraction", "spider", "spider"),
    ("extraction", "crawlee", "crawlee"),
    ("extraction", "scrapy", "scrapy"),
    ("extraction", "crawlee-rs", "crawlee-rs"),
    ("extraction", "pagesift", "pagesift"),
    ("browser", "foxhound", "foxhound"),
    ("browser", "spider", "spider"),
    ("browser", "crawlee", "crawlee"),
    ("browser", "scrapy", "scrapy"),
    ("browser", "pagesift", "pagesift"),
    ("raw", "spider capacity 4096", "spider"),
    ("raw", "spider default features", "spider"),
    ("raw", "spider bloom", "spider"),
    ("raw", "spider jemalloc", "spider"),
    ("extraction", "spider mimalloc", "spider"),
    ("polite", "foxhound auto", "foxhound"),
];

/// The runs of `cell` in a pass that asks for `asked`: pagesift's polite
/// cell runs one trial of each pass, the owner's decision of 2026-11-14
/// (#2215), and every other cell the run's.
fn runs(scenario: &str, cell: &str, asked: u32) -> u32 {
    if (scenario, cell) == ("polite", "pagesift") {
        asked.min(1)
    } else {
        asked
    }
}

/// The pages of `scenario`'s site: the polite scenario's multi-domain site
/// holds 10 pages on each of its 1,000 domains (#2283), every other the
/// declared 1,000.
fn site_pages(scenario: &str) -> u64 {
    if scenario == "polite" { 10_000 } else { 1000 }
}

/// The work of a trial that fetched each page of a site of `pages` once,
/// 600 bytes each, and `again` pages a second time.
fn work_on(pages: u64, again: u64) -> Value {
    json!({
        "pages": pages,
        "missing": [],
        "extra": [],
        "requests": pages + again,
        "bytes": 600 * (pages + again),
        "first_bytes": 600 * pages,
        "other_protocol": 0,
    })
}

/// What the output check found of a crawl `differing` of whose pages
/// differ from the whole right answer, each by `first`, and
/// `beside_the_ad` of them by more than the ad's marker.
fn extracted(differing: u64, beside_the_ad: u64, first: &str) -> Value {
    let found = |pages: u64| {
        if pages == 0 {
            json!({ "pages": 0 })
        } else {
            json!({ "pages": pages, "first": first })
        }
    };
    json!({ "whole": found(differing), "ad_allowed": found(beside_the_ad) })
}

/// The first difference of a page that holds the ad.
const AD: &str =
    "/sites/1000/1: the text holds the ad's \"Sponsored offer from Harbourline Freight\"";

/// A trial of `cell` in `seconds`, at `peak_bytes`, that did `work`, its
/// figures otherwise the same for every trial: 2 s of user and 1 s of
/// system time, 8 MiB written, 1 MiB left on disk, its tree sampled at
/// half its peak on average, its peak PSS 1 MiB and its private memory
/// 2 MiB under its peak RSS. An extraction trial read every page right;
/// Foxhound's with its Filter lists off kept the ad on every page, as
/// nothing removes it there.
fn trial(
    scenario: &str,
    cell: &str,
    tool: &str,
    number: u32,
    seconds: f64,
    peak_bytes: u64,
    work: &Value,
) -> Value {
    let mut trial = json!({
        "scenario": scenario,
        "cell": cell,
        "tool": tool,
        "number": number,
        "site_pages": site_pages(scenario),
        "concurrency": 16,
        "seconds": seconds,
        "processes": if scenario == "durable" { 2 } else { 1 },
        "user_seconds": 2.0,
        "system_seconds": 1.0,
        "peak_bytes": peak_bytes,
        "sampled": {
            "samples": 40,
            "peak_bytes": peak_bytes,
            "average_bytes": peak_bytes / 2,
            "peak_pss_bytes": peak_bytes - MIB,
            "peak_private_bytes": peak_bytes - 2 * MIB,
            "peak_cores": 1.5,
            "peak_threads": 20,
            "peak_open_files": 30,
            "at_pages": [[1000, peak_bytes]],
        },
        "read_bytes": 0,
        "written_bytes": 8 * MIB,
        "footprint_bytes": MIB,
        "executable_bytes": 20 * MIB,
        "work": work,
        "requests_sent": 1000,
        "duplicates": 0,
        "errors": 0,
        "sent_bytes": 100_000,
        "received_bytes": 600_000,
        "connections": 16,
        "tls_connections": 0,
        "startup_seconds": 0.01,
        "first_page_seconds": 0.02,
        "first_record_seconds": if tool == "foxhound" { json!(0.05) } else { json!(null) },
        "curve": [100.0, 100.0, 100.0, 100.0, 100.0, 100.0, 100.0, 100.0, 100.0, 100.0],
        "latency": { "p50": 0.01, "p95": 0.02, "p99": 0.03 },
        "latency_end": if tool == "foxhound" { "record" } else { "fetch" },
    });
    if scenario == "extraction" {
        trial["extracted"] = if cell == "foxhound filter lists off" {
            extracted(1000, 0, AD)
        } else {
            extracted(0, 0, "")
        };
    }
    trial
}

/// Trial `number` of `cell`, in `seconds` at `peak_bytes`, over its
/// scenario's site: a durable trial fetched `number` pages again after its
/// kill, and Foxhound's `auto` row 3,000 pages twice, its samples.
fn cell_trial(
    scenario: &str,
    cell: &str,
    tool: &str,
    number: u32,
    seconds: f64,
    peak_bytes: u64,
) -> Value {
    let again = match (scenario, cell) {
        ("durable", _) => u64::from(number),
        (_, "foxhound auto") => 3000,
        _ => 0,
    };
    let work = work_on(site_pages(scenario), again);
    let mut trial = trial(scenario, cell, tool, number, seconds, peak_bytes, &work);
    if cell == "foxhound auto" {
        trial["sample_pages"] = json!(3000);
    }
    trial
}

/// A cell's three trials, or its one ([`runs`]): in 1, 2 and 4 seconds at
/// 20, 30 and 10 MiB.
fn cell_trials(scenario: &str, cell: &str, tool: &str) -> Vec<Value> {
    [(1.0, 20), (2.0, 30), (4.0, 10)]
        .into_iter()
        .zip(1..=runs(scenario, cell, 3))
        .map(|((seconds, mib), number)| {
            cell_trial(scenario, cell, tool, number, seconds, mib * MIB)
        })
        .collect()
}

fn not_run(reason: &str) -> Value {
    json!({ "status": "not_run", "reason": reason })
}

/// A trials file of three trials of each measured cell over the 1,000-page
/// site; the durable cell's trials fetched 1, 2 and 3 pages again after
/// their kill.
fn trials() -> Value {
    let trials: Vec<Value> = CELLS
        .iter()
        .flat_map(|(scenario, cell, tool)| cell_trials(scenario, cell, tool))
        .collect();
    json!({
        "declared": {
            "trials": 3,
            "statistic": "median, with the lowest and highest trial",
            "concurrency": 16,
            "site": "/sites/1000/1",
            "spider": "2.53.9",
            "crawlee": "3.18.2",
            "scrapy": "2.19.0",
            "playwright": "1.64.1",
            "scrapy_playwright": "0.0.48",
            "crawlee_rs": "2a3ed90b8391c86005ce6ed8dd3e3248d0d34579",
            "pagesift": "1.3.0",
            "browser": "156.0.8078.4",
            "scenarios": ["raw", "durable", "polite", "extraction", "browser"],
            "command": "just compare release",
        },
        "machine": {
            "class": "local-controlled",
            "os": "linux 6.8.0",
            "arch": "x86_64",
            "cpu": "AMD EPYC-Milan Processor",
            "threads": 16,
            "load": 0.5,
        },
        "commit": "0123456789abcdef0123456789abcdef01234567",
        "order_seed": 7,
        "trials": trials,
        "rules": [],
        "syscalls": not_run("strace is not installed on this host"),
        "growth": not_run("the growth cell runs with `--growth`"),
        "sizes": not_run("the run asked for no scenario 1 main cell of Foxhound or Spider"),
        "instructions": not_run("valgrind is not installed on this host"),
        "collie": not_run("the run asked for no Collie cell"),
    })
}

/// A counted run of `cell` counting `count`, with `syncs`, over the site;
/// an extraction run read its pages as its cell's trials did.
fn run(scenario: &str, cell: &str, tool: &str, number: u32, count: u64, syncs: u64) -> Value {
    let mut run = json!({
        "scenario": scenario,
        "cell": cell,
        "tool": tool,
        "number": number,
        "count": count,
        "syncs": syncs,
        "work": work_on(site_pages(scenario), 0),
    });
    if scenario == "extraction" {
        run["extracted"] = if cell == "foxhound" {
            extracted(0, 0, "")
        } else {
            extracted(1000, 0, AD)
        };
    }
    run
}

/// A Callgrind cell of three runs of each tool over the 1,000-page site.
/// In scenario 1, Foxhound's a median of 32,000 instructions a page and
/// Spider's 1,000; in the extraction scenario, Foxhound's 60,000 at its
/// defaults and 45,000 with its Filter lists off, and Spider's 50,000, its
/// pages holding the ad, which it does not remove.
fn callgrind() -> Value {
    json!({
        "status": "measured",
        "trials": 3,
        "runs": [
            run("raw", "foxhound", "foxhound", 1, 31_000_000, 0),
            run("raw", "foxhound", "foxhound", 2, 32_000_000, 0),
            run("raw", "foxhound", "foxhound", 3, 40_000_000, 0),
            run("raw", "spider", "spider", 1, 1_000_000, 0),
            run("raw", "spider", "spider", 2, 900_000, 0),
            run("raw", "spider", "spider", 3, 1_100_000, 0),
            run("extraction", "foxhound", "foxhound", 1, 59_000_000, 0),
            run("extraction", "foxhound", "foxhound", 2, 60_000_000, 0),
            run("extraction", "foxhound", "foxhound", 3, 70_000_000, 0),
            run("extraction", "foxhound filter lists off", "foxhound", 1, 45_000_000, 0),
            run("extraction", "foxhound filter lists off", "foxhound", 2, 44_000_000, 0),
            run("extraction", "foxhound filter lists off", "foxhound", 3, 46_000_000, 0),
            run("extraction", "spider", "spider", 1, 50_000_000, 0),
            run("extraction", "spider", "spider", 2, 49_000_000, 0),
            run("extraction", "spider", "spider", 3, 51_000_000, 0),
        ],
    })
}

/// A system-call cell of three runs of each measured cell, or its one
/// ([`runs`]): Foxhound's durable crawl syncing 7,000, 7,500 and 8,000
/// times.
fn strace() -> Value {
    let mut counted = Vec::new();
    for (scenario, cell, tool) in CELLS {
        for (number, syncs) in [(1, 7000), (2, 7500), (3, 8000)]
            .into_iter()
            .filter(|(number, _)| *number <= runs(scenario, cell, 3))
        {
            let syncs = if cell == "foxhound full" { syncs } else { 0 };
            counted.push(run(scenario, cell, tool, number, 300_000, syncs));
        }
    }
    json!({ "status": "measured", "trials": 3, "runs": counted })
}

fn report(workspace: &Workspace, trials: &Value) -> crate::workspace::Outcome {
    workspace.write(TRIALS, trials.to_string().as_bytes());
    workspace.xtask(&["compare", "report", "--trials", TRIALS])
}

fn written(workspace: &Workspace) -> Value {
    serde_json::from_slice(&workspace.read(REPORT).unwrap()).unwrap()
}

/// The report's cell `cell` of the scenario at `index`.
fn cell<'a>(report: &'a Value, index: usize, cell: &str) -> &'a Value {
    report["scenarios"][index]["cells"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["cell"] == cell)
        .unwrap()
}

#[test]
fn trials_of_the_same_work_report_each_cells_medians_with_their_spread() {
    let workspace = Workspace::from_fixture("empty");

    let outcome = report(&workspace, &trials());

    assert_eq!(
        (outcome.findings(), outcome.code),
        (vec![], 0),
        "{}",
        outcome.stderr
    );
    let report = written(&workspace);
    let foxhound = &cell(&report, 0, "foxhound")["metrics"];
    assert_eq!(
        (
            &report["label"],
            &report["published"],
            &report["machine"]["cpu"],
            &foxhound["pages_per_second"],
            &foxhound["peak_rss_bytes"],
            &foxhound["cpu_seconds_per_1000_pages"]["median"],
            &foxhound["written_bytes"]["median"],
        ),
        (
            &json!("local-controlled"),
            &json!(false),
            &json!("AMD EPYC-Milan Processor"),
            &json!({ "status": "measured", "median": 500.0, "low": 250.0, "high": 1000.0, "unit": "pages_per_second" }),
            &json!({ "status": "measured", "median": f64::from(20 * 1024 * 1024), "low": f64::from(10 * 1024 * 1024), "high": f64::from(30 * 1024 * 1024), "unit": "bytes" }),
            &json!(3.0),
            &json!(8.0 * 1024.0 * 1024.0),
        )
    );
    // Scenario 1's figures are the ones `just targets` reads.
    assert_eq!(
        report["tools"],
        json!({
            "foxhound": { "pages_per_second": 500.0, "peak_bytes": 20 * MIB },
            "spider": { "pages_per_second": 500.0, "peak_bytes": 20 * MIB },
            "crawlee": { "pages_per_second": 500.0, "peak_bytes": 20 * MIB },
            "scrapy": { "pages_per_second": 500.0, "peak_bytes": 20 * MIB },
            "crawlee-rs": { "pages_per_second": 500.0, "peak_bytes": 20 * MIB },
            "pagesift": { "pages_per_second": 500.0, "peak_bytes": 20 * MIB },
        })
    );
}

/// The trials file with a sizes cell: three trials of Foxhound and Spider
/// at each size and concurrency, Spider's at 1 in flight aside, Foxhound's
/// peak 22 MiB, Spider's 18 MiB at 100 pages and 30 MiB above.
fn sized_trials() -> Value {
    let mut trials = trials();
    let mut sized = Vec::new();
    for pages in [100_u64, 1000, 10_000] {
        for concurrency in [1, 16, 64] {
            for (tool, mib) in [
                ("foxhound", 22),
                ("spider", if pages == 100 { 18 } else { 30 }),
            ] {
                if (tool, concurrency) == ("spider", 1) {
                    continue;
                }
                for number in 1..=3 {
                    let mut trial = trial(
                        "raw",
                        tool,
                        tool,
                        number,
                        1.0,
                        mib * MIB,
                        &json!({
                            "pages": pages,
                            "missing": [],
                            "extra": [],
                            "requests": pages,
                            "bytes": 600 * pages,
                            "first_bytes": 600 * pages,
                            "other_protocol": 0,
                        }),
                    );
                    trial["site_pages"] = json!(pages);
                    trial["concurrency"] = json!(concurrency);
                    trial["requests_sent"] = json!(pages);
                    trial["sampled"]["peak_executable_bytes"] = json!(9 * MIB);
                    sized.push(trial);
                }
            }
        }
    }
    trials["sizes"] = json!({
        "status": "measured",
        "trials": 3,
        "trials_run": sized,
        "stopped": [],
    });
    trials
}

/// The sizes cell (spec #2165): Foxhound's and Spider's scenario 1 main
/// rows at each size and concurrency, each tool's median peak, private
/// memory and executable's pages, and at 16 in flight alone whether
/// Foxhound's peak is at most Spider's. Spider runs no trial at 1 in
/// flight, and its row there says why.
#[test]
fn the_sizes_cell_reports_each_tools_peak_with_the_bar_at_16_in_flight() {
    let workspace = Workspace::from_fixture("empty");

    let outcome = report(&workspace, &sized_trials());

    assert_eq!(
        (outcome.findings(), outcome.code),
        (vec![], 0),
        "{}",
        outcome.stderr
    );
    let report = written(&workspace);
    let points = report["sizes"]["points"].as_array().unwrap();
    let bars: Vec<(u64, u64, Value)> = points
        .iter()
        .map(|point| {
            (
                point["site_pages"].as_u64().unwrap(),
                point["concurrency"].as_u64().unwrap(),
                point["bar_met"].clone(),
            )
        })
        .collect();
    let mut expected = Vec::new();
    for concurrency in [16, 1, 64] {
        for pages in [100, 1000, 10_000] {
            let bar = match (concurrency, pages) {
                (16, 100) => json!(false),
                (16, _) => json!(true),
                _ => Value::Null,
            };
            expected.push((pages, concurrency, bar));
        }
    }
    assert_eq!(bars, expected);
    assert_eq!(
        points[0]["tools"]["foxhound"],
        json!({
            "status": "measured",
            "trials": 3,
            "peak_bytes": 22 * MIB,
            "private_bytes": 20 * MIB,
            "executable_bytes": 9 * MIB,
        })
    );
    let spider_at_1 = &points[3]["tools"]["spider"];
    assert_eq!(
        (
            &points[3]["concurrency"],
            &spider_at_1["status"],
            spider_at_1["reason"]
                .as_str()
                .is_some_and(|reason| !reason.is_empty()),
        ),
        (&json!(1), &json!("not_measured"), true)
    );
    let markdown = workspace.read_text(MARKDOWN);
    assert!(
        markdown.contains("| 100 | 16 | foxhound | 3 | 22.0 MiB | 20.0 MiB | 9.0 MiB | missed |")
            && markdown.contains("| 100 | 1 | spider | not measured: "),
        "{markdown}"
    );
}

/// Spider's sensitivity rows (#2213) stand beside its main row, each
/// labelled as one in the report and its Markdown, and the figures `just
/// targets` reads stay the main row's, whatever a sensitivity row measured.
#[test]
fn spiders_sensitivity_rows_are_labelled_and_no_target_reads_them() {
    let workspace = Workspace::from_fixture("empty");
    let mut trials = trials();
    for trial in trials["trials"].as_array_mut().unwrap() {
        if trial["cell"] == "spider capacity 4096" {
            trial["peak_bytes"] = json!(60 * MIB);
            trial["sampled"]["peak_bytes"] = json!(60 * MIB);
        }
    }

    let outcome = report(&workspace, &trials);

    assert_eq!(
        (outcome.findings(), outcome.code),
        (vec![], 0),
        "{}",
        outcome.stderr
    );
    let report = written(&workspace);
    let rows = |index: usize| -> Vec<(&str, &str)> {
        report["scenarios"][index]["cells"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|cell| cell["tool"] == "spider")
            .map(|cell| {
                (
                    cell["cell"].as_str().unwrap(),
                    cell["row"].as_str().unwrap(),
                )
            })
            .collect()
    };
    assert_eq!(
        (rows(0), rows(3), rows(4)),
        (
            vec![
                ("spider", "main"),
                ("spider capacity 4096", "sensitivity"),
                ("spider default features", "sensitivity"),
                ("spider bloom", "sensitivity"),
                ("spider jemalloc", "sensitivity"),
            ],
            vec![("spider", "main"), ("spider mimalloc", "sensitivity")],
            vec![("spider", "main"), ("spider smart", "sensitivity")],
        )
    );
    assert_eq!(
        cell(&report, 4, "spider smart")["status"],
        json!("not_supported")
    );
    assert_eq!(
        (
            &cell(&report, 0, "spider capacity 4096")["metrics"]["peak_rss_bytes"]["median"],
            &report["tools"]["spider"],
        ),
        (
            &json!(f64::from(60 * 1024 * 1024)),
            &json!({ "pages_per_second": 500.0, "peak_bytes": 20 * MIB }),
        )
    );
    let markdown = workspace.read_text(MARKDOWN);
    for expected in [
        "- **spider**: HTTP mode (`crawl_raw`, no `chrome` feature)",
        "- **spider capacity 4096** (sensitivity row): the spider row's settings with its pages \
         read from `subscribe(4096)`",
        "- **spider jemalloc** (sensitivity row): ",
        "- **spider mimalloc** (sensitivity row): ",
        "- **spider smart** (sensitivity row): ",
        "  - not supported: Spider's `crawl_smart()` fetched 1 of the site's 100 pages",
        "| Metric | foxhound, 1000 of the site's 1000 pages | spider, 1000 of the site's 1000 pages \
         | crawlee, 1000 of the site's 1000 pages | scrapy, 1000 of the site's 1000 pages | \
         crawlee-rs, 1000 of the site's 1000 pages | pagesift, 1000 of the site's 1000 pages | \
         spider capacity 4096 (sensitivity), 1000 of the site's 1000 pages | spider default \
         features (sensitivity), 1000 of the site's 1000 pages | spider bloom (sensitivity), 1000 \
         of the site's 1000 pages | spider jemalloc (sensitivity), 1000 of the site's 1000 pages |",
    ] {
        assert!(
            markdown.contains(expected),
            "{expected} missing:\n{markdown}"
        );
    }
}

/// Every metric of the owner's list has a row in each measured cell.
#[test]
fn each_measured_cell_reports_every_metric_of_the_owners_list() {
    let workspace = Workspace::from_fixture("empty");

    report(&workspace, &trials());

    let report = written(&workspace);
    let metrics = cell(&report, 0, "foxhound")["metrics"].as_object().unwrap();
    let names: Vec<&str> = metrics.keys().map(String::as_str).collect();
    for name in [
        "pages_per_second",
        "first_page_seconds",
        "first_record_seconds",
        "latency_p50_seconds",
        "latency_p95_seconds",
        "latency_p99_seconds",
        "user_seconds",
        "system_seconds",
        "cpu_seconds_per_1000_pages",
        "average_cores",
        "peak_cores",
        "peak_rss_bytes",
        "peak_pss_bytes",
        "average_rss_bytes",
        "rss_per_1000_pages_at_1000",
        "rss_per_1000_pages_at_10000",
        "rss_per_1000_pages_at_100000",
        "read_bytes",
        "written_bytes",
        "syncs",
        "footprint_bytes_per_1000_pages",
        "sent_bytes",
        "received_bytes",
        "requests_sent",
        "tcp_connections",
        "tls_connections",
        "peak_threads",
        "peak_open_files",
        "startup_seconds",
        "executable_bytes",
        "pages",
        "duplicates",
        "errors",
        "skipped",
        "sample_share",
    ] {
        assert!(names.contains(&name), "{name} missing from {names:?}");
    }
    assert_eq!(
        cell(&report, 0, "foxhound")["curve"][0]["median"],
        json!(100.0)
    );
}

/// A metric a cell cannot give says why: a tool with no Record, a counted
/// cell or the growth cell that did not run.
#[test]
fn a_metric_with_no_figure_says_why() {
    let workspace = Workspace::from_fixture("empty");

    report(&workspace, &trials());

    let report = written(&workspace);
    let spider = &cell(&report, 0, "spider")["metrics"];
    assert_eq!(
        (
            &spider["first_record_seconds"]["status"],
            &spider["syncs"],
            &spider["rss_per_1000_pages_at_10000"],
            &spider["rss_per_1000_pages_at_1000"]["median"],
        ),
        (
            &json!("not_measured"),
            &json!({ "status": "not_measured", "reason": "strace is not installed on this host" }),
            &json!({ "status": "not_measured", "reason": "the growth cell runs with `--growth`" }),
            &json!(f64::from(20 * 1024 * 1024)),
        )
    );
}

/// Each cell's peak PSS over its process tree stands beside its peak RSS
/// (#2213): summed RSS counts a page two processes share twice, PSS once. A
/// cell with a trial whose sampler was refused a process's PSS says so
/// rather than give a sum that left the process out.
#[test]
fn each_cell_reports_its_trees_peak_pss_beside_its_peak_rss() {
    let workspace = Workspace::from_fixture("empty");
    let mut trials = trials();
    for trial in trials["trials"].as_array_mut().unwrap() {
        if trial["scenario"] == "browser" && trial["cell"] == "crawlee" && trial["number"] == 2 {
            trial["sampled"]["peak_pss_bytes"] = json!(null);
        }
    }

    let outcome = report(&workspace, &trials);

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    let report = written(&workspace);
    assert_eq!(
        (
            &cell(&report, 0, "foxhound")["metrics"]["peak_pss_bytes"],
            &cell(&report, 4, "crawlee")["metrics"]["peak_pss_bytes"],
        ),
        (
            &json!({ "status": "measured", "median": f64::from(19 * 1024 * 1024), "low": f64::from(9 * 1024 * 1024), "high": f64::from(29 * 1024 * 1024), "unit": "bytes" }),
            &json!({ "status": "not_measured", "reason": "a process of the tree kept its PSS from the sampler (its `/proc/<pid>/smaps_rollup` could not be read)" }),
        )
    );
    let markdown = workspace.read_text(MARKDOWN);
    let expected = "| Memory: peak RSS of the process tree | 20.0 MiB (10.0 MiB–30.0 MiB) |";
    let beside = "\n| Memory: peak PSS of the process tree | 19.0 MiB (9.0 MiB–29.0 MiB) |";
    let rss_line_end = markdown
        .find(expected)
        .and_then(|start| markdown[start..].find('\n').map(|end| start + end));
    assert!(
        rss_line_end.is_some_and(|end| markdown[end..].starts_with(beside)),
        "{markdown}"
    );
}

/// A cell with a trial that ended before the sampler's first sample, as a
/// 100-page crawl does, has no peak PSS, and says that rather than blame
/// the OS.
#[test]
fn a_trial_too_short_to_sample_leaves_its_cell_without_a_peak_pss() {
    let workspace = Workspace::from_fixture("empty");
    let mut trials = trials();
    for trial in trials["trials"].as_array_mut().unwrap() {
        if trial["scenario"] == "raw" && trial["cell"] == "spider" && trial["number"] == 1 {
            trial["sampled"]["samples"] = json!(0);
            trial["sampled"]["peak_pss_bytes"] = json!(null);
        }
    }

    let outcome = report(&workspace, &trials);

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    assert_eq!(
        cell(&written(&workspace), 0, "spider")["metrics"]["peak_pss_bytes"],
        json!({ "status": "not_measured", "reason": "a trial ended before the sampler's first sample of its process tree" })
    );
}

/// A crawl that ended within one window of the core sampling, as Spider's
/// 1,000 pages can, gives no peak of cores rather than none used.
#[test]
fn a_crawl_shorter_than_the_core_window_gives_no_peak_cores() {
    let workspace = Workspace::from_fixture("empty");
    let mut trials = trials();
    for trial in trials["trials"].as_array_mut().unwrap() {
        if trial["tool"] == "spider" {
            trial["sampled"]["peak_cores"] = json!(null);
        }
    }

    let outcome = report(&workspace, &trials);

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    let report = written(&workspace);
    assert_eq!(
        (
            &cell(&report, 0, "spider")["metrics"]["peak_cores"]["status"],
            &cell(&report, 0, "foxhound")["metrics"]["peak_cores"]["median"],
        ),
        (&json!("not_measured"), &json!(1.5))
    );
}

/// The report writes each scenario's mapping: the settings each tool ran
/// with, and the cells a tool cannot run.
#[test]
fn the_report_writes_each_scenarios_mapping() {
    let workspace = Workspace::from_fixture("empty");

    report(&workspace, &trials());

    let report = written(&workspace);
    assert_eq!(
        (
            &cell(&report, 1, "spider")["status"],
            &cell(&report, 1, "foxhound periodic")["status"],
        ),
        (&json!("not_supported"), &json!("measured"))
    );
    let markdown = workspace.read_text(MARKDOWN);
    for expected in [
        "## 1. Raw fetch and follow",
        "## 2. Durable and resumable",
        "`run_state.durability: off`",
        "`dataset.records: false`",
        "`run_state.durability: periodic` and `run_state.commit_interval: 60s`",
        "- **spider**: none",
        "not supported: Spider keeps no crawl state",
        "| Throughput: pages per second | 500.0 (250.0–1000.0) |",
    ] {
        assert!(
            markdown.contains(expected),
            "{expected} missing:\n{markdown}"
        );
    }
}

/// Crawlee, Scrapy and crawlee-rs join scenario 1, each cell with every
/// metric and the settings its tool ran with; Crawlee's and Scrapy's
/// scenario 2 cells say why neither resumes all the site's pages after a
/// kill, where crawlee-rs's is measured, and the report names the pinned
/// version or commit of each tool.
#[test]
fn crawlee_and_scrapy_cells_report_every_metric_with_their_mapping() {
    let workspace = Workspace::from_fixture("empty");

    let outcome = report(&workspace, &trials());

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    let report = written(&workspace);
    let names = |cell: &Value| {
        cell["metrics"]
            .as_object()
            .map(|metrics| metrics.keys().cloned().collect::<Vec<_>>())
    };
    let foxhound = names(cell(&report, 0, "foxhound"));
    for (scenario, name) in [
        (0, "crawlee"),
        (0, "scrapy"),
        (0, "crawlee-rs"),
        (1, "crawlee-rs"),
    ] {
        let cell = cell(&report, scenario, name);
        assert_eq!(
            (&cell["status"], names(cell)),
            (&json!("measured"), foxhound.clone()),
            "scenario {scenario}'s {name}"
        );
    }
    assert_eq!(
        (
            &cell(&report, 1, "crawlee")["status"],
            &cell(&report, 1, "scrapy")["status"]
        ),
        (&json!("not_supported"), &json!("not_supported"))
    );
    let markdown = workspace.read_text(MARKDOWN);
    for expected in [
        "Crawlee 3.18.2 (`@crawlee/cheerio`, and `@crawlee/playwright` on Playwright 1.64.1) on \
         Node.js, Scrapy 2.19.0 (and scrapy-playwright 0.0.48 on Playwright 1.64.1) on CPython, \
         crawlee-rs at commit `2a3ed90b8391c86005ce6ed8dd3e3248d0d34579` and pagesift 1.3.0, every \
         tool of the headless browser scenario running Chrome for Testing 156.0.8078.4",
        "- **crawlee**: `CheerioCrawler` (`@crawlee/cheerio`) with `persistStorage: false` and \
         `maxConcurrency: 16`",
        "- **scrapy**: a default `scrapy.Spider`",
        "`ROBOTSTXT_OBEY: False`, `CONCURRENT_REQUESTS: 16`, `CONCURRENT_REQUESTS_PER_DOMAIN: 16`, \
         `DOWNLOAD_DELAY: 0`",
        "not supported: Crawlee's `persistStorage` writes each request of its queue to a file",
        "not supported: Scrapy's `JOBDIR` writes its queue's index only on a clean close",
    ] {
        assert!(
            markdown.contains(expected),
            "{expected} missing:\n{markdown}"
        );
    }
}

/// pagesift joins the matrix's five scenarios (#2215): each of its cells
/// is measured with every metric, and the report names its pinned version
/// and its settings.
#[test]
fn pagesifts_cells_report_every_metric_with_their_mapping_and_its_pin() {
    let workspace = Workspace::from_fixture("empty");

    let outcome = report(&workspace, &trials());

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    let report = written(&workspace);
    let names = |cell: &Value| {
        cell["metrics"]
            .as_object()
            .map(|metrics| metrics.keys().cloned().collect::<Vec<_>>())
    };
    let foxhound = names(cell(&report, 0, "foxhound"));
    for scenario in 0..5 {
        let cell = cell(&report, scenario, "pagesift");
        assert_eq!(
            (&cell["status"], names(cell)),
            (&json!("measured"), foxhound.clone()),
            "scenario {scenario}'s pagesift"
        );
    }
    let markdown = workspace.read_text(MARKDOWN);
    for expected in [
        "crawlee-rs at commit `2a3ed90b8391c86005ce6ed8dd3e3248d0d34579` and pagesift 1.3.0, every",
        "- **pagesift**: its library's `Scraper::crawl`, the entry point its own crawl benchmark uses",
        "- **pagesift**: scenario 1's crawl as a job of its HTTP API (`POST /v1/crawl`)",
        "- **pagesift**: scenario 1's crawl on its browser engine (`engine: browser`)",
    ] {
        assert!(
            markdown.contains(expected),
            "{expected} missing:\n{markdown}"
        );
    }
}

/// Scenarios 3 to 5 (#1903): the polite crawl of the multi-domain site,
/// extraction to a dataset and the headless browser each report every
/// metric for each measured cell, with its mapping; Spider's polite cell
/// says why Spider cannot be polite to each domain.
#[test]
fn the_polite_extraction_and_browser_scenarios_report_every_metric_of_each_cell() {
    let workspace = Workspace::from_fixture("empty");

    let outcome = report(&workspace, &trials());

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    let report = written(&workspace);
    let names = |cell: &Value| {
        cell["metrics"]
            .as_object()
            .map(|metrics| metrics.keys().cloned().collect::<Vec<_>>())
    };
    let foxhound = names(cell(&report, 0, "foxhound"));
    let scenarios: Vec<&Value> = (2..5)
        .map(|index| &report["scenarios"][index]["scenario"])
        .collect();
    assert_eq!(
        scenarios,
        [&json!("polite"), &json!("extraction"), &json!("browser")]
    );
    for (index, cells) in [
        (
            2,
            &[
                "foxhound",
                "crawlee",
                "scrapy",
                "crawlee-rs",
                "foxhound auto",
            ][..],
        ),
        (
            3,
            &["foxhound", "spider", "crawlee", "scrapy", "crawlee-rs"][..],
        ),
        (4, &["foxhound", "spider", "crawlee", "scrapy"][..]),
    ] {
        for name in cells {
            let cell = cell(&report, index, name);
            assert_eq!(
                (&cell["status"], names(cell)),
                (&json!("measured"), foxhound.clone()),
                "scenario {index}'s {name}"
            );
        }
    }
    assert_eq!(
        (
            &cell(&report, 2, "spider")["status"],
            &cell(&report, 4, "crawlee-rs")["status"]
        ),
        (&json!("not_supported"), &json!("not_supported"))
    );
    let markdown = workspace.read_text(MARKDOWN);
    for expected in [
        "## 3. Polite: robots.txt obeyed, 1 in flight per domain, 1 s delay, on the multi-domain site (10,000 pages on 1,000 domains)",
        "## 4. Extraction to a dataset",
        "## 5. Headless browser",
        "- **crawlee**: `CheerioCrawler` with `sameDomainDelaySecs: 1`, `respectRobotsTxtFile: true`",
        "- **scrapy**: `scrapy-playwright`'s download handler for every request",
        "- **spider**: none: Spider's delay and robots.txt are its crawl's, not each domain's",
    ] {
        assert!(
            markdown.contains(expected),
            "{expected} missing:\n{markdown}"
        );
    }
}

/// The polite scenario's `auto` row (#2283) reports the share of its pages
/// that were samples, and is held to its pages' first answers, its samples
/// fetched twice; every other cell says why it has no share.
#[test]
fn the_polite_auto_row_reports_the_share_of_its_pages_that_were_samples() {
    let workspace = Workspace::from_fixture("empty");
    let mut trials = trials();
    trials["syscalls"] = strace();

    let outcome = report(&workspace, &trials);

    assert_eq!(
        (outcome.findings(), outcome.code),
        (vec![], 0),
        "{}",
        outcome.stderr
    );
    let report = written(&workspace);
    assert_eq!(
        (
            &cell(&report, 2, "foxhound auto")["row"],
            &cell(&report, 2, "foxhound auto")["metrics"]["sample_share"],
            &cell(&report, 2, "foxhound")["metrics"]["sample_share"]["status"],
        ),
        (
            &json!("sensitivity"),
            &json!({ "status": "measured", "median": 0.3, "low": 0.3, "high": 0.3, "unit": "share" }),
            &json!("not_measured"),
        )
    );
    let markdown = workspace.read_text(MARKDOWN);
    let expected = "| Tiers: share of the pages that were samples | not measured: it fetches no \
                    sample: only Foxhound's `auto` row does | 30.0 % (30.0 %–30.0 %) |";
    assert!(
        markdown.contains(expected),
        "{expected} missing:\n{markdown}"
    );
}

/// Each trial records the 1-minute load it started at, which the trials
/// table shows, and the report says how many timed trials started above a
/// quiet machine's 4.
#[test]
fn the_report_shows_the_load_each_trial_started_at() {
    let workspace = Workspace::from_fixture("empty");
    let mut trials = trials();
    for (index, trial) in trials["trials"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .enumerate()
    {
        trial["load"] = json!(if index == 0 { 6.5 } else { 1.25 });
    }

    let outcome = report(&workspace, &trials);

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    let markdown = workspace.read_text(MARKDOWN);
    for expected in [
        "| Scenario | Cell | Trial | Load at start | Seconds | Pages per second | Pages fetched | \
         Peak RSS |",
        "| raw | foxhound | 1 | 6.50 | 1.000 |",
        "| raw | foxhound | 2 | 1.25 | 2.000 |",
        "1 of the 97 timed trials started above a 1-minute load of 4.",
    ] {
        assert!(
            markdown.contains(expected),
            "{expected} missing:\n{markdown}"
        );
    }
}

/// A run of some scenarios only (`--scenario`) declares them: the
/// same-work check holds only their cells to their trials, and the report
/// gives only those scenarios.
#[test]
fn a_run_of_some_scenarios_reports_those_scenarios_only() {
    let workspace = Workspace::from_fixture("empty");
    let mut trials = trials();
    trials["declared"]["scenarios"] = json!(["polite", "browser"]);
    let kept: Vec<Value> = trials["trials"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|trial| trial["scenario"] == "polite" || trial["scenario"] == "browser")
        .cloned()
        .collect();
    trials["trials"] = json!(kept);
    trials["instructions"] = not_run("scenario 1 did not run");

    let outcome = report(&workspace, &trials);

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    let report = written(&workspace);
    let scenarios: Vec<&Value> = report["scenarios"]
        .as_array()
        .unwrap()
        .iter()
        .map(|scenario| &scenario["scenario"])
        .collect();
    assert_eq!(scenarios, [&json!("polite"), &json!("browser")]);
}

#[test]
fn the_system_call_cell_gives_each_cells_syncs() {
    let workspace = Workspace::from_fixture("empty");
    let mut trials = trials();
    trials["syscalls"] = strace();

    let outcome = report(&workspace, &trials);

    assert_eq!(
        (outcome.findings(), outcome.code),
        (vec![], 0),
        "{}",
        outcome.stderr
    );
    let report = written(&workspace);
    let durable = &cell(&report, 1, "foxhound full")["metrics"];
    assert_eq!(
        (
            &durable["syncs"]["median"],
            &durable["system_calls_per_page"]["median"],
        ),
        (&json!(7500.0), &json!(300.0))
    );
}

/// A run under strace that the comparison stopped, its crawl no longer
/// progressing, leaves its cell's system calls not measured with the
/// measured reason, and the cell short of its declared runs is no failure.
#[test]
fn a_cell_whose_strace_run_was_stopped_reports_its_system_calls_not_measured_with_why() {
    let workspace = Workspace::from_fixture("empty");
    let mut trials = trials();
    let mut syscalls = strace();
    syscalls["runs"]
        .as_array_mut()
        .unwrap()
        .retain(|run| !(run["scenario"] == "browser" && run["cell"] == "spider"));
    let why = "under `strace -f`, run 1 stopped after 400 s: the Proving ground received no request for 120 s after 240 requests";
    syscalls["stopped"] = json!([{
        "scenario": "browser",
        "cell": "spider",
        "tool": "spider",
        "number": 1,
        "reason": why,
    }]);
    trials["syscalls"] = syscalls;

    let outcome = report(&workspace, &trials);

    assert_eq!(
        (outcome.findings(), outcome.code),
        (vec![], 0),
        "{}",
        outcome.stderr
    );
    let report = written(&workspace);
    let not_measured = json!({ "status": "not_measured", "reason": why });
    let spider = &cell(&report, 4, "spider")["metrics"];
    let crawlee = &cell(&report, 4, "crawlee")["metrics"];
    assert_eq!(
        (
            &spider["syncs"],
            &spider["system_calls_per_page"],
            &crawlee["system_calls_per_page"]["median"],
        ),
        (&not_measured, &not_measured, &json!(300.0))
    );
}

/// A timed trial the comparison stopped once and ran again counts its
/// retry's figures, and the trials table marks that trial with why the
/// first attempt stopped.
#[test]
fn a_timed_trial_stopped_and_retried_is_marked_with_why() {
    let workspace = Workspace::from_fixture("empty");
    let mut trials = trials();
    let why = "timed trial 2 stopped after 130 s: the Proving ground received no request for 120 s after 0 requests";
    trials["trials"][1]["retried"] = json!(why);

    let outcome = report(&workspace, &trials);

    assert_eq!(
        (outcome.findings(), outcome.code),
        (vec![], 0),
        "{}",
        outcome.stderr
    );
    let markdown = workspace.read_text(MARKDOWN);
    let expected = format!("| raw | foxhound | 2 (retried: {why}) | not measured | 2.000 |");
    assert!(
        markdown.contains(&expected),
        "{expected} missing:\n{markdown}"
    );
}

/// A cell whose timed trial was stopped twice, its first attempt and its
/// retry, is not measured, with the measured reasons; it is not short of
/// its declared trials, and the run's other cells report as before.
#[test]
fn a_cell_whose_timed_trial_was_stopped_twice_is_not_measured_with_why() {
    let workspace = Workspace::from_fixture("empty");
    let mut trials = trials();
    trials["trials"]
        .as_array_mut()
        .unwrap()
        .retain(|trial| !(trial["scenario"] == "browser" && trial["cell"] == "crawlee"));
    let why = "timed trial 1 stopped after 600 s, the longest a timed trial may take, at 0 requests; its first attempt: timed trial 1 stopped after 130 s: the Proving ground received no request for 120 s after 0 requests";
    trials["stopped"] = json!([{
        "scenario": "browser",
        "cell": "crawlee",
        "tool": "crawlee",
        "number": 1,
        "reason": why,
    }]);

    let outcome = report(&workspace, &trials);

    assert_eq!(
        (outcome.findings(), outcome.code),
        (vec![], 0),
        "{}",
        outcome.stderr
    );
    let report = written(&workspace);
    assert_eq!(
        (
            &cell(&report, 4, "crawlee")["status"],
            &cell(&report, 4, "crawlee")["reason"],
            &cell(&report, 4, "scrapy")["status"],
        ),
        (&json!("not_measured"), &json!(why), &json!("measured"))
    );
    let markdown = workspace.read_text(MARKDOWN);
    let expected = format!("  - not measured: {why}.");
    assert!(
        markdown.contains(&expected),
        "{expected} missing:\n{markdown}"
    );
}

#[test]
fn a_tool_given_a_different_page_set_fails_the_same_work_check_and_reports_no_figure() {
    let workspace = Workspace::from_fixture("empty");
    let mut trials = trials();
    trials["trials"][5]["work"]["pages"] = json!(999);
    trials["trials"][5]["work"]["missing"] = json!(["/sites/1000/1000"]);

    let outcome = report(&workspace, &trials);

    assert_eq!(
        (outcome.findings(), outcome.code),
        (vec![finding("different_pages", "raw spider trial 3")], 1),
        "{}",
        outcome.stderr
    );
    assert_eq!(workspace.read(REPORT), None);
}

#[test]
fn a_trial_served_other_bytes_for_the_same_pages_fails_the_same_work_check() {
    let workspace = Workspace::from_fixture("empty");
    let mut trials = trials();
    trials["trials"][1]["work"]["bytes"] = json!(600_001);

    let outcome = report(&workspace, &trials);

    assert_eq!(
        (outcome.findings(), outcome.code),
        (vec![finding("different_bytes", "raw foxhound trial 2")], 1),
        "{}",
        outcome.stderr
    );
}

/// A resumed crawl's pages fetched again after its kill are no other work,
/// but other bytes for a page's first answer are.
#[test]
fn a_resumed_trial_is_held_to_its_pages_first_answers() {
    let workspace = Workspace::from_fixture("empty");
    let mut trials = trials();
    let resumed = trials["trials"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|trial| trial["cell"] == "foxhound full" && trial["number"] == 2)
        .unwrap();
    resumed["work"]["first_bytes"] = json!(600_001);

    let outcome = report(&workspace, &trials);

    assert_eq!(
        (outcome.findings(), outcome.code),
        (
            vec![finding("different_bytes", "durable foxhound full trial 2")],
            1
        ),
        "{}",
        outcome.stderr
    );
}

#[test]
fn a_cell_short_of_its_declared_trials_fails() {
    let workspace = Workspace::from_fixture("empty");
    let mut trials = trials();
    trials["trials"].as_array_mut().unwrap().remove(5);

    let outcome = report(&workspace, &trials);

    assert_eq!(
        (outcome.findings(), outcome.code),
        (vec![finding("trial_count", "raw spider")], 1),
        "{}",
        outcome.stderr
    );
}

/// The matrix asks for at least 10 trials of each cell: a report of fewer,
/// a quicker try's, says it falls short, and one of 10 does not.
#[test]
fn a_report_of_fewer_than_10_trials_says_it_falls_short() {
    let short = Workspace::from_fixture("empty");
    let enough = Workspace::from_fixture("empty");
    let mut ten = trials();
    ten["declared"]["trials"] = json!(10);
    ten["trials"] = CELLS
        .iter()
        .flat_map(|(scenario, cell, tool)| {
            (1..=runs(scenario, cell, 10))
                .map(move |number| cell_trial(scenario, cell, tool, number, 2.0, 20 * MIB))
        })
        .collect();

    let outcomes = [report(&short, &trials()), report(&enough, &ten)];

    assert_eq!(
        outcomes.each_ref().map(|outcome| outcome.code),
        [0, 0],
        "{}{}",
        outcomes[0].stderr,
        outcomes[1].stderr
    );
    assert_eq!(
        (
            &written(&short)["enough_trials"],
            &written(&enough)["enough_trials"]
        ),
        (&json!(false), &json!(true))
    );
    let fell_short = "short of the 10 the matrix asks";
    assert_eq!(
        (
            short.read_text(MARKDOWN).contains(fell_short),
            enough.read_text(MARKDOWN).contains(fell_short),
        ),
        (true, false),
        "{}",
        short.read_text(MARKDOWN)
    );
}

#[test]
fn the_callgrind_cell_reports_foxhounds_instructions_per_page_over_spiders() {
    let workspace = Workspace::from_fixture("empty");
    let mut trials = trials();
    trials["instructions"] = callgrind();

    let outcome = report(&workspace, &trials);

    assert_eq!(
        (outcome.findings(), outcome.code),
        (vec![], 0),
        "{}",
        outcome.stderr
    );
    assert_eq!(
        written(&workspace)["instructions"]["ratios"][0],
        json!({
            "row": "foxhound.spider_instruction_ratio",
            "scenario": "scenario 1 of ADR 0138's matrix",
            "foxhound": "foxhound",
            "foxhound_per_page": 32_000.0,
            "spider_per_page": 1_000.0,
            "ratio": 32.0,
        })
    );
    let markdown = workspace.read_text(MARKDOWN);
    assert!(
        markdown.contains("`foxhound.spider_instruction_ratio`: 32.000"),
        "{markdown}"
    );
}

/// The Callgrind cell counts the extraction scenario too (#2136; the
/// owner's decision of 2026-11-10, #2252 Q7), both of Foxhound's rows over
/// Spider's: at its defaults, where Spider does not do the Clean-up rules'
/// work, so its output differs and the row says so; and with its Filter
/// lists off, where both tools read the same Main content and Spider's
/// output passes that row's check, the ad allowed.
#[test]
fn the_callgrind_cell_reports_the_extraction_scenarios_two_ratios() {
    let workspace = Workspace::from_fixture("empty");
    let mut trials = trials();
    trials["instructions"] = callgrind();

    let outcome = report(&workspace, &trials);

    assert_eq!(
        (outcome.findings(), outcome.code),
        (vec![], 0),
        "{}",
        outcome.stderr
    );
    assert_eq!(
        written(&workspace)["instructions"]["ratios"]
            .as_array()
            .unwrap()[1..],
        [
            json!({
                "row": "foxhound.spider_extraction_instruction_ratio",
                "scenario": "the extraction scenario of ADR 0138's matrix, Foxhound at its defaults",
                "foxhound": "foxhound",
                "foxhound_per_page": 60_000.0,
                "spider_per_page": 50_000.0,
                "ratio": 1.2,
                "spider_output_differs": "output differs: 1000 of 1000",
            }),
            json!({
                "row": "foxhound.spider_extraction_instruction_ratio_filter_lists_off",
                "scenario": "the extraction scenario of ADR 0138's matrix, Foxhound's Filter lists off",
                "foxhound": "foxhound filter lists off",
                "foxhound_per_page": 45_000.0,
                "spider_per_page": 50_000.0,
                "ratio": 0.9,
            }),
        ]
    );
    let markdown = workspace.read_text(MARKDOWN);
    for expected in [
        "`foxhound.spider_extraction_instruction_ratio`: 1.200, Foxhound's 60000 instructions per page over Spider's 50000",
        "Spider's output differs under this row's check (output differs: 1000 of 1000)",
        "`foxhound.spider_extraction_instruction_ratio_filter_lists_off`: 0.900, Foxhound's 45000 instructions per page over Spider's 50000",
    ] {
        assert!(
            markdown.contains(expected),
            "{expected} missing:\n{markdown}"
        );
    }
}

/// A run that asked for the extraction scenario alone gives its two ratios
/// and no scenario 1 ratio, which it did not count.
#[test]
fn a_run_of_the_extraction_scenario_alone_gives_its_ratios_only() {
    let workspace = Workspace::from_fixture("empty");
    let mut trials = trials();
    trials["declared"]["scenarios"] = json!(["extraction"]);
    trials["trials"]
        .as_array_mut()
        .unwrap()
        .retain(|trial| trial["scenario"] == "extraction");
    let mut cell = callgrind();
    cell["runs"]
        .as_array_mut()
        .unwrap()
        .retain(|run| run["scenario"] == "extraction");
    trials["instructions"] = cell;

    let outcome = report(&workspace, &trials);

    assert_eq!(
        (outcome.findings(), outcome.code),
        (vec![], 0),
        "{}",
        outcome.stderr
    );
    let report = written(&workspace);
    let rows: Vec<&Value> = report["instructions"]["ratios"]
        .as_array()
        .unwrap()
        .iter()
        .map(|ratio| &ratio["row"])
        .collect();
    assert_eq!(
        rows,
        [
            &json!("foxhound.spider_extraction_instruction_ratio"),
            &json!("foxhound.spider_extraction_instruction_ratio_filter_lists_off"),
        ]
    );
}

/// A Foxhound trial whose pages differ from the right answer is refused
/// (#2136): the report fails naming it, and writes no figure.
#[test]
fn a_foxhound_trial_whose_output_differs_is_refused() {
    let workspace = Workspace::from_fixture("empty");
    let mut trials = trials();
    let boilerplate = "/sites/1000/7: the text holds \"Tidewater Supply masthead\"";
    for trial in trials["trials"].as_array_mut().unwrap() {
        if trial["scenario"] == "extraction" && trial["cell"] == "foxhound" && trial["number"] == 2
        {
            trial["extracted"] = extracted(3, 3, boilerplate);
        }
    }

    let outcome = report(&workspace, &trials);

    assert_eq!(
        (outcome.findings(), outcome.code),
        (
            vec![finding("output_differs", "extraction foxhound trial 2")],
            1
        ),
        "{}",
        outcome.stderr
    );
    assert!(
        outcome.stdout.contains(
            "3 of the site's 1000 pages differ from the right answer; the first: /sites/1000/7: the text holds"
        ),
        "{}",
        outcome.stdout
    );
    assert_eq!(workspace.read(REPORT), None);
}

/// Foxhound's row with its Filter lists off is held to the right answer
/// with the ad allowed: the ad on every page refuses nothing (the
/// fixture's own trials), and a page that differs otherwise does. At its
/// defaults, the ad's marker on a page refuses the trial.
#[test]
fn foxhounds_rows_are_each_held_to_their_own_check() {
    let refused = |cell: &str, found: Value| {
        let workspace = Workspace::from_fixture("empty");
        let mut trials = trials();
        for trial in trials["trials"].as_array_mut().unwrap() {
            if trial["scenario"] == "extraction" && trial["cell"] == cell && trial["number"] == 1 {
                trial["extracted"] = found.clone();
            }
        }
        let outcome = report(&workspace, &trials);
        (outcome.findings(), outcome.code)
    };
    let title = "/sites/1000/3: title \"Oak &amp; Co.\", not \"Oak & Co.\"";

    assert_eq!(
        (
            refused("foxhound filter lists off", extracted(1000, 2, title)),
            refused("foxhound", extracted(1000, 0, AD)),
        ),
        (
            (
                vec![finding(
                    "output_differs",
                    "extraction foxhound filter lists off trial 1"
                )],
                1
            ),
            (
                vec![finding("output_differs", "extraction foxhound trial 1")],
                1
            ),
        )
    );
}

/// A trial on the article site that holds no result of the output check
/// is no figure: the report fails naming it.
#[test]
fn an_extraction_trial_the_output_check_did_not_read_is_refused() {
    let workspace = Workspace::from_fixture("empty");
    let mut trials = trials();
    for trial in trials["trials"].as_array_mut().unwrap() {
        if trial["scenario"] == "extraction" && trial["cell"] == "scrapy" && trial["number"] == 3 {
            trial.as_object_mut().unwrap().remove("extracted");
        }
    }

    let outcome = report(&workspace, &trials);

    assert_eq!(
        (outcome.findings(), outcome.code),
        (
            vec![finding("output_unchecked", "extraction scrapy trial 3")],
            1
        ),
        "{}",
        outcome.stderr
    );
    assert_eq!(workspace.read(REPORT), None);
}

/// A competitor's trial whose pages differ keeps its figures, labelled
/// `output differs: N of <pages>` with the first difference, its worst
/// trial's. Its standing against a Foxhound row is labelled when its
/// output differs under that row's check: Spider, which removes no ad,
/// against Foxhound at its defaults, and not against Foxhound with its
/// Filter lists off, held with the ad allowed.
#[test]
fn a_competitors_trial_whose_output_differs_keeps_its_figures_labelled() {
    let workspace = Workspace::from_fixture("empty");
    let mut trials = trials();
    for trial in trials["trials"].as_array_mut().unwrap() {
        if trial["scenario"] == "extraction" && trial["cell"] == "spider" {
            trial["extracted"] = extracted(1000, 0, AD);
        }
        if trial["scenario"] == "extraction" && trial["cell"] == "crawlee" && trial["number"] == 2 {
            trial["extracted"] = extracted(4, 4, "/sites/1000/9: the text lacks paragraph 2");
        }
    }

    let outcome = report(&workspace, &trials);

    assert_eq!(
        (outcome.findings(), outcome.code),
        (vec![], 0),
        "{}",
        outcome.stderr
    );
    let report = written(&workspace);
    let labels: Vec<(&str, &str, &Value)> = report["scenarios"][3]["standings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|standings| {
            (
                standings["product"].as_str().unwrap(),
                standings["competitor"].as_str().unwrap(),
                &standings["output_differs"],
            )
        })
        .collect();
    let (differs, crawlee) = (
        json!("output differs: 1000 of 1000"),
        json!("output differs: 4 of 1000"),
    );
    assert_eq!(
        (
            &cell(&report, 3, "spider")["status"],
            &cell(&report, 3, "spider")["output"],
            &cell(&report, 3, "scrapy")["output"],
            &cell(&report, 0, "spider")["output"],
            labels,
        ),
        (
            &json!("measured"),
            &extracted(1000, 0, AD),
            &extracted(0, 0, ""),
            &Value::Null,
            vec![
                ("foxhound", "spider", &differs),
                ("foxhound", "crawlee", &crawlee),
                ("foxhound", "scrapy", &Value::Null),
                ("foxhound", "crawlee-rs", &Value::Null),
                ("foxhound", "pagesift", &Value::Null),
                ("foxhound filter lists off", "spider", &Value::Null),
                ("foxhound filter lists off", "crawlee", &crawlee),
                ("foxhound filter lists off", "scrapy", &Value::Null),
                ("foxhound filter lists off", "crawlee-rs", &Value::Null),
                ("foxhound filter lists off", "pagesift", &Value::Null),
            ],
        )
    );
    let markdown = workspace.read_text(MARKDOWN);
    for expected in [
        "  - output differs: 1000 of 1000; the first: /sites/1000/1: the text holds the ad's \"Sponsored offer from Harbourline Freight\".",
        "  - output differs: 4 of 1000; the first: /sites/1000/9: the text lacks paragraph 2.",
        "  - output: every page's title and Main content read right.",
        "  - output: every page's title and Main content read right, the ad allowed.",
        "| spider, 1000 of the site's 1000 pages, output differs: 1000 of 1000 |",
        "foxhound (1000 of 1000 pages) against spider (1000 of 1000 pages, output differs: 1000 of 1000) |",
        "foxhound filter lists off (1000 of 1000 pages) against spider (1000 of 1000 pages) |",
    ] {
        assert!(
            markdown.contains(expected),
            "{expected} missing:\n{markdown}"
        );
    }
}

#[test]
fn a_cell_that_did_not_run_says_why_and_gives_no_ratio() {
    let workspace = Workspace::from_fixture("empty");

    let outcome = report(&workspace, &trials());

    assert_eq!((outcome.findings(), outcome.code), (vec![], 0));
    assert_eq!(
        written(&workspace)["instructions"],
        not_run("valgrind is not installed on this host")
    );
    assert!(
        workspace
            .read_text(MARKDOWN)
            .contains("`foxhound.spider_instruction_ratio`: not run: valgrind is not installed")
    );
}

#[test]
fn a_callgrind_run_of_other_pages_fails_the_same_work_check_and_reports_no_figure() {
    let workspace = Workspace::from_fixture("empty");
    let mut cell = callgrind();
    cell["runs"][4]["work"]["pages"] = json!(999);
    let mut trials = trials();
    trials["instructions"] = cell;

    let outcome = report(&workspace, &trials);

    assert_eq!(
        (outcome.findings(), outcome.code),
        (
            vec![finding("different_pages", "raw spider callgrind trial 2")],
            1
        ),
        "{}",
        outcome.stderr
    );
    assert_eq!(workspace.read(REPORT), None);
}

#[test]
fn a_callgrind_run_served_other_bytes_fails_the_same_work_check() {
    let workspace = Workspace::from_fixture("empty");
    let mut cell = callgrind();
    cell["runs"][1]["work"]["bytes"] = json!(600_001);
    let mut trials = trials();
    trials["instructions"] = cell;

    let outcome = report(&workspace, &trials);

    assert_eq!(
        (outcome.findings(), outcome.code),
        (
            vec![finding("different_bytes", "raw foxhound callgrind trial 2")],
            1
        ),
        "{}",
        outcome.stderr
    );
    assert_eq!(workspace.read(REPORT), None);
}

#[test]
fn a_tool_short_of_its_declared_callgrind_runs_fails() {
    let workspace = Workspace::from_fixture("empty");
    let mut cell = callgrind();
    cell["runs"].as_array_mut().unwrap().pop();
    let mut trials = trials();
    trials["instructions"] = cell;

    let outcome = report(&workspace, &trials);

    assert_eq!(
        (outcome.findings(), outcome.code),
        (
            vec![finding("trial_count", "extraction spider callgrind")],
            1
        ),
        "{}",
        outcome.stderr
    );
}

/// A Spider count of 0 instructions is no measurement, and no ratio follows
/// from it.
#[test]
fn a_tool_that_counted_no_instructions_fails() {
    let workspace = Workspace::from_fixture("empty");
    let mut cell = callgrind();
    cell["runs"][3]["count"] = json!(0);
    let mut trials = trials();
    trials["instructions"] = cell;

    let outcome = report(&workspace, &trials);

    assert_eq!(
        (outcome.findings(), outcome.code),
        (
            vec![finding("nothing_counted", "raw spider callgrind trial 1")],
            1
        ),
        "{}",
        outcome.stderr
    );
}

/// The Collie scenario's cells, each with its pages and actions.
const COLLIE_CELLS: [(&str, &str, &[u32], u64); 5] = [
    (
        "click",
        "actionability",
        &[1, 2, 3, 4, 5, 6, 7, 8, 9, 18, 19, 26, 27],
        62,
    ),
    ("form", "actionability", &[20, 21, 22, 23, 24, 28], 22),
    ("navigation", "portal", &[6, 7, 8, 9, 10, 11, 12, 13], 8),
    ("sessions", "", &[], 10),
    (
        "at_once",
        "actionability",
        &[1, 2, 3, 4, 5, 6, 15, 18, 26, 27, 20, 21, 22, 23, 24, 28],
        81,
    ),
];

/// The steps the Collie scenario plans on `family`'s page `number`: a
/// form's fields and its submit, fifty clicks on page 26's counter, and
/// one click, press or read on any other.
fn collie_steps(family: &str, number: u32) -> u64 {
    match (family, number) {
        ("actionability", 20..=24) => 4,
        ("actionability", 28) => 2,
        ("actionability", 26) => 50,
        _ => 1,
    }
}

/// A Collie scenario of two trials of each tool in each cell, every page
/// done and verified, Collie sending `collie` commands per action and
/// Playwright one.
fn collie(collie: u64) -> Value {
    let mut trials_run = Vec::new();
    for (exercise, family, numbers, actions) in COLLIE_CELLS {
        for (tool, per_action) in [("collie", collie), ("playwright", 1)] {
            for number in 1..=2 {
                let pages: Vec<Value> = numbers
                    .iter()
                    .map(|page| {
                        let task = format!("{family}/{page}");
                        json!({
                            "task": task,
                            "ended": "success",
                            "status": 200,
                            "steps": collie_steps(family, *page),
                            "read": (family == "portal").then(|| format!("digest of {task}")),
                            "error": null,
                            "outcome": (family == "actionability").then_some("verified"),
                        })
                    })
                    .collect();
                trials_run.push(json!({
                    "exercise": exercise,
                    "tool": tool,
                    "number": number,
                    "actions": actions,
                    "sessions": if exercise == "sessions" { 10 } else { 0 },
                    "commands": actions * per_action,
                    "cpu_seconds": 0.5,
                    "wall_seconds": 1.0,
                    "peak_bytes": 100 * MIB,
                    "pages": pages,
                }));
            }
        }
    }
    json!({ "status": "measured", "trials": 2, "trials_run": trials_run })
}

/// TESTING.md's register, its Collie row's Today `today`.
fn register(today: &str) -> String {
    format!(
        "# Testing\n\n## Quality register\n\n### Enforced\n\n| Dimension | Rule | Checked by |\n|---|---|---|\n| a | b | c |\n\n### Measured, not yet enforced\n\n| Row | Metric, per unit | Contract | Today | Benchmark | Ratchet | Checked by | Run by |\n|---|---|---|---|---|---|---|---|\n| `collie.playwright_command_ratio` | Collie's CDP commands per action over Playwright's | ≤ 1 | {today} | the Collie scenario | must not grow | `just compare` | owner's report |\n\n### Exceptions\n\n| ID | Rule | Expires |\n|---|---|---|\n| x | y | z |\n"
    )
}

#[test]
fn the_collie_scenario_reports_its_command_ratio_against_the_rows_today() {
    let workspace = Workspace::from_fixture("empty");
    workspace.write("TESTING.md", register("1.000").as_bytes());
    let mut trials = trials();
    trials["collie"] = collie(2);

    let outcome = report(&workspace, &trials);

    assert_eq!(
        (outcome.findings(), outcome.code),
        (vec![], 0),
        "{}",
        outcome.stderr
    );
    let section = &written(&workspace)["collie"];
    assert_eq!(section["status"], "measured");
    assert_eq!(section["ratios"][0]["exercise"], "click");
    assert_eq!(section["ratios"][0]["ratio"], 2.0);
    assert_eq!(section["ratios"][0]["over_today"], true);
    assert_eq!(
        section["pages_right"]["collie"],
        json!({"right": 20, "ran": 20, "not_right": []})
    );
    let markdown = workspace.read_text(MARKDOWN);
    assert!(
        markdown.contains("over the row's Today, 1.000, in: click (2.000), form (2.000)"),
        "{markdown}"
    );
}

#[test]
fn a_collie_trial_of_other_work_fails_the_same_work_check_and_reports_no_figure() {
    let workspace = Workspace::from_fixture("empty");
    let mut trials = trials();
    let mut cell = collie(1);
    // Playwright's second trial read another text on a portal page.
    let run = cell["trials_run"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|run| {
            run["exercise"] == "navigation" && run["tool"] == "playwright" && run["number"] == 2
        })
        .unwrap();
    run["pages"][0]["read"] = json!("another digest");
    trials["collie"] = cell;

    let outcome = report(&workspace, &trials);

    assert_eq!(
        (outcome.findings(), outcome.code),
        (
            vec![finding(
                "different_work",
                "collie navigation playwright trial 2"
            )],
            1
        ),
        "{}",
        outcome.stdout
    );
    assert!(workspace.read(REPORT).is_none());
}
