//! The test quarantine list (ADR 0064): every entry names an existing test
//! and its open `flaky-test` issue, and every runner skips the listed tests
//! through its own filter. Issues are read from a local stand-in for GitHub,
//! and each runner lists the fixture workspace's tests for real.
//! Covers: story:1-65

use std::collections::BTreeMap;

use rstest::rstest;
use serde_json::Value;

use crate::github::GitHub;
use crate::workspace::{Outcome, Workspace, finding};

const ISSUE: &str = "/repos/goodfetch/goodfetch/issues/1601";

const OPEN_FLAKY_TEST_ISSUE: &str =
    r#"{ "number": 1601, "state": "open", "labels": [ { "name": "flaky-test" } ] }"#;

fn quarantine(workspace: &Workspace, runner: &str, id: &str) {
    let list = format!(
        "[[test]]\nrunner = \"{runner}\"\nid = \"{id}\"\nissue = 1601\ndate = 2026-10-18\n"
    );
    workspace.write("quarantine.toml", list.as_bytes());
}

fn check(workspace: &Workspace, github: &GitHub) -> Outcome {
    workspace.xtask_with_env(
        &["quarantine", "check", "--api-url", github.url()],
        &[("GITHUB_TOKEN", "a-read-token".as_ref())],
    )
}

/// Without a token, a private repository answers 404 for every issue: the
/// check names the missing token instead of each entry's issue.
#[test]
fn an_entry_read_without_a_github_token_fails_naming_the_token() {
    let workspace = Workspace::from_fixture("quarantine");
    quarantine(&workspace, "nextest", "sample tests::flaky");
    let github = GitHub::serving(&[]);

    let outcome = workspace.xtask(&["quarantine", "check", "--api-url", github.url()]);

    assert_ne!(outcome.code, 0);
    assert_eq!(outcome.findings(), vec![]);
    assert!(
        outcome.stderr.contains("no GitHub token"),
        "{}",
        outcome.stderr
    );
}

/// An empty list runs no runner, so `just lint` builds no test binary for
/// it; a listed nextest test needs Cargo for its listing (a Cargo that
/// cannot start fails the check), which shows the empty list's pass ran none.
#[test]
fn an_empty_list_runs_no_runner_and_builds_nothing() {
    let workspace = Workspace::from_fixture("quarantine");
    let github = GitHub::serving(&[(ISSUE, OPEN_FLAKY_TEST_ISSUE)]);
    let no_cargo = workspace.path().join("no-cargo-here");
    let with_no_cargo = |workspace: &Workspace| {
        workspace
            .xtask_with_env(
                &["quarantine", "check", "--api-url", github.url()],
                &[
                    ("CARGO", no_cargo.as_os_str()),
                    ("GITHUB_TOKEN", "a-read-token".as_ref()),
                ],
            )
            .code
    };

    let empty = with_no_cargo(&workspace);
    quarantine(&workspace, "nextest", "sample tests::flaky");
    let listed = with_no_cargo(&workspace);

    assert_eq!((empty, listed == 0), (0, false));
}

#[rstest]
#[case::nextest("nextest", "sample tests::flaky")]
#[case::vitest("vitest", "sample > flaky")]
#[case::playwright("playwright", "e2e/sample.spec.ts › sample › flaky")]
#[case::pytest("pytest", "tests/test_sample.py::test_flaky")]
fn entry_with_an_open_flaky_test_issue_and_an_existing_test_passes(
    #[case] runner: &str,
    #[case] id: &str,
) {
    let workspace = Workspace::from_fixture("quarantine");
    workspace.link_installed_runners();
    quarantine(&workspace, runner, id);
    let github = GitHub::serving(&[(ISSUE, OPEN_FLAKY_TEST_ISSUE)]);

    let outcome = check(&workspace, &github);

    assert_eq!(outcome.findings(), vec![]);
    assert_eq!(outcome.code, 0);
}

#[rstest]
#[case::closed(
    r#"{ "number": 1601, "state": "closed", "labels": [ { "name": "flaky-test" } ] }"#,
    "quarantine-issue-closed"
)]
#[case::without_the_flaky_test_label(
    r#"{ "number": 1601, "state": "open", "labels": [ { "name": "bug" } ] }"#,
    "quarantine-issue-not-flaky-test"
)]
#[case::a_pull_request(
    r#"{ "number": 1601, "state": "open", "labels": [ { "name": "flaky-test" } ],
         "pull_request": { "url": "https://api.github.com/repos/goodfetch/goodfetch/pulls/1601" } }"#,
    "quarantine-issue-missing"
)]
fn entry_without_an_open_flaky_test_issue_fails(#[case] issue: &str, #[case] kind: &str) {
    let workspace = Workspace::from_fixture("quarantine");
    workspace.link_installed_runners();
    quarantine(&workspace, "pytest", "tests/test_sample.py::test_flaky");
    let github = GitHub::serving(&[(ISSUE, issue)]);

    let outcome = check(&workspace, &github);

    assert_eq!(
        outcome.findings(),
        vec![finding(kind, "tests/test_sample.py::test_flaky")]
    );
    assert_eq!(outcome.code, 1);
}

#[test]
fn entry_whose_issue_does_not_exist_fails() {
    let workspace = Workspace::from_fixture("quarantine");
    workspace.link_installed_runners();
    quarantine(&workspace, "pytest", "tests/test_sample.py::test_flaky");
    let github = GitHub::serving(&[]);

    let outcome = check(&workspace, &github);

    assert_eq!(
        outcome.findings(),
        vec![finding(
            "quarantine-issue-missing",
            "tests/test_sample.py::test_flaky"
        )]
    );
    assert_eq!(outcome.code, 1);
}

#[rstest]
#[case::nextest("nextest", "sample tests::removed")]
#[case::vitest("vitest", "sample > removed")]
#[case::playwright("playwright", "e2e/sample.spec.ts › sample › removed")]
#[case::pytest("pytest", "tests/test_sample.py::test_removed")]
fn entry_naming_a_removed_test_fails(#[case] runner: &str, #[case] id: &str) {
    let workspace = Workspace::from_fixture("quarantine");
    workspace.link_installed_runners();
    quarantine(&workspace, runner, id);
    let github = GitHub::serving(&[(ISSUE, OPEN_FLAKY_TEST_ISSUE)]);

    let outcome = check(&workspace, &github);

    assert_eq!(
        outcome.findings(),
        vec![finding("quarantined-test-missing", id)]
    );
    assert_eq!(outcome.code, 1);
}

/// nextest skips by substring, so an entry would also skip `tests::steady_twin`.
#[test]
fn nextest_entry_whose_name_is_part_of_another_tests_name_fails() {
    let workspace = Workspace::from_fixture("quarantine");
    quarantine(&workspace, "nextest", "sample tests::steady");
    let github = GitHub::serving(&[(ISSUE, OPEN_FLAKY_TEST_ISSUE)]);

    let outcome = check(&workspace, &github);

    assert_eq!(
        outcome.findings(),
        vec![finding(
            "quarantine-skips-other-tests",
            "sample tests::steady"
        )]
    );
    assert_eq!(outcome.code, 1);
}

/// The arguments `xtask quarantine exclude` gives one runner, which the
/// `justfile`'s suite commands append to that runner's command line.
fn exclusions(workspace: &Workspace, runner: &str) -> Vec<String> {
    let outcome = workspace.xtask(&["quarantine", "exclude"]);
    assert_eq!(outcome.code, 0);
    let mut exclusions: BTreeMap<String, Vec<String>> =
        serde_json::from_str(&outcome.stdout).expect("`xtask quarantine exclude` prints JSON");
    exclusions.remove(runner).expect("every runner is listed")
}

/// Runs a command in the workspace and returns its stdout.
fn run(workspace: &Workspace, program: &str, args: &[&str], exclusions: &[String]) -> String {
    let output = workspace
        .command(program)
        .args(args)
        .args(exclusions)
        .output()
        .unwrap();
    String::from_utf8(output.stdout).unwrap()
}

/// The tests the runner would run, as their quarantine IDs.
fn selected(workspace: &Workspace, runner: &str, exclusions: &[String]) -> Vec<String> {
    let root = workspace.path();
    let mut tests: Vec<String> = match runner {
        "nextest" => {
            let listing = run(
                workspace,
                "cargo",
                &["nextest", "list", "--message-format", "json"],
                exclusions,
            );
            let listing: Value = serde_json::from_str(&listing).unwrap();
            let suites = listing["rust-suites"].as_object().unwrap();
            suites
                .iter()
                .flat_map(|(binary, suite)| {
                    suite["testcases"]
                        .as_object()
                        .unwrap()
                        .iter()
                        .filter(|(_, case)| case["filter-match"]["status"] == "matches")
                        .map(move |(name, _)| format!("{binary} {name}"))
                })
                .collect()
        }
        "vitest" => {
            let program = root.join("node_modules/.bin/vitest");
            let listing = run(
                workspace,
                program.to_str().unwrap(),
                &["list", "--json"],
                exclusions,
            );
            let listing: Value = serde_json::from_str(&listing).unwrap();
            listing
                .as_array()
                .unwrap()
                .iter()
                .map(|test| test["name"].as_str().unwrap().to_owned())
                .collect()
        }
        "playwright" => {
            let program = root.join("node_modules/.bin/playwright");
            let listing = run(
                workspace,
                program.to_str().unwrap(),
                &["test", "--list"],
                exclusions,
            );
            // Lines read `  e2e/sample.spec.ts:4:7 › sample › flaky`.
            listing
                .lines()
                .filter_map(|line| line.trim().split_once(" › "))
                .map(|(location, titles)| {
                    let file = location.split(':').next().unwrap();
                    format!("{file} › {titles}")
                })
                .collect()
        }
        "pytest" => {
            let program = root.join(".venv/bin/pytest");
            let listing = run(
                workspace,
                program.to_str().unwrap(),
                &[
                    "--collect-only",
                    "-q",
                    "-p",
                    "no:randomly",
                    "-p",
                    "no:cacheprovider",
                ],
                exclusions,
            );
            listing
                .lines()
                .filter(|line| line.contains("::"))
                .map(str::to_owned)
                .collect()
        }
        other => panic!("no runner {other}"),
    };
    tests.sort();
    tests
}

#[rstest]
#[case::nextest(
    "nextest",
    "sample tests::flaky",
    &["sample tests::steady", "sample tests::steady_twin"]
)]
#[case::vitest("vitest", "sample > flaky", &["sample > steady (a+b)"])]
#[case::playwright(
    "playwright",
    "e2e/sample.spec.ts › sample › flaky",
    &["e2e/sample.spec.ts › sample › steady (a+b)"]
)]
#[case::pytest(
    "pytest",
    "tests/test_sample.py::test_flaky",
    &["tests/test_sample.py::test_steady"]
)]
fn runner_skips_the_quarantined_test_and_runs_the_others(
    #[case] runner: &str,
    #[case] id: &str,
    #[case] expected: &[&str],
) {
    let workspace = Workspace::from_fixture("quarantine");
    workspace.link_installed_runners();
    quarantine(&workspace, runner, id);
    let exclusions = exclusions(&workspace, runner);

    let tests = selected(&workspace, runner, &exclusions);

    assert_eq!(tests, expected);
}
