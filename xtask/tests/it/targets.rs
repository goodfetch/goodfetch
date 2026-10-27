//! The Performance targets report (ADR 0134): `xtask targets report` reads
//! what `just targets` measured on the owner's machine, each Product's
//! startup to `hello` as hyperfine exports it and each idle peak as `xtask
//! rss measure` writes it, and the latest `just compare-spider` report, and
//! gives each target its figure and verdict: met at or under its target,
//! missed over it with the gap and the optimization pass's write-up, or not
//! measured. It never fails on a miss. Measurements are fixtures here.

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

/// Peaks of the idle Attached engine, one line per run.
fn idle_peaks(workspace: &Workspace, peaks: &[(&str, u64)]) {
    let lines: Vec<String> = peaks
        .iter()
        .map(|(product, bytes)| {
            json!({
                "product": product,
                "platform": "macos-arm64",
                "scenario": "idle-attached-engine",
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
    // Foxhound's median of 7, 9 and 10 MiB is 9 MiB.
    idle_peaks(
        &workspace,
        &[
            ("collie", 8 * MIB),
            ("foxhound", 7 * MIB),
            ("foxhound", 10 * MIB),
            ("foxhound", 9 * MIB),
        ],
    );
    spider_report(&workspace, (30.0, 33 * MIB), (40.0, 25 * MIB));

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
            row(&report, "crawl-peak", "foxhound"),
            row(&report, "crawl-pages-per-second", "foxhound"),
        ],
        [
            ("met".to_owned(), Value::Null),
            ("missed".to_owned(), json!("2.5 ms")),
            ("met".to_owned(), Value::Null),
            ("missed".to_owned(), json!("1.0 MiB")),
            ("missed".to_owned(), json!("1.0 MiB")),
            ("missed".to_owned(), json!("10.0 pages per second")),
        ]
    );
}

#[test]
fn as_many_pages_per_second_as_spider_meets_its_target() {
    let workspace = Workspace::from_fixture("empty");
    spider_report(&workspace, (40.0, 32 * MIB), (40.0, 25 * MIB));

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
