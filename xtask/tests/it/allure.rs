//! The Allure Report 3 build from the results store (ADR 0064): `xtask
//! results allure` writes each test's latest result and one history point
//! per run attempt, and the pinned Allure, run for real, shows per-test
//! history and durations from them.
//! Covers: story:1-65

use serde_json::{Value, json};

use crate::store::{Attempt, STORE, test_run, with, write_store};
use crate::workspace::Workspace;

fn attempt(run: u64, started_at: &'static str, test_runs: Vec<Value>) -> Attempt {
    let mut attempt = Attempt::new(run, started_at);
    attempt.test_runs = test_runs;
    attempt
}

/// A status and a duration in milliseconds.
type Run = (String, u64);

/// One of the report's test results: its name, status, duration and
/// history (newest first).
type Shown = (String, String, u64, Vec<Run>);

/// Each of the report's test results, by name.
fn report(workspace: &Workspace) -> Vec<Shown> {
    let folder = workspace.path().join("allure/report/data/test-results");
    let mut results: Vec<_> = std::fs::read_dir(folder)
        .unwrap()
        .map(|entry| {
            let result: Value =
                serde_json::from_slice(&std::fs::read(entry.unwrap().path()).unwrap()).unwrap();
            let history = result["history"]
                .as_array()
                .unwrap()
                .iter()
                .map(|point| {
                    (
                        point["status"].as_str().unwrap().to_owned(),
                        point["duration"].as_u64().unwrap(),
                    )
                })
                .collect();
            (
                result["name"].as_str().unwrap().to_owned(),
                result["status"].as_str().unwrap().to_owned(),
                result["duration"].as_u64().unwrap(),
                history,
            )
        })
        .collect();
    results.sort();
    results
}

#[test]
fn report_shows_each_tests_history_and_durations_from_the_store() {
    let workspace = Workspace::from_fixture("empty");
    workspace.link_installed_runners();
    let timed = |test: &str, outcome: &str, seconds: f64| {
        with(
            test_run(test, outcome, "aaa"),
            "duration_seconds",
            json!(seconds),
        )
    };
    write_store(
        &workspace,
        &[
            attempt(
                10,
                "2026-10-01T08:00:00Z",
                vec![
                    timed("sample tests::flaky", "failed", 0.5),
                    timed("sample tests::steady", "passed", 0.1),
                ],
            ),
            attempt(
                11,
                "2026-10-02T08:00:00Z",
                vec![timed("sample tests::flaky", "passed", 0.3)],
            ),
            attempt(
                12,
                "2026-10-03T08:00:00Z",
                vec![timed("sample tests::flaky", "passed", 0.25)],
            ),
        ],
    );

    let outcome = workspace.xtask(&["results", "allure", "--store", STORE, "--out", "allure"]);
    assert_eq!(outcome.code, 0);
    let generated = workspace
        .command(workspace.path().join("node_modules/.bin/allure"))
        .current_dir(workspace.path().join("allure"))
        .args(["generate", "results", "--config", "allurerc.json"])
        .output()
        .unwrap();
    assert!(generated.status.success());

    assert_eq!(
        report(&workspace),
        vec![
            (
                "sample tests::flaky".to_owned(),
                "passed".to_owned(),
                250,
                vec![("passed".to_owned(), 300), ("failed".to_owned(), 500)],
            ),
            (
                "sample tests::steady".to_owned(),
                "passed".to_owned(),
                100,
                vec![],
            ),
        ]
    );
}
