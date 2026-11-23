//! The diagnostic run's choice of tests (ADR 0064): after a failed run, each
//! test that failed in its attempt is run again 20 times on the same commit
//! and OS, in a job of its own. GitHub's interface is a local stand-in; the
//! artifacts are a downloaded run's folders.
//! Covers: story:1-113

use rstest::rstest;
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

/// A failed cell is diagnosed on the runner label it ran on, so each
/// runner's jobs take the tests of their own OS and architecture.
#[rstest]
#[case::os(&["--os", "macos"])]
#[case::architecture(&["--os", "linux", "--arch", "arm64"])]
fn failed_tests_of_another_platform_get_no_job_here(#[case] platform: &[&str]) {
    let workspace = Workspace::from_fixture("results-ingest");
    let github = serving();

    let outcome = failed(&workspace, &github, platform);

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
                "commit": "c2",
                "suite": "pre-merge",
                "tier": "test-health",
                "os": "linux",
                "arch": "x64",
                "runtime": null,
            },
            {
                "test": "pytest=tests/test_sample.py::test_flaky",
                "commit": "c2",
                "suite": "pre-merge",
                "tier": "test-health",
                "os": "linux",
                "arch": "x64",
                "runtime": null,
            },
        ] })
    );
}

/// The constrained run after a Release run tests the commit that run
/// tested, which each of its JUnit artifacts names (ADR 0064).
#[test]
fn tested_commit_is_the_one_the_runs_artifacts_name() {
    let workspace = Workspace::from_fixture("results-ingest");

    let outcome = workspace.xtask(&["results", "tested-commit", "--artifacts", "artifacts"]);

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    assert_eq!(outcome.stdout, "commit=c2\n");
}

#[test]
fn artifacts_naming_two_commits_name_no_tested_commit() {
    let workspace = Workspace::from_fixture("results-ingest");
    workspace.write(
        "artifacts/junit-quick-test-health-linux-x64-1/context.json",
        br#"{ "commit": "c3", "tree": "t3", "job": "quick", "order_seed": null, "entropy_seed": null }"#,
    );

    let outcome = workspace.xtask(&["results", "tested-commit", "--artifacts", "artifacts"]);

    assert_eq!(outcome.code, 2);
    assert_eq!(outcome.stdout, "");
}
