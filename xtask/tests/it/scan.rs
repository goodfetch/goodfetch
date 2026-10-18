//! The scan of the results store (ADR 0064): a test that passed and failed on
//! one tree hash and configuration is flaky and gets one `flaky-test` issue,
//! reopened on a recurrence; a Live check's fail, or its third inconclusive
//! result in a row, opens or updates its `live-check` issue. Issues are read
//! and written through a local stand-in for GitHub.
//! Covers: story:1-65, story:1-113

use rstest::rstest;
use serde_json::{Value, json};

use crate::github::{GitHub, Request};
use crate::store::{Attempt, STORE, test_run, with, write_store};
use crate::workspace::{Outcome, Workspace, finding};

const FLAKY_TEST_ISSUES: &str =
    "/repos/goodfetch/goodfetch/issues?labels=flaky-test&state=all&per_page=100&page=1";
const LIVE_CHECK_ISSUES: &str =
    "/repos/goodfetch/goodfetch/issues?labels=live-check&state=all&per_page=100&page=1";
const MILESTONES: &str = "/repos/goodfetch/goodfetch/milestones?state=open&per_page=100&page=1";

/// M1 is done, so M2 is the current milestone: the earliest with an open issue.
const OPEN_MILESTONES: &str = r#"[
  { "number": 1, "title": "M1 Core", "open_issues": 0, "due_on": null },
  { "number": 3, "title": "M3 Browser automation", "open_issues": 9, "due_on": null },
  { "number": 2, "title": "M2 Client surfaces", "open_issues": 4, "due_on": null }
]"#;

const ISSUES: &str = "/repos/goodfetch/goodfetch/issues";

fn scan(workspace: &Workspace, github: &GitHub) -> Outcome {
    workspace.xtask(&[
        "results",
        "scan",
        "--store",
        STORE,
        "--repository",
        "goodfetch/goodfetch",
        "--api-url",
        github.url(),
    ])
}

fn serving(flaky_test_issues: &str, live_check_issues: &str) -> GitHub {
    GitHub::serving(&[
        (FLAKY_TEST_ISSUES, flaky_test_issues),
        (LIVE_CHECK_ISSUES, live_check_issues),
        (MILESTONES, OPEN_MILESTONES),
        (
            "POST /repos/goodfetch/goodfetch/issues",
            r#"{ "number": 1700 }"#,
        ),
        (
            "PATCH /repos/goodfetch/goodfetch/issues/1601",
            r#"{ "number": 1601 }"#,
        ),
    ])
}

/// The fields of a write the scan sets, without the body's text.
fn summary(write: &Request) -> (String, String, Value) {
    let mut fields: Value = serde_json::from_str(&write.body).unwrap();
    if let Some(fields) = fields.as_object_mut() {
        fields.remove("body");
    }
    (write.method.clone(), write.path.clone(), fields)
}

fn attempt(run: u64, started_at: &'static str, test_runs: Vec<Value>) -> Attempt {
    let mut attempt = Attempt::new(run, started_at);
    attempt.test_runs = test_runs;
    attempt
}

#[test]
fn test_that_passed_and_failed_on_one_tree_and_configuration_gets_one_flaky_test_issue() {
    let workspace = Workspace::from_fixture("empty");
    write_store(
        &workspace,
        &[
            attempt(
                10,
                "2026-10-01T08:00:00Z",
                vec![test_run("sample tests::flaky", "passed", "aaa")],
            ),
            attempt(
                11,
                "2026-10-02T08:00:00Z",
                vec![
                    test_run("sample tests::flaky", "failed", "aaa"),
                    test_run("sample tests::steady", "passed", "aaa"),
                ],
            ),
        ],
    );
    let github = serving("[]", "[]");

    let outcome = scan(&workspace, &github);

    assert_eq!(outcome.code, 0);
    assert_eq!(
        github.writes().iter().map(summary).collect::<Vec<_>>(),
        vec![(
            "POST".to_owned(),
            ISSUES.to_owned(),
            json!({
                "title": "Flaky test: sample tests::flaky (nextest)",
                "labels": ["flaky-test"],
                "milestone": 2,
            })
        )]
    );
}

#[test]
fn test_that_passed_and_failed_on_different_trees_is_not_flaky() {
    let workspace = Workspace::from_fixture("empty");
    write_store(
        &workspace,
        &[
            attempt(
                10,
                "2026-10-01T08:00:00Z",
                vec![test_run("sample tests::changed", "passed", "aaa")],
            ),
            attempt(
                11,
                "2026-10-02T08:00:00Z",
                vec![test_run("sample tests::changed", "failed", "bbb")],
            ),
        ],
    );
    let github = serving("[]", "[]");

    let outcome = scan(&workspace, &github);

    assert_eq!(outcome.code, 0);
    assert_eq!(github.writes(), vec![]);
}

#[rstest]
#[case::os("os", json!("windows"))]
#[case::architecture("arch", json!("arm64"))]
#[case::runtime("runtime", json!("node-24"))]
fn test_that_passed_and_failed_in_different_configurations_is_not_flaky(
    #[case] field: &str,
    #[case] other: Value,
) {
    let workspace = Workspace::from_fixture("empty");
    write_store(
        &workspace,
        &[attempt(
            10,
            "2026-10-01T08:00:00Z",
            vec![
                test_run("sample tests::platform", "passed", "aaa"),
                with(
                    test_run("sample tests::platform", "failed", "aaa"),
                    field,
                    other,
                ),
            ],
        )],
    );
    let github = serving("[]", "[]");

    let outcome = scan(&workspace, &github);

    assert_eq!(outcome.code, 0);
    assert_eq!(github.writes(), vec![]);
}

/// A flaky test's results on one tree, in two tiers and two attempts.
fn flaky_on_one_tree(workspace: &Workspace, failed_at: &'static str) {
    let mut retried = attempt(
        11,
        failed_at,
        vec![with(
            test_run("sample tests::flaky", "failed", "aaa"),
            "tier",
            json!("diagnostic"),
        )],
    );
    retried.number = 2;
    write_store(
        workspace,
        &[
            attempt(
                10,
                "2026-10-01T08:00:00Z",
                vec![test_run("sample tests::flaky", "passed", "aaa")],
            ),
            retried,
        ],
    );
}

fn flaky_test_issue(state: &str, closed_at: &str) -> String {
    format!(
        r#"[{{ "number": 1601, "state": "{state}", "closed_at": {closed_at},
             "body": "<!-- goodfetch-results: flaky-test nextest sample tests::flaky -->\nOld results." }}]"#
    )
}

#[test]
fn flaky_test_with_an_open_issue_updates_that_issue() {
    let workspace = Workspace::from_fixture("empty");
    flaky_on_one_tree(&workspace, "2026-10-02T08:00:00Z");
    let github = serving(&flaky_test_issue("open", "null"), "[]");

    let outcome = scan(&workspace, &github);

    assert_eq!(outcome.code, 0);
    assert_eq!(
        github.writes().iter().map(summary).collect::<Vec<_>>(),
        vec![("PATCH".to_owned(), format!("{ISSUES}/1601"), json!({}))]
    );
}

#[test]
fn flaky_test_that_recurs_after_its_issue_closed_reopens_it() {
    let workspace = Workspace::from_fixture("empty");
    flaky_on_one_tree(&workspace, "2026-10-09T08:00:00Z");
    let github = serving(
        &flaky_test_issue("closed", r#""2026-10-05T08:00:00Z""#),
        "[]",
    );

    let outcome = scan(&workspace, &github);

    assert_eq!(outcome.code, 0);
    assert_eq!(
        github.writes().iter().map(summary).collect::<Vec<_>>(),
        vec![(
            "PATCH".to_owned(),
            format!("{ISSUES}/1601"),
            json!({ "state": "open", "milestone": 2 })
        )]
    );
}

#[test]
fn flaky_test_whose_results_all_came_before_its_issue_closed_stays_closed() {
    let workspace = Workspace::from_fixture("empty");
    flaky_on_one_tree(&workspace, "2026-10-02T08:00:00Z");
    let github = serving(
        &flaky_test_issue("closed", r#""2026-10-05T08:00:00Z""#),
        "[]",
    );

    let outcome = scan(&workspace, &github);

    assert_eq!(outcome.code, 0);
    assert_eq!(github.writes(), vec![]);
}

/// Declares the fixture Live check, owned by area 6 (Collection).
fn declare_live_check(workspace: &Workspace) {
    workspace.write(
        "ci/live-checks.toml",
        b"[[live-check]]\nid = \"saas-sinks snowflake\"\narea = 6\n",
    );
}

#[test]
fn undeclared_live_check_still_opens_its_issue_and_fails_the_scan() {
    let workspace = Workspace::from_fixture("empty");
    workspace.write("ci/live-checks.toml", b"");
    live_check_results(&workspace, &["failed"]);
    let github = serving("[]", "[]");

    let outcome = scan(&workspace, &github);

    assert_eq!(
        outcome.findings(),
        vec![finding("undeclared-live-check", "saas-sinks snowflake")]
    );
    assert_eq!(outcome.code, 1);
    assert_eq!(
        github.writes().iter().map(summary).collect::<Vec<_>>(),
        vec![(
            "POST".to_owned(),
            ISSUES.to_owned(),
            json!({
                "title": "Live check: saas-sinks snowflake (area undeclared)",
                "labels": ["live-check"],
                "milestone": 2,
            })
        )]
    );
}

/// A Live check's results, one attempt a day.
fn live_check_results(workspace: &Workspace, outcomes: &[&str]) {
    const DAYS: [&str; 4] = [
        "2026-10-01T03:00:00Z",
        "2026-10-02T03:00:00Z",
        "2026-10-03T03:00:00Z",
        "2026-10-04T03:00:00Z",
    ];
    let attempts: Vec<Attempt> = outcomes
        .iter()
        .zip(DAYS)
        .zip(20..)
        .map(|((outcome, day), run)| {
            let result = with(
                test_run("saas-sinks snowflake", outcome, "aaa"),
                "tier",
                json!("live"),
            );
            attempt(run, day, vec![with(result, "runner", json!("live-check"))])
        })
        .collect();
    write_store(workspace, &attempts);
}

#[rstest]
#[case::third_inconclusive_in_a_row(&["passed", "inconclusive", "inconclusive", "inconclusive"])]
#[case::fail(&["passed", "passed", "failed"])]
fn live_check_that_failed_or_was_inconclusive_three_times_in_a_row_opens_its_issue(
    #[case] outcomes: &[&str],
) {
    let workspace = Workspace::from_fixture("empty");
    declare_live_check(&workspace);
    live_check_results(&workspace, outcomes);
    let github = serving("[]", "[]");

    let outcome = scan(&workspace, &github);

    assert_eq!(outcome.code, 0);
    assert_eq!(
        github.writes().iter().map(summary).collect::<Vec<_>>(),
        vec![(
            "POST".to_owned(),
            ISSUES.to_owned(),
            json!({
                "title": "Live check: saas-sinks snowflake (area 6)",
                "labels": ["live-check"],
                "milestone": 2,
            })
        )]
    );
}

#[rstest]
#[case::second_inconclusive_in_a_row(&["passed", "inconclusive", "inconclusive"])]
#[case::inconclusive_three_times_not_in_a_row(&["inconclusive", "inconclusive", "passed", "inconclusive"])]
#[case::fail_then_pass(&["failed", "passed"])]
fn live_check_whose_last_results_are_neither_a_fail_nor_three_inconclusive_opens_no_issue(
    #[case] outcomes: &[&str],
) {
    let workspace = Workspace::from_fixture("empty");
    live_check_results(&workspace, outcomes);
    let github = serving("[]", "[]");

    let outcome = scan(&workspace, &github);

    assert_eq!(outcome.code, 0);
    assert_eq!(github.writes(), vec![]);
}
