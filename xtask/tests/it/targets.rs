//! The Performance targets report (ADR 0134): `xtask targets report` reads
//! what `just targets` measured on the owner's machine, each Product's
//! startup to `hello` as hyperfine exports it and each resident-memory
//! scenario's peaks as `xtask rss measure` writes them, and the latest `just
//! compare-spider` report's pages per second, and
//! gives each target its figure and verdict: met at or under its target,
//! missed over it with the gap and the optimization pass's write-up, or not
//! measured. It never fails on a miss. Measurements are fixtures here.

use rstest::rstest;
use serde_json::{Value, json};

use crate::workspace::{Outcome, Workspace};

const MIB: u64 = 1024 * 1024;
const MEASURED: &str = "target/targets";
const SPIDER: &str = "target/compare-spider/report.json";
const REPORT: &str = "target/targets/report.json";

/// A hyperfine export of one command whose median took `seconds`.
fn startup(workspace: &Workspace, product: &str, seconds: f64) {
    let export = json!({
        "results": [{
            "command": format!("{product} attached-engine"),
            "mean": seconds,
            "median": seconds,
            "min": seconds,
            "max": seconds,
            "times": [seconds, seconds, seconds],
            "exit_codes": [0, 0, 0],
        }],
    });
    workspace.write(
        &format!("{MEASURED}/startup-{product}.json"),
        export.to_string().as_bytes(),
    );
}

/// A hyperfine 2 export (its schema 2) of one command whose median took
/// `seconds` over `runs` runs.
fn startup_v2(workspace: &Workspace, product: &str, seconds: f64, runs: u32) {
    let summary = |unit: &str, value: f64| {
        json!({
            "unit": unit, "count": runs, "mean": value, "stddev": 0.0,
            "median": value, "min": value, "max": value,
        })
    };
    let export = json!({
        "schema_version": 2,
        "metadata": { "hyperfine_version": "2.0.0" },
        "primary_metric": "time_wall_clock",
        "results": [{
            "command": format!("{product} attached-engine"),
            "name": format!("{product} attached engine to hello"),
            "measurements": [],
            "summary": {
                "time_wall_clock": summary("second", seconds),
                "time_cpu": summary("second", seconds / 2.0),
                "memory_peak_resident": summary("byte", 30_000_000.0),
            },
        }],
    });
    workspace.write(
        &format!("{MEASURED}/startup-{product}.json"),
        export.to_string().as_bytes(),
    );
}

/// Peaks of the resident-memory check's scenarios, one line per run.
fn peaks(workspace: &Workspace, peaks: &[(&str, &str, u64)]) {
    let lines: Vec<String> = peaks
        .iter()
        .map(|(product, scenario, bytes)| {
            json!({
                "product": product,
                "platform": "macos-arm64",
                "scenario": scenario,
                "bytes": bytes,
            })
            .to_string()
        })
        .collect();
    let lines = lines.join("\n");
    workspace.write(&format!("{MEASURED}/peaks.jsonl"), lines.as_bytes());
}

/// A `just compare-spider` report with each tool's medians.
fn spider_report(workspace: &Workspace, foxhound: (f64, u64), spider: (f64, u64)) {
    let report = json!({
        "label": "local-controlled",
        "published": false,
        "machine": {
            "class": "local-controlled",
            "os": "macos 27.0.1",
            "arch": "aarch64",
            "cpu": "Apple M3 Max",
            "threads": 14,
        },
        "commit": "0123456789abcdef0123456789abcdef01234567",
        "declared": {
            "trials": 10,
            "statistic": "median",
            "concurrency": 16,
            "site": "/sites/1000/1",
            "spider": "2.53.9",
        },
        "order_seed": 7,
        "same_work": { "pages": 1000, "bytes": 462_362, "trials": 20 },
        "tools": {
            "foxhound": { "pages_per_second": foxhound.0, "peak_bytes": foxhound.1 },
            "spider": { "pages_per_second": spider.0, "peak_bytes": spider.1 },
        },
    });
    workspace.write(SPIDER, report.to_string().as_bytes());
}

fn report(workspace: &Workspace) -> (Outcome, Value) {
    let outcome = workspace.xtask(&[
        "targets",
        "report",
        "--measured",
        MEASURED,
        "--spider",
        SPIDER,
    ]);
    let report = workspace
        .read(REPORT)
        .map_or(Value::Null, |bytes| serde_json::from_slice(&bytes).unwrap());
    (outcome, report)
}

/// The row of `target` for `product`: its verdict and gap.
fn row(report: &Value, target: &str, product: &str) -> (String, Value) {
    let row = report["rows"]
        .as_array()
        .unwrap_or_else(|| panic!("no rows in {report}"))
        .iter()
        .find(|row| row["target"] == target && row["product"] == product)
        .unwrap_or_else(|| panic!("no row of {target} for {product} in {report}"));
    (
        row["verdict"].as_str().unwrap().to_owned(),
        row["gap"].clone(),
    )
}

#[test]
fn a_figure_over_its_target_is_missed_with_its_gap_and_one_at_or_under_is_met() {
    let workspace = Workspace::from_fixture("empty");
    startup(&workspace, "collie", 0.010);
    startup(&workspace, "foxhound", 0.0125);
    // Foxhound's median of 7, 9 and 10 MiB is 9 MiB; each scenario's
    // figure is the median of its own peaks only.
    peaks(
        &workspace,
        &[
            ("collie", "idle-attached-engine", 8 * MIB),
            ("foxhound", "idle-attached-engine", 7 * MIB),
            ("foxhound", "idle-attached-engine", 10 * MIB),
            ("foxhound", "idle-attached-engine", 9 * MIB),
            ("collie", "idle-daemon", 16 * MIB),
            ("foxhound", "idle-daemon", 18 * MIB),
            ("foxhound", "crawl", 33 * MIB),
            ("foxhound", "crawl", 34 * MIB),
            ("foxhound", "crawl", 32 * MIB),
        ],
    );
    // The crawl's peak is the resident-memory check's, not compare-spider's.
    spider_report(&workspace, (30.0, 20 * MIB), (40.0, 25 * MIB));

    let (outcome, report) = report(&workspace);

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    assert_eq!(report["label"], "local-controlled");
    assert_eq!(report["published"], false);
    assert_eq!(
        [
            row(&report, "startup-to-hello", "collie"),
            row(&report, "startup-to-hello", "foxhound"),
            row(&report, "idle-attached-engine", "collie"),
            row(&report, "idle-attached-engine", "foxhound"),
            row(&report, "idle-daemon", "collie"),
            row(&report, "idle-daemon", "foxhound"),
            row(&report, "crawl-peak", "foxhound"),
            row(&report, "crawl-pages-per-second", "foxhound"),
        ],
        [
            ("met".to_owned(), Value::Null),
            ("missed".to_owned(), json!("2.5 ms")),
            ("met".to_owned(), Value::Null),
            ("missed".to_owned(), json!("1.0 MiB")),
            ("met".to_owned(), Value::Null),
            ("missed".to_owned(), json!("2.0 MiB")),
            ("missed".to_owned(), json!("1.0 MiB")),
            ("missed".to_owned(), json!("10.0 pages per second")),
        ]
    );
}

#[test]
fn as_many_pages_per_second_as_spider_meets_its_target() {
    let workspace = Workspace::from_fixture("empty");
    peaks(&workspace, &[("foxhound", "crawl", 32 * MIB)]);
    spider_report(&workspace, (40.0, 40 * MIB), (40.0, 25 * MIB));

    let (outcome, report) = report(&workspace);

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    assert_eq!(
        [
            row(&report, "crawl-peak", "foxhound"),
            row(&report, "crawl-pages-per-second", "foxhound"),
        ],
        [
            ("met".to_owned(), Value::Null),
            ("met".to_owned(), Value::Null),
        ]
    );
}

#[test]
fn a_miss_carries_the_optimization_pass_write_up_for_it() {
    let workspace = Workspace::from_fixture("empty");
    startup(&workspace, "collie", 0.012);
    startup(&workspace, "foxhound", 0.012);
    workspace.write(
        "ci/performance-target-gaps.toml",
        b"[startup-to-hello.collie]\nwrite-up = \"The dynamic loader relocates 9,046 relative relocations.\"\n",
    );

    let (outcome, report) = report(&workspace);

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    let write_up = |product: &str| {
        report["rows"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["target"] == "startup-to-hello" && row["product"] == product)
            .unwrap()["write_up"]
            .clone()
    };
    assert_eq!(
        [write_up("collie"), write_up("foxhound")],
        [
            json!("The dynamic loader relocates 9,046 relative relocations."),
            Value::Null
        ]
    );
    let markdown = workspace.read_text("target/targets/report.md");
    assert!(
        markdown.contains("The dynamic loader relocates 9,046 relative relocations."),
        "{markdown}"
    );
}

/// A write-up under a target or Product the file does not take would never
/// reach its miss: the report refuses the file, naming the key.
#[rstest]
#[case::a_mistyped_target("[crawl-peaks.foxhound]", "crawl-peaks")]
#[case::a_mistyped_product("[crawl-peak.foxhoud]", "foxhoud")]
fn a_write_up_under_a_name_the_file_does_not_take_is_refused(
    #[case] table: &str,
    #[case] name: &str,
) {
    let workspace = Workspace::from_fixture("empty");
    startup(&workspace, "collie", 0.012);
    workspace.write(
        "ci/performance-target-gaps.toml",
        format!("{table}\nwrite-up = \"A write-up.\"\n").as_bytes(),
    );

    let (outcome, _) = report(&workspace);

    assert_eq!(outcome.code, 2, "{}", outcome.stderr);
    assert!(
        outcome.stderr.contains(&format!("`{name}` is no name")),
        "{}",
        outcome.stderr
    );
}

#[test]
fn a_target_with_no_measurement_is_reported_as_not_measured_and_nothing_fails() {
    let workspace = Workspace::from_fixture("empty");

    let (outcome, report) = report(&workspace);

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    let verdicts: Vec<(String, String, String)> = report["rows"]
        .as_array()
        .unwrap_or_else(|| panic!("no rows in {report}"))
        .iter()
        .map(|row| {
            (
                row["target"].as_str().unwrap().to_owned(),
                row["product"].as_str().unwrap().to_owned(),
                row["verdict"].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    let not_measured = |target: &str, product: &str| {
        (
            target.to_owned(),
            product.to_owned(),
            "not-measured".to_owned(),
        )
    };
    assert_eq!(
        verdicts,
        vec![
            not_measured("startup-to-hello", "collie"),
            not_measured("startup-to-hello", "foxhound"),
            not_measured("idle-attached-engine", "collie"),
            not_measured("idle-attached-engine", "foxhound"),
            not_measured("idle-daemon", "collie"),
            not_measured("idle-daemon", "foxhound"),
            not_measured("crawl-peak", "foxhound"),
            not_measured("crawl-pages-per-second", "foxhound"),
        ]
    );
}

#[test]
fn startup_is_read_from_hyperfine_two_s_export_too() {
    let workspace = Workspace::from_fixture("empty");
    startup_v2(&workspace, "collie", 0.009, 100);
    startup_v2(&workspace, "foxhound", 0.0112, 100);

    let (outcome, report) = report(&workspace);

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    assert_eq!(
        [
            row(&report, "startup-to-hello", "collie"),
            row(&report, "startup-to-hello", "foxhound"),
        ],
        [
            ("met".to_owned(), Value::Null),
            ("missed".to_owned(), json!("1.2 ms")),
        ]
    );
}
