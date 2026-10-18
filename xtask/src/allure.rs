//! The inputs of the Allure Report 3 build from the results store (ADR 0064):
//! the daily test-health job runs `allure generate` on them. One test is one
//! test ID of one runner in one configuration (OS, architecture, runtime),
//! named by a key that is both its current result's `historyId` and its key
//! in each history point, which Allure matches.
//! - `results/`: each test's latest result, as an Allure result file;
//! - `history.jsonl`: one history point per run attempt, oldest first, with
//!   each test's result in it; a test the attempt ran several times (a
//!   repeated run) shows its first failure, or else its first run. A test's
//!   current result is left out of the history point it came from;
//! - `allurerc.json`: the report's name, `report/` as its output and the
//!   history file, all relative to the folder.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs ban targets async crates (STYLE.md)"
)]

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context as _, Result};
use serde_json::{Value, json};

use crate::results::{self, Outcome, TestRun};

/// Writes the inputs into `out`, which it empties first.
pub(crate) fn write(store: &Path, out: &Path) -> Result<()> {
    let mut attempts = results::read_all(store)?;
    attempts.sort_by_key(|attempt| (attempt.run.started_at, attempt.run.run, attempt.run.attempt));
    let mut current: BTreeMap<String, &TestRun> = BTreeMap::new();
    for test_run in attempts.iter().flat_map(|attempt| &attempt.test_runs) {
        current.insert(key(test_run), test_run);
    }

    if out.exists() {
        std::fs::remove_dir_all(out).with_context(|| format!("emptying {}", out.display()))?;
    }
    let results = out.join("results");
    std::fs::create_dir_all(&results).with_context(|| format!("creating {}", results.display()))?;
    for (index, (key, test_run)) in current.iter().enumerate() {
        let result = json!({
            "uuid": format!("current-{index}"),
            "historyId": key,
            "name": test_run.test,
            "fullName": key,
            "status": status(test_run.outcome),
            "start": start(test_run),
            "stop": start(test_run) + milliseconds(test_run),
            "labels": labels(test_run),
        });
        let path = results.join(format!("current-{index}-result.json"));
        std::fs::write(&path, result.to_string())
            .with_context(|| format!("writing {}", path.display()))?;
    }

    let mut history = String::new();
    for attempt in &attempts {
        let mut points: BTreeMap<String, &TestRun> = BTreeMap::new();
        for test_run in &attempt.test_runs {
            let key = key(test_run);
            let is_current = current.get(&key).is_some_and(|current| {
                (current.run, current.attempt) == (test_run.run, test_run.attempt)
            });
            if is_current {
                continue;
            }
            let shown = points.entry(key).or_insert(test_run);
            if shown.outcome != Outcome::Failed && test_run.outcome == Outcome::Failed {
                *shown = test_run;
            }
        }
        let uuid = format!("{}-{}", attempt.run.run, attempt.run.attempt);
        let test_results: serde_json::Map<String, Value> = points
            .iter()
            .enumerate()
            .map(|(index, (key, test_run))| {
                (
                    key.clone(),
                    json!({
                        "id": format!("{uuid}-{index}"),
                        "name": test_run.test,
                        "fullName": key,
                        "status": status(test_run.outcome),
                        "start": start(test_run),
                        "stop": start(test_run) + milliseconds(test_run),
                        "duration": milliseconds(test_run),
                        "labels": labels(test_run),
                        "url": "",
                    }),
                )
            })
            .collect();
        let point = json!({
            "uuid": uuid,
            "name": format!("{} run {} attempt {}", attempt.run.workflow, attempt.run.run, attempt.run.attempt),
            "timestamp": attempt.run.started_at.as_millisecond(),
            "knownTestCaseIds": [],
            "testResults": test_results,
            "metrics": {},
            "url": "",
        });
        history.push_str(&point.to_string());
        history.push('\n');
    }
    let history_path = out.join("history.jsonl");
    std::fs::write(&history_path, history)
        .with_context(|| format!("writing {}", history_path.display()))?;

    let config = json!({
        "name": "Goodfetch test results",
        "output": "report",
        "historyPath": "history.jsonl",
        "plugins": { "awesome": { "options": { "singleFile": false } } },
    });
    let config_path = out.join("allurerc.json");
    std::fs::write(&config_path, config.to_string())
        .with_context(|| format!("writing {}", config_path.display()))
}

/// A test in one configuration.
fn key(test_run: &TestRun) -> String {
    let runtime = test_run
        .runtime
        .as_deref()
        .map_or(String::new(), |runtime| format!("-{runtime}"));
    format!(
        "{} {} [{}-{}{runtime}]",
        test_run.runner, test_run.test, test_run.os, test_run.arch
    )
}

/// Allure's status; an inconclusive Live check is `unknown`, never a pass.
fn status(outcome: Outcome) -> &'static str {
    match outcome {
        Outcome::Passed => "passed",
        Outcome::Failed => "failed",
        Outcome::Skipped => "skipped",
        Outcome::Inconclusive => "unknown",
    }
}

fn start(test_run: &TestRun) -> i64 {
    test_run.started_at.as_millisecond()
}

fn milliseconds(test_run: &TestRun) -> i64 {
    // A test run's duration is far below what the cast would truncate.
    #[expect(
        clippy::cast_possible_truncation,
        reason = "a test's duration in milliseconds fits in an i64"
    )]
    let milliseconds = (test_run.duration_seconds * 1000.0).round() as i64;
    milliseconds
}

fn labels(test_run: &TestRun) -> Value {
    let mut configuration = format!("{}-{}", test_run.os, test_run.arch);
    if let Some(runtime) = &test_run.runtime {
        configuration.push('-');
        configuration.push_str(runtime);
    }
    json!([
        { "name": "parentSuite", "value": test_run.runner },
        { "name": "suite", "value": test_run.suite },
        { "name": "subSuite", "value": configuration },
        { "name": "tag", "value": test_run.tier },
    ])
}
