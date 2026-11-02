//! Spider's comparison (ADR 0134): `xtask compare-spider report` reads the
//! trials `xtask compare-spider run` wrote and gives each tool's median
//! pages per second and peak resident memory, labelled `local-controlled`
//! with the machine, only once every trial of both tools fetched the same
//! pages and was served the same bytes; otherwise it fails naming the trial
//! that did other work, and writes no figure. Its Callgrind cell gives
//! `foxhound.spider_instruction_ratio` under the same check (ADR 0140). Trials
//! are fixtures here.

use serde_json::{Value, json};

use crate::workspace::{Workspace, finding};

const TRIALS: &str = "target/compare-spider/trials.json";
const REPORT: &str = "target/compare-spider/report.json";

/// The paths of the site of `size` pages.
fn pages(size: u32) -> Vec<String> {
    (1..=size)
        .map(|page| format!("/sites/{size}/{page}"))
        .collect()
}

fn trial(tool: &str, number: u32, seconds: f64, peak_bytes: u64, pages: &[String]) -> Value {
    json!({
        "tool": tool,
        "number": number,
        "seconds": seconds,
        "peak_bytes": peak_bytes,
        "pages": pages,
        "requests": pages.len(),
        "bytes": 600 * pages.len(),
    })
}

/// A trials file of three trials of each tool over the 1,000-page site:
/// Foxhound's in 1, 2 and 4 seconds at 20, 30 and 10 MiB, Spider's in 2
/// seconds each at 40 MiB.
fn trials() -> Value {
    let site = pages(1000);
    let mib = 1024 * 1024;
    json!({
        "declared": {
            "trials": 3,
            "statistic": "median",
            "concurrency": 16,
            "site": "/sites/1000/1",
            "spider": "2.53.9",
        },
        "machine": {
            "class": "local-controlled",
            "os": "macos 27.0.1",
            "arch": "aarch64",
            "cpu": "Apple M3 Max",
            "threads": 14,
        },
        "commit": "0123456789abcdef0123456789abcdef01234567",
        "order_seed": 7,
        "trials": [
            trial("foxhound", 1, 1.0, 20 * mib, &site),
            trial("spider", 1, 2.0, 40 * mib, &site),
            trial("spider", 2, 2.0, 40 * mib, &site),
            trial("foxhound", 2, 2.0, 30 * mib, &site),
            trial("foxhound", 3, 4.0, 10 * mib, &site),
            trial("spider", 3, 2.0, 40 * mib, &site),
        ],
        "instructions": { "status": "not_run", "reason": "Valgrind is not installed on this host" },
    })
}

/// A run of the Callgrind cell counting `instructions` over `pages`.
fn run(tool: &str, number: u32, instructions: u64, pages: &[String]) -> Value {
    json!({
        "tool": tool,
        "number": number,
        "instructions": instructions,
        "pages": pages,
        "requests": pages.len(),
        "bytes": 600 * pages.len(),
    })
}

/// A Callgrind cell of three runs of each tool over the 1,000-page site:
/// Foxhound's a median of 32,000 instructions a page, Spider's 1,000.
fn measured_cell() -> Value {
    let site = pages(1000);
    json!({
        "status": "measured",
        "scenario": "scenario 1 of ADR 0138's matrix",
        "trials": 3,
        "runs": [
            run("foxhound", 1, 31_000_000, &site),
            run("foxhound", 2, 32_000_000, &site),
            run("foxhound", 3, 40_000_000, &site),
            run("spider", 1, 1_000_000, &site),
            run("spider", 2, 900_000, &site),
            run("spider", 3, 1_100_000, &site),
        ],
    })
}

fn trials_with(cell: Value) -> Value {
    let mut trials = trials();
    trials["instructions"] = cell;
    trials
}

fn report(workspace: &Workspace, trials: &Value) -> crate::workspace::Outcome {
    workspace.write(TRIALS, trials.to_string().as_bytes());
    workspace.xtask(&["compare-spider", "report", "--trials", TRIALS])
}

#[test]
fn trials_of_the_same_work_report_each_tools_medians_labelled_with_the_machine() {
    let workspace = Workspace::from_fixture("empty");

    let outcome = report(&workspace, &trials());

    assert_eq!(
        (outcome.findings(), outcome.code),
        (vec![], 0),
        "{}",
        outcome.stderr
    );
    let written: Value = serde_json::from_slice(&workspace.read(REPORT).unwrap()).unwrap();
    assert_eq!(
        (
            &written["label"],
            &written["machine"]["cpu"],
            &written["same_work"],
            &written["tools"],
        ),
        (
            &json!("local-controlled"),
            &json!("Apple M3 Max"),
            &json!({ "pages": 1000, "bytes": 600_000, "trials": 6 }),
            &json!({
                "foxhound": { "pages_per_second": 500.0, "peak_bytes": 20 * 1024 * 1024 },
                "spider": { "pages_per_second": 500.0, "peak_bytes": 40 * 1024 * 1024 },
            }),
        )
    );
}

#[test]
fn a_tool_given_a_different_page_set_fails_the_same_work_check_and_reports_no_figure() {
    let workspace = Workspace::from_fixture("empty");
    let mut trials = trials();
    trials["trials"][5] = trial("spider", 3, 2.0, 1, &pages(999));

    let outcome = report(&workspace, &trials);

    assert_eq!(
        (outcome.findings(), outcome.code),
        (vec![finding("different_pages", "spider trial 3")], 1),
        "{}",
        outcome.stderr
    );
    assert_eq!(workspace.read(REPORT), None);
}

#[test]
fn a_trial_served_other_bytes_for_the_same_pages_fails_the_same_work_check() {
    let workspace = Workspace::from_fixture("empty");
    let mut trials = trials();
    trials["trials"][3]["bytes"] = json!(600_001);

    let outcome = report(&workspace, &trials);

    assert_eq!(
        (outcome.findings(), outcome.code),
        (vec![finding("different_bytes", "foxhound trial 2")], 1),
        "{}",
        outcome.stderr
    );
}

#[test]
fn a_tool_short_of_its_declared_trials_fails() {
    let workspace = Workspace::from_fixture("empty");
    let mut trials = trials();
    trials["trials"].as_array_mut().unwrap().pop();

    let outcome = report(&workspace, &trials);

    assert_eq!(
        (outcome.findings(), outcome.code),
        (vec![finding("trial_count", "spider")], 1),
        "{}",
        outcome.stderr
    );
}

#[test]
fn the_callgrind_cell_reports_foxhounds_instructions_per_page_over_spiders() {
    let workspace = Workspace::from_fixture("empty");

    let outcome = report(&workspace, &trials_with(measured_cell()));

    assert_eq!(
        (outcome.findings(), outcome.code),
        (vec![], 0),
        "{}",
        outcome.stderr
    );
    let written: Value = serde_json::from_slice(&workspace.read(REPORT).unwrap()).unwrap();
    assert_eq!(
        written["instructions"],
        json!({
            "status": "measured",
            "row": "foxhound.spider_instruction_ratio",
            "scenario": "scenario 1 of ADR 0138's matrix",
            "foxhound_per_page": 32_000.0,
            "spider_per_page": 1_000.0,
            "ratio": 32.0,
        })
    );
    let markdown = workspace.read_text("target/compare-spider/report.md");
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
    let written: Value = serde_json::from_slice(&workspace.read(REPORT).unwrap()).unwrap();
    assert_eq!(
        written["instructions"],
        json!({ "status": "not_run", "reason": "Valgrind is not installed on this host" })
    );
    assert!(
        workspace
            .read_text("target/compare-spider/report.md")
            .contains("`foxhound.spider_instruction_ratio`: not run: Valgrind is not installed")
    );
}

#[test]
fn a_callgrind_run_of_other_pages_fails_the_same_work_check_and_reports_no_figure() {
    let workspace = Workspace::from_fixture("empty");
    let mut cell = measured_cell();
    cell["runs"][4] = run("spider", 2, 500, &pages(999));

    let outcome = report(&workspace, &trials_with(cell));

    assert_eq!(
        (outcome.findings(), outcome.code),
        (
            vec![finding("different_pages", "spider callgrind trial 2")],
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
    let mut cell = measured_cell();
    cell["runs"][1]["bytes"] = json!(600_001);

    let outcome = report(&workspace, &trials_with(cell));

    assert_eq!(
        (outcome.findings(), outcome.code),
        (
            vec![finding("different_bytes", "foxhound callgrind trial 2")],
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
    let mut cell = measured_cell();
    cell["runs"].as_array_mut().unwrap().pop();

    let outcome = report(&workspace, &trials_with(cell));

    assert_eq!(
        (outcome.findings(), outcome.code),
        (vec![finding("trial_count", "spider callgrind")], 1),
        "{}",
        outcome.stderr
    );
}

/// A Spider count of 0 instructions is no measurement, and no ratio follows
/// from it.
#[test]
fn a_tool_that_counted_no_instructions_fails() {
    let workspace = Workspace::from_fixture("empty");
    let mut cell = measured_cell();
    cell["runs"][3]["instructions"] = json!(0);

    let outcome = report(&workspace, &trials_with(cell));

    assert_eq!(
        (outcome.findings(), outcome.code),
        (
            vec![finding("no_instructions", "spider callgrind trial 1")],
            1
        ),
        "{}",
        outcome.stderr
    );
}
