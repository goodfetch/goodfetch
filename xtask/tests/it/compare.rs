//! The comparison matrix (ADRs 0134 and 0138): `xtask compare report` reads
//! the trials `xtask compare run` wrote and gives, for each scenario, each
//! cell's mapping and every metric of the owner's list as the median of its
//! trials with their spread, labelled `local-controlled` with the machine,
//! only once every trial of every measured cell fetched the same pages and
//! was served the same bytes; otherwise it fails naming the trial that did
//! other work, and writes no figure. A cell a tool cannot run says so, a
//! Foxhound value that does not exist yet is pending, and the Callgrind
//! cell gives `foxhound.spider_instruction_ratio` under the same check (ADR
//! 0140). Trials are fixtures here.

use serde_json::{Value, json};

use crate::workspace::{Workspace, finding};

const TRIALS: &str = "target/compare/trials.json";
const REPORT: &str = "target/compare/report.json";
const MARKDOWN: &str = "target/compare/report.md";
const MIB: u64 = 1024 * 1024;

/// The measured cells, by scenario.
const CELLS: [(&str, &str, &str); 16] = [
    ("raw", "foxhound", "foxhound"),
    ("raw", "spider", "spider"),
    ("raw", "crawlee", "crawlee"),
    ("raw", "scrapy", "scrapy"),
    ("durable", "foxhound full", "foxhound"),
    ("polite", "foxhound", "foxhound"),
    ("polite", "crawlee", "crawlee"),
    ("polite", "scrapy", "scrapy"),
    ("extraction", "foxhound", "foxhound"),
    ("extraction", "spider", "spider"),
    ("extraction", "crawlee", "crawlee"),
    ("extraction", "scrapy", "scrapy"),
    ("browser", "foxhound", "foxhound"),
    ("browser", "spider", "spider"),
    ("browser", "crawlee", "crawlee"),
    ("browser", "scrapy", "scrapy"),
];

/// The work of a trial that fetched each page of the 1,000-page site once,
/// 600 bytes each, and `again` pages a second time.
fn work(again: u64) -> Value {
    json!({
        "pages": 1000,
        "missing": [],
        "extra": [],
        "requests": 1000 + again,
        "bytes": 600 * (1000 + again),
        "first_bytes": 600_000,
    })
}

/// A trial of `cell` in `seconds`, at `peak_bytes`, that did `work`, its
/// figures otherwise the same for every trial: 2 s of user and 1 s of
/// system time, 8 MiB written, 1 MiB left on disk, its tree sampled at
/// half its peak on average.
fn trial(
    scenario: &str,
    cell: &str,
    tool: &str,
    number: u32,
    seconds: f64,
    peak_bytes: u64,
    work: &Value,
) -> Value {
    json!({
        "scenario": scenario,
        "cell": cell,
        "tool": tool,
        "number": number,
        "site_pages": 1000,
        "seconds": seconds,
        "processes": if scenario == "durable" { 2 } else { 1 },
        "user_seconds": 2.0,
        "system_seconds": 1.0,
        "peak_bytes": peak_bytes,
        "sampled": {
            "samples": 40,
            "peak_bytes": peak_bytes,
            "average_bytes": peak_bytes / 2,
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
    })
}

/// A cell's three trials: in 1, 2 and 4 seconds at 20, 30 and 10 MiB.
fn cell_trials(scenario: &str, cell: &str, tool: &str) -> Vec<Value> {
    [(1.0, 20), (2.0, 30), (4.0, 10)]
        .into_iter()
        .zip(1..)
        .map(|((seconds, mib), number)| {
            let again = if scenario == "durable" {
                u64::from(number)
            } else {
                0
            };
            trial(
                scenario,
                cell,
                tool,
                number,
                seconds,
                mib * MIB,
                &work(again),
            )
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
            "browser": "156.0.8078.4",
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
        "syscalls": not_run("strace is not installed on this host"),
        "growth": not_run("the growth cell runs with `--growth`"),
        "instructions": not_run("valgrind is not installed on this host"),
    })
}

/// A counted run of `cell` counting `count`, with `syncs`, over the site.
fn run(scenario: &str, cell: &str, tool: &str, number: u32, count: u64, syncs: u64) -> Value {
    json!({
        "scenario": scenario,
        "cell": cell,
        "tool": tool,
        "number": number,
        "count": count,
        "syncs": syncs,
        "work": work(0),
    })
}

/// A Callgrind cell of three runs of each tool over the 1,000-page site:
/// Foxhound's a median of 32,000 instructions a page, Spider's 1,000.
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
        ],
    })
}

/// A system-call cell of three runs of each measured cell: Foxhound's
/// durable crawl syncing 7,000, 7,500 and 8,000 times.
fn strace() -> Value {
    let mut runs = Vec::new();
    for (scenario, cell, tool) in CELLS {
        for (number, syncs) in [(1, 7000), (2, 7500), (3, 8000)] {
            let syncs = if cell == "foxhound full" { syncs } else { 0 };
            runs.push(run(scenario, cell, tool, number, 300_000, syncs));
        }
    }
    json!({ "status": "measured", "trials": 3, "runs": runs })
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
        })
    );
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
/// with, the Foxhound values still pending, and the cells a tool cannot
/// run.
#[test]
fn the_report_writes_each_scenarios_mapping() {
    let workspace = Workspace::from_fixture("empty");

    report(&workspace, &trials());

    let report = written(&workspace);
    assert_eq!(
        (
            &cell(&report, 1, "spider")["status"],
            &cell(&report, 1, "foxhound periodic")["status"],
            cell(&report, 0, "foxhound")["pending"]
                .as_array()
                .unwrap()
                .len(),
        ),
        (&json!("not_supported"), &json!("pending"), 2)
    );
    let markdown = workspace.read_text(MARKDOWN);
    for expected in [
        "## 1. Raw fetch and follow",
        "## 2. Durable and resumable",
        "`run_state.durability: fast`",
        "pending: `run_state.durability: off` (#1927)",
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

/// Crawlee and Scrapy join scenario 1, each cell with every metric and the
/// settings its tool ran with; their scenario 2 cells say why neither
/// resumes all the site's pages after a kill, and the report names the
/// pinned version of each tool.
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
    for (scenario, name) in [(0, "crawlee"), (0, "scrapy")] {
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
         Node.js and Scrapy 2.19.0 (and scrapy-playwright 0.0.48 on Playwright 1.64.1) on CPython, \
         every tool of the headless browser scenario running Chrome for Testing 156.0.8078.4",
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
        (2, &["foxhound", "crawlee", "scrapy"][..]),
        (3, &["foxhound", "spider", "crawlee", "scrapy"][..]),
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
        &cell(&report, 2, "spider")["status"],
        &json!("not_supported")
    );
    let markdown = workspace.read_text(MARKDOWN);
    for expected in [
        "## 3. Polite: robots.txt obeyed, 1 in flight per domain, 1 s delay, on the multi-domain site (50 domains)",
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
        "| Scenario | Cell | Trial | Load at start | Seconds | Pages per second | Peak RSS |",
        "| raw | foxhound | 1 | 6.50 | 1.000 |",
        "| raw | foxhound | 2 | 1.25 | 2.000 |",
        "1 of the 48 timed trials started above a 1-minute load of 4.",
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
    trials["trials"][13]["work"]["first_bytes"] = json!(600_001);

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
            (1..=10).map(move |number| {
                let again = u64::from(*scenario == "durable");
                trial(scenario, cell, tool, number, 2.0, 20 * MIB, &work(again))
            })
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
        written(&workspace)["instructions"],
        json!({
            "status": "measured",
            "row": "foxhound.spider_instruction_ratio",
            "scenario": "scenario 1 of ADR 0138's matrix",
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
fn a_tool_short_of_its_declared_callgrind_runs_fails() {
    let workspace = Workspace::from_fixture("empty");
    let mut cell = callgrind();
    cell["runs"].as_array_mut().unwrap().pop();
    let mut trials = trials();
    trials["instructions"] = cell;

    let outcome = report(&workspace, &trials);

    assert_eq!(
        (outcome.findings(), outcome.code),
        (vec![finding("trial_count", "raw spider callgrind")], 1),
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
