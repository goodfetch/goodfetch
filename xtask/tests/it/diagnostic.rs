//! The diagnostic run's choice of tests (ADR 0064): after a failed run, each
//! test that failed in its attempt is run again 20 times on the same commit
//! and OS, in a job of its own. GitHub's interface is a local stand-in; the
//! artifacts are a downloaded run's folders.
//! Covers: story:1-113

use serde_json::{Value, json};

use crate::github::GitHub;
use crate::workspace::Workspace;

const RUN_ATTEMPT: &str = r#"{
  "id": 42, "run_attempt": 2, "path": ".github/workflows/test-health.yml",
  "event": "schedule", "head_branch": "main", "head_sha": "c1",
  "head_commit": { "id": "c1", "tree_id": "t1" },
  "run_started_at": "2026-10-18T03:00:00Z", "conclusion": "failure"
}"#;

/// The quick artifact is the first attempt's, whose failures were diagnosed
/// then.
const RUN_ARTIFACTS: &str = r#"{ "total_count": 2, "artifacts": [
  { "name": "junit-quick-test-health-linux-x64-1", "created_at": "2026-10-17T03:05:00Z", "expired": false },
  { "name": "junit-pre-merge-test-health-linux-x64-1", "created_at": "2026-10-18T03:02:00Z", "expired": false }
] }"#;

fn serving() -> GitHub {
    GitHub::serving(&[
        (
            "/repos/goodfetch/goodfetch/actions/runs/42/attempts/2",
            RUN_ATTEMPT,
        ),
        (
            "/repos/goodfetch/goodfetch/actions/runs/42/attempts/2/jobs?per_page=100&page=1",
            r#"{ "total_count": 0, "jobs": [] }"#,
        ),
        (
            "/repos/goodfetch/goodfetch/actions/runs/42/artifacts?per_page=100&page=1",
            RUN_ARTIFACTS,
        ),
    ])
}

/// The value of `key` in the command's `key=value` lines.
fn value(stdout: &str, key: &str) -> Value {
    let text = stdout
        .lines()
        .find_map(|line| line.strip_prefix(key)?.strip_prefix('='))
        .unwrap();
    serde_json::from_str(text).unwrap()
}

fn failed(workspace: &Workspace, github: &GitHub, extra: &[&str]) -> crate::workspace::Outcome {
    let arguments = [
        &[
            "results",
            "failed",
            "--run-id",
            "42",
            "--attempt",
            "2",
            "--artifacts",
            "artifacts",
            "--api-url",
            github.url(),
        ],
        extra,
    ]
    .concat();
    workspace.xtask(&arguments)
}

/// A failed cell on the owner's machine is diagnosed there (ADR 0108), so
/// each runner's jobs take the tests of their own OS.
#[test]
fn failed_tests_of_another_os_get_no_job_here() {
    let workspace = Workspace::from_fixture("results-ingest");
    let github = serving();

    let outcome = failed(&workspace, &github, &["--os", "macos"]);

    assert_eq!(outcome.code, 0);
    assert_eq!(value(&outcome.stdout, "count"), json!(0));
    assert_eq!(value(&outcome.stdout, "matrix"), json!({ "include": [] }));
}

#[test]
fn each_test_that_failed_in_the_attempt_gets_one_diagnostic_job() {
    let workspace = Workspace::from_fixture("results-ingest");
    let github = serving();

    let outcome = failed(&workspace, &github, &["--os", "linux"]);

    assert_eq!(outcome.code, 0);
    assert_eq!(value(&outcome.stdout, "count"), json!(2));
    assert_eq!(
        value(&outcome.stdout, "matrix"),
        json!({ "include": [
            {
                "test": "nextest=sample tests::flaky",
                "suite": "pre-merge",
                "tier": "test-health",
                "os": "linux",
                "arch": "x64",
                "runtime": null,
            },
            {
                "test": "pytest=tests/test_sample.py::test_flaky",
                "suite": "pre-merge",
                "tier": "test-health",
                "os": "linux",
                "arch": "x64",
                "runtime": null,
            },
        ] })
    );
}
