//! The ingest into the results store (ADR 0064): every attempt of every run,
//! pass or fail, becomes one compressed file holding its JUnit files as
//! written, one record per test run and one per job. GitHub's interface is a
//! local stand-in; the artifacts are a downloaded run's folders.
//! Covers: story:1-113

use serde_json::{Value, json};

use crate::github::GitHub;
use crate::store::{Attempt, STORE, read_store_file, write_store};
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
        "commit": "c2",
        "tree": "t2",
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
    let outcome = ingest_third_attempt(
        workspace,
        "junit-pre-merge-test-health-linux-x64-1",
        &[
            ("nextest-ci.xml", passing.as_bytes()),
            (
                "context.json",
                br#"{ "commit": "c2", "tree": "t2", "job": "quarantined", "order_seed": "99", "entropy_seed": null }"#,
            ),
        ],
    );
    assert_eq!(outcome.code, 0);
}

/// Ingests attempt 3 of the run, started two hours after attempt 2, whose
/// one artifact `name` holds `files`.
fn ingest_third_attempt(workspace: &Workspace, name: &str, files: &[(&str, &[u8])]) -> Outcome {
    for (file, contents) in files {
        workspace.write(&format!("third/{name}/{file}"), contents);
    }
    let artifacts = format!(
        r#"{{ "total_count": 1, "artifacts": [
          {{ "name": "{name}", "created_at": "2026-10-18T05:02:00Z", "expired": false }}
        ] }}"#
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
        (ARTIFACTS, &artifacts),
    ]);
    workspace.xtask(&[
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
    ])
}

/// The daily ingest (ADR 0135) takes every finished run attempt of the past
/// day that the store lacks, such as the pull-request tier's, which no
/// per-run ingest follows.
#[test]
fn daily_ingest_lists_the_finished_attempts_the_store_lacks() {
    let workspace = Workspace::from_fixture("empty");
    write_store(
        &workspace,
        &[
            Attempt::new(50, "2026-10-17T09:00:00Z"),
            Attempt::new(51, "2026-10-17T10:00:00Z"),
        ],
    );
    let github = GitHub::serving(&[(
        "/repos/goodfetch/goodfetch/actions/runs?status=completed&created=%3E%3D2026-10-17&per_page=100&page=1",
        r#"{ "total_count": 3, "workflow_runs": [
          { "id": 50, "run_attempt": 2 },
          { "id": 51, "run_attempt": 1 },
          { "id": 52, "run_attempt": 1 }
        ] }"#,
    )]);

    let outcome = workspace.xtask(&[
        "results",
        "pending",
        "--store",
        STORE,
        "--since",
        "2026-10-17",
        "--api-url",
        github.url(),
    ]);

    assert_eq!(outcome.code, 0);
    assert_eq!(outcome.stdout, "50 2\n52 1\n");
}

/// A missed daily ingest, or a run queued for a day while the runner was
/// offline, loses nothing: with no `--since`, the runs are listed from the
/// day before the first of the store's newest month.
#[test]
fn daily_ingest_lists_from_the_stores_newest_month() {
    let workspace = Workspace::from_fixture("empty");
    write_store(
        &workspace,
        &[
            Attempt::new(50, "2026-09-20T09:00:00Z"),
            Attempt::new(51, "2026-10-17T10:00:00Z"),
        ],
    );
    let github = GitHub::serving(&[(
        "/repos/goodfetch/goodfetch/actions/runs?status=completed&created=%3E%3D2026-09-30&per_page=100&page=1",
        r#"{ "total_count": 2, "workflow_runs": [
          { "id": 51, "run_attempt": 1 },
          { "id": 53, "run_attempt": 1 }
        ] }"#,
    )]);

    let outcome = workspace.xtask(&[
        "results",
        "pending",
        "--store",
        STORE,
        "--api-url",
        github.url(),
    ]);

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    assert_eq!(outcome.stdout, "53 1\n");
}

/// A Release cell runs Vitest on each Node.js line and pytest on each
/// Python version, one JUnit file each (`vitest-node24.xml`): the file's runtime
/// joins the artifact's, so each is a configuration of its own; nextest's
/// file names its cell or profile, which adds nothing.
#[test]
fn runtime_of_a_runners_file_joins_its_artifacts() {
    let workspace = Workspace::from_fixture("results-ingest");
    let vitest = r#"<testsuites name="vitest tests" tests="1" failures="0" errors="0" time="0.1">
  <testsuite name="web/sample.test.ts" tests="1" failures="0" errors="0" skipped="0" time="0.1">
    <testcase classname="web/sample.test.ts" name="sample &gt; flaky" time="0.1"/>
  </testsuite>
</testsuites>"#;
    let nextest = r#"<testsuites name="nextest-run" tests="1" failures="0" errors="0" time="0.1">
  <testsuite name="sample" tests="1" failures="0" errors="0">
    <testcase name="tests::flaky" classname="sample" time="0.1"/>
  </testsuite>
</testsuites>"#;

    let outcome = ingest_third_attempt(
        &workspace,
        "junit-release-dispatch-linux-x64-gnu-1",
        &[
            (
                "context.json",
                br#"{ "commit": "c2", "tree": "t2", "job": "linux-x64-gnu", "order_seed": null, "entropy_seed": null }"#,
            ),
            ("nextest-linux-x64-gnu.xml", nextest.as_bytes()),
            ("vitest-node24.xml", vitest.as_bytes()),
        ],
    );

    assert_eq!(outcome.code, 0);
    let file = read_store_file(&workspace, &format!("{STORE}/runs/2026/10/42-3.tar.gz"));
    let runtimes: Vec<(&str, &str)> = file
        .test_runs
        .iter()
        .map(|run| {
            (
                run["runner"].as_str().unwrap(),
                run["runtime"].as_str().unwrap(),
            )
        })
        .collect();
    assert_eq!(runtimes, vec![("nextest", "gnu"), ("vitest", "gnu-node24")]);
}

#[test]
fn repeated_runs_become_one_record_each() {
    let workspace = Workspace::from_fixture("results-ingest");
    let vitest = r#"<testsuites name="vitest tests" tests="1" failures="1" errors="0" time="0.3">
  <testsuite name="web/sample.test.ts" tests="1" failures="1" errors="0" skipped="0" time="0.3">
    <testcase classname="web/sample.test.ts" name="sample &gt; flaky" time="0.3">
      <failure message="expected 1 to be 2">AssertionError</failure>
    </testcase>
  </testsuite>
</testsuites>"#;
    let pytest = r#"<testsuites name="pytest tests"><testsuite name="pytest" errors="0" failures="1" skipped="0" tests="3" time="0.3">
<testcase classname="tests.test_sample" name="test_flaky[1-3]" time="0.1" />
<testcase classname="tests.test_sample" name="test_flaky[2-3]" time="0.1"><failure message="boom">boom</failure></testcase>
<testcase classname="tests.test_sample" name="test_flaky[3-3]" time="0.1" />
</testsuite></testsuites>"#;

    let outcome = ingest_third_attempt(
        &workspace,
        "junit-pre-merge-diagnostic-linux-x64-1",
        &[
            (
                "context.json",
                br#"{ "commit": "c2", "tree": "t2", "job": "rerun", "order_seed": null, "entropy_seed": null, "repeat": 3 }"#,
            ),
            ("pytest.xml", pytest.as_bytes()),
            ("vitest.xml", vitest.as_bytes()),
        ],
    );

    assert_eq!(outcome.code, 0);
    let file = read_store_file(&workspace, &format!("{STORE}/runs/2026/10/42-3.tar.gz"));
    let runs: Vec<(String, String)> = file
        .test_runs
        .iter()
        .map(|run| {
            (
                run["test"].as_str().unwrap().to_owned(),
                run["outcome"].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    let run = |test: &str, outcome: &str| (test.to_owned(), outcome.to_owned());
    assert_eq!(
        runs,
        vec![
            run("tests/test_sample.py::test_flaky", "passed"),
            run("tests/test_sample.py::test_flaky", "failed"),
            run("tests/test_sample.py::test_flaky", "passed"),
            run("sample > flaky", "failed"),
            run("sample > flaky", "passed"),
            run("sample > flaky", "passed"),
        ]
    );
}

/// pytest-repeat adds each run's `<run>-<count>` to a parameterized case's
/// parameter ID: the runs are the case's, under the ID it is listed with.
#[test]
fn repeated_runs_of_a_parameterized_pytest_case_are_the_cases() {
    let workspace = Workspace::from_fixture("results-ingest");
    let pytest = r#"<testsuites name="pytest tests"><testsuite name="pytest" errors="0" failures="1" skipped="0" tests="3" time="0.3">
<testcase classname="tests.test_sample" name="test_reads[a.log-1-3]" time="0.1" />
<testcase classname="tests.test_sample" name="test_reads[a.log-2-3]" time="0.1"><failure message="boom">boom</failure></testcase>
<testcase classname="tests.test_sample" name="test_reads[a.log-3-3]" time="0.1" />
</testsuite></testsuites>"#;

    let outcome = ingest_third_attempt(
        &workspace,
        "junit-pre-merge-diagnostic-linux-x64-1",
        &[
            (
                "context.json",
                br#"{ "commit": "c2", "tree": "t2", "job": "rerun", "order_seed": null, "entropy_seed": null, "repeat": 3 }"#,
            ),
            ("pytest.xml", pytest.as_bytes()),
        ],
    );

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    let file = read_store_file(&workspace, &format!("{STORE}/runs/2026/10/42-3.tar.gz"));
    let runs: Vec<(String, String)> = file
        .test_runs
        .iter()
        .map(|run| {
            (
                run["test"].as_str().unwrap().to_owned(),
                run["outcome"].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    let run = |outcome: &str| {
        (
            "tests/test_sample.py::test_reads[a.log]".to_owned(),
            outcome.to_owned(),
        )
    };
    assert_eq!(runs, vec![run("passed"), run("failed"), run("passed")]);
}

/// The stress run (`just stress`) repeats its tests 20 times, and those taken
/// off the quarantine list 200 times, in one job: each count's files sit in
/// a `repeat-<count>/` folder beside the job's other JUnit files.
#[test]
fn runs_in_a_repeat_folder_are_repeated_its_count_of_times() {
    let workspace = Workspace::from_fixture("results-ingest");
    let vitest = r#"<testsuites name="vitest tests" tests="1" failures="1" errors="0" time="0.3">
  <testsuite name="web/sample.test.ts" tests="1" failures="1" errors="0" skipped="0" time="0.3">
    <testcase classname="web/sample.test.ts" name="sample &gt; flaky" time="0.3">
      <failure message="expected 1 to be 2">AssertionError</failure>
    </testcase>
  </testsuite>
</testsuites>"#;

    let outcome = ingest_third_attempt(
        &workspace,
        "junit-pre-merge-pull-request-linux-x64-1",
        &[
            (
                "context.json",
                br#"{ "commit": "c2", "tree": "t2", "job": "pre-merge", "order_seed": null, "entropy_seed": null }"#,
            ),
            ("repeat-3/vitest.xml", vitest.as_bytes()),
        ],
    );

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    let file = read_store_file(&workspace, &format!("{STORE}/runs/2026/10/42-3.tar.gz"));
    let outcomes: Vec<&str> = file
        .test_runs
        .iter()
        .map(|run| run["outcome"].as_str().unwrap())
        .collect();
    assert_eq!(outcomes, vec!["failed", "passed", "passed"]);
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
        .map(|write| serde_json::from_str::<Value>(&write.body).unwrap()["title"].clone())
        .collect();
    assert_eq!(
        opened,
        vec![json!("Flaky test: sample tests::flaky (nextest)")]
    );
}

/// A run's head is not always what its jobs tested: a Release run tests the
/// commit its `commit` input names, a pull-request run its merge commit. Each
/// test run records the commit and tree its job checked out (`context.json`):
/// here `c2` and `t2`, beside the run's head `c1` and `t1`.
#[test]
fn attempt_becomes_one_store_file_with_a_record_per_test_run_and_job() {
    let workspace = Workspace::from_fixture("results-ingest");
    let github = GitHub::serving(&[
        (ATTEMPT, RUN_ATTEMPT),
        (JOBS, ATTEMPT_JOBS),
        (ARTIFACTS, RUN_ARTIFACTS),
    ]);

    let outcome = ingest(&workspace, &github);

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
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
