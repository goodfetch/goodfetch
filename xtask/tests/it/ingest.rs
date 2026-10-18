//! The ingest into the results store (ADR 0064): every attempt of every run,
//! pass or fail, becomes one compressed file holding its JUnit files as
//! written, one record per test run and one per job. GitHub's interface is a
//! local stand-in; the artifacts are a downloaded run's folders.
//! Covers: story:1-65

use serde_json::{Value, json};

use crate::github::GitHub;
use crate::store::{STORE, read_store_file};
use crate::workspace::{Outcome, Workspace};

const ATTEMPT: &str = "/repos/goodfetch/goodfetch/actions/runs/42/attempts/2";
const JOBS: &str = "/repos/goodfetch/goodfetch/actions/runs/42/attempts/2/jobs?per_page=100&page=1";
const ARTIFACTS: &str = "/repos/goodfetch/goodfetch/actions/runs/42/artifacts?per_page=100&page=1";

const RUN_ATTEMPT: &str = r#"{
  "id": 42, "run_attempt": 2, "path": ".github/workflows/test-health.yml",
  "event": "schedule", "head_branch": "main", "head_sha": "c1",
  "head_commit": { "id": "c1", "tree_id": "t1" },
  "run_started_at": "2026-10-18T03:00:00Z", "conclusion": "failure"
}"#;

const ATTEMPT_JOBS: &str = r#"{ "total_count": 2, "jobs": [
  { "name": "quarantined", "labels": ["ubuntu-24.04"], "conclusion": "failure",
    "started_at": "2026-10-18T03:00:05Z", "completed_at": "2026-10-18T03:02:10Z" },
  { "name": "rerun (macOS)", "labels": ["self-hosted", "owner-machine"], "conclusion": "success",
    "started_at": "2026-10-18T03:00:05Z", "completed_at": "2026-10-18T03:00:35Z" }
] }"#;

/// The quick artifact is the first attempt's: it was uploaded before this
/// attempt started.
const RUN_ARTIFACTS: &str = r#"{ "total_count": 3, "artifacts": [
  { "name": "junit-quick-test-health-linux-x64-1", "created_at": "2026-10-17T03:05:00Z", "expired": false },
  { "name": "junit-pre-merge-test-health-linux-x64-1", "created_at": "2026-10-18T03:02:00Z", "expired": false },
  { "name": "visual-differences-test-health", "created_at": "2026-10-18T03:02:00Z", "expired": false }
] }"#;

fn ingest(workspace: &Workspace, github: &GitHub) -> Outcome {
    workspace.xtask(&[
        "results",
        "ingest",
        "--repository",
        "goodfetch/goodfetch",
        "--run-id",
        "42",
        "--attempt",
        "2",
        "--artifacts",
        "artifacts",
        "--store",
        STORE,
        "--api-url",
        github.url(),
    ])
}

fn test_run(test: &str, runner: &str, outcome: &str, duration: f64) -> Value {
    json!({
        "test": test,
        "runner": runner,
        "suite": "pre-merge",
        "tier": "test-health",
        "job": "quarantined",
        "os": "linux",
        "arch": "x64",
        "runtime": null,
        "shard": "1",
        "commit": "c1",
        "tree": "t1",
        "workflow": ".github/workflows/test-health.yml",
        "run": 42,
        "attempt": 2,
        "started_at": "2026-10-18T03:00:00Z",
        "outcome": outcome,
        "duration_seconds": duration,
        "order_seed": "1234",
        "entropy_seed": null,
    })
}

/// A later attempt, 3, of the same run on the same tree, in which
/// `tests::flaky` passed.
fn ingest_a_passing_third_attempt(workspace: &Workspace) {
    let passing = workspace
        .read("artifacts/junit-pre-merge-test-health-linux-x64-1/nextest-ci.xml")
        .unwrap();
    let passing = String::from_utf8(passing)
        .unwrap()
        .replace(r#"failures="1""#, r#"failures="0""#)
        .replace(
            r#"<failure type="test failure">thread 'tests::flaky' panicked</failure>"#,
            "",
        );
    let folder = "third/junit-pre-merge-test-health-linux-x64-1";
    workspace.write(&format!("{folder}/nextest-ci.xml"), passing.as_bytes());
    workspace.write(
        &format!("{folder}/context.json"),
        br#"{ "job": "quarantined", "order_seed": "99", "entropy_seed": null }"#,
    );
    let github = GitHub::serving(&[
        (
            "/repos/goodfetch/goodfetch/actions/runs/42/attempts/3",
            &RUN_ATTEMPT
                .replace(r#""run_attempt": 2"#, r#""run_attempt": 3"#)
                .replace("2026-10-18T03:00:00Z", "2026-10-18T05:00:00Z"),
        ),
        (
            "/repos/goodfetch/goodfetch/actions/runs/42/attempts/3/jobs?per_page=100&page=1",
            ATTEMPT_JOBS,
        ),
        (
            ARTIFACTS,
            r#"{ "total_count": 1, "artifacts": [
              { "name": "junit-pre-merge-test-health-linux-x64-1", "created_at": "2026-10-18T05:02:00Z", "expired": false }
            ] }"#,
        ),
    ]);
    let outcome = workspace.xtask(&[
        "results",
        "ingest",
        "--run-id",
        "42",
        "--attempt",
        "3",
        "--artifacts",
        "third",
        "--store",
        STORE,
        "--api-url",
        github.url(),
    ]);
    assert_eq!(outcome.code, 0);
}

#[test]
fn test_that_failed_in_one_attempt_and_passed_in_another_is_found_flaky_after_ingest() {
    let workspace = Workspace::from_fixture("results-ingest");
    let github = GitHub::serving(&[
        (ATTEMPT, RUN_ATTEMPT),
        (JOBS, ATTEMPT_JOBS),
        (ARTIFACTS, RUN_ARTIFACTS),
    ]);
    assert_eq!(ingest(&workspace, &github).code, 0);
    ingest_a_passing_third_attempt(&workspace);
    let issues = GitHub::serving(&[
        (
            "/repos/goodfetch/goodfetch/issues?labels=flaky-test&state=all&per_page=100&page=1",
            "[]",
        ),
        (
            "/repos/goodfetch/goodfetch/issues?labels=live-check&state=all&per_page=100&page=1",
            "[]",
        ),
        (
            "/repos/goodfetch/goodfetch/milestones?state=open&per_page=100&page=1",
            r#"[{ "number": 1, "title": "M1 Core", "open_issues": 5, "due_on": null }]"#,
        ),
        (
            "POST /repos/goodfetch/goodfetch/issues",
            r#"{ "number": 1700 }"#,
        ),
    ]);

    let outcome = workspace.xtask(&[
        "results",
        "scan",
        "--store",
        STORE,
        "--api-url",
        issues.url(),
    ]);

    assert_eq!(outcome.code, 0);
    let opened: Vec<Value> = issues
        .writes()
        .into_iter()
        .map(|write| write.body["title"].clone())
        .collect();
    assert_eq!(
        opened,
        vec![json!("Flaky test: sample tests::flaky (nextest)")]
    );
}

#[test]
fn attempt_becomes_one_store_file_with_a_record_per_test_run_and_job() {
    let workspace = Workspace::from_fixture("results-ingest");
    let github = GitHub::serving(&[
        (ATTEMPT, RUN_ATTEMPT),
        (JOBS, ATTEMPT_JOBS),
        (ARTIFACTS, RUN_ARTIFACTS),
    ]);

    let outcome = ingest(&workspace, &github);

    assert_eq!(outcome.code, 0);
    let file = read_store_file(&workspace, &format!("{STORE}/runs/2026/10/42-2.tar.gz"));
    assert_eq!(
        file.paths,
        vec![
            "jobs.jsonl",
            "junit/junit-pre-merge-test-health-linux-x64-1/context.json",
            "junit/junit-pre-merge-test-health-linux-x64-1/nextest-ci.xml",
            "junit/junit-pre-merge-test-health-linux-x64-1/playwright.xml",
            "junit/junit-pre-merge-test-health-linux-x64-1/pytest.xml",
            "junit/junit-pre-merge-test-health-linux-x64-1/vitest.xml",
            "run.json",
            "test-runs.jsonl",
        ]
    );
    assert_eq!(
        file.run,
        json!({
            "repository": "goodfetch/goodfetch",
            "workflow": ".github/workflows/test-health.yml",
            "run": 42,
            "attempt": 2,
            "event": "schedule",
            "branch": "main",
            "commit": "c1",
            "tree": "t1",
            "started_at": "2026-10-18T03:00:00Z",
        })
    );
    assert_eq!(
        file.test_runs,
        vec![
            test_run("sample tests::steady", "nextest", "passed", 0.008),
            test_run("sample tests::flaky", "nextest", "failed", 0.25),
            test_run(
                "e2e/sample.spec.ts › sample › flaky",
                "playwright",
                "skipped",
                0.0
            ),
            test_run("tests/test_sample.py::test_flaky", "pytest", "failed", 1.25),
            test_run("sample > steady (a+b)", "vitest", "passed", 0.5),
        ]
    );
    assert_eq!(
        file.jobs,
        vec![
            json!({
                "job": "quarantined",
                "labels": ["ubuntu-24.04"],
                "runner_os": "linux",
                "self_hosted": false,
                "started_at": "2026-10-18T03:00:05Z",
                "completed_at": "2026-10-18T03:02:10Z",
                "duration_seconds": 125,
                "conclusion": "failure",
            }),
            json!({
                "job": "rerun (macOS)",
                "labels": ["self-hosted", "owner-machine"],
                "runner_os": "macos",
                "self_hosted": true,
                "started_at": "2026-10-18T03:00:05Z",
                "completed_at": "2026-10-18T03:00:35Z",
                "duration_seconds": 30,
                "conclusion": "success",
            }),
        ]
    );
}
