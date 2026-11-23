//! The scan of the results store (ADR 0064): a test that passed and failed on
//! one tree hash and configuration is flaky and gets one `flaky-test` issue,
//! reopened on a recurrence; a Live check's fail, or its third inconclusive
//! result in a row, opens or updates its `live-check` issue. A new issue is a
//! sub-issue of the spec that owns its test, in the milestone of that spec's
//! ticket that added it, found from the commit that added the test (ADR
//! 0136); with no owning spec found it opens with `needs-triage` and no
//! milestone. Issues are read and written through a local stand-in for
//! GitHub, and the workspace is a Git repository.
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

const ISSUES: &str = "/repos/goodfetch/goodfetch/issues";

/// The spec whose ticket 1490, in milestone 3, added the fixture's test.
const SPEC: u64 = 130;

/// The issue ID GitHub gives the issue the scan opens, which adding a
/// sub-issue names.
const OPENED_ID: u64 = 9_001;

/// GitHub's answers for the commit `commit` that added a test: its merged
/// pull requests, each with its body, and ticket 1490 of spec 130, in
/// milestone 3.
fn added_by(commit: &str, pull_requests: &str) -> Vec<(String, String)> {
    vec![
        (
            format!("/repos/goodfetch/goodfetch/commits/{commit}/pulls"),
            pull_requests.to_owned(),
        ),
        (format!("{ISSUES}/1490"), ticket(1490, Some(SPEC), Some(3))),
        (
            format!("POST {ISSUES}/{SPEC}/sub_issues"),
            format!(r#"{{ "number": {SPEC} }}"#),
        ),
    ]
}

/// A ticket as GitHub answers it, with its parent spec and milestone.
fn ticket(number: u64, spec: Option<u64>, milestone: Option<u64>) -> String {
    json!({
        "number": number,
        "milestone": milestone.map(|number| json!({ "number": number })),
        "parent_issue_url": spec.map(|spec| format!("https://api.github.com/repos/goodfetch/goodfetch/issues/{spec}")),
    })
    .to_string()
}

/// One merged pull request whose body has `closes`, one `Closes #<n>` line
/// each.
fn merged_pull_request(number: u64, closes: &[u64]) -> Value {
    let lines = closes
        .iter()
        .map(|ticket| format!("Closes #{ticket}"))
        .collect::<Vec<_>>()
        .join("\n");
    merged_with_body(
        number,
        &format!("## Summary\n\nAdds the test; Closes #77 in prose names nothing.\n\n{lines}"),
    )
}

/// One merged pull request whose body is `body`.
fn merged_with_body(number: u64, body: &str) -> Value {
    json!({
        "number": number,
        "merged_at": "2026-09-30T08:00:00Z",
        "body": body,
    })
}

/// The test the fixture store's flaky results are of, as Rust source.
const FLAKY_TEST_SOURCE: &[u8] = b"#[test]\nfn flaky() {}\n";

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
    serving_with(flaky_test_issues, live_check_issues, &[])
}

/// As [`serving`], with `more` routes: those of a test's owner.
fn serving_with(
    flaky_test_issues: &str,
    live_check_issues: &str,
    more: &[(String, String)],
) -> GitHub {
    let opened = format!(r#"{{ "number": 1700, "id": {OPENED_ID} }}"#);
    let mut routes = vec![
        (FLAKY_TEST_ISSUES, flaky_test_issues),
        (LIVE_CHECK_ISSUES, live_check_issues),
        ("POST /repos/goodfetch/goodfetch/issues", opened.as_str()),
        (
            "PATCH /repos/goodfetch/goodfetch/issues/1601",
            r#"{ "number": 1601 }"#,
        ),
    ];
    routes.extend(
        more.iter()
            .map(|(path, body)| (path.as_str(), body.as_str())),
    );
    GitHub::serving(&routes)
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

/// `sample tests::flaky` passed, then failed, on one tree; `sample
/// tests::steady` passed.
fn flaky_and_steady(workspace: &Workspace) {
    write_store(
        workspace,
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
}

/// The writes that open a new issue titled `title` under `label` as a
/// sub-issue of [`SPEC`] in milestone 3.
fn opened_under_the_spec(label: &str, title: &str) -> Vec<(String, String, Value)> {
    vec![
        (
            "POST".to_owned(),
            ISSUES.to_owned(),
            json!({ "title": title, "labels": [label], "milestone": 3 }),
        ),
        (
            "POST".to_owned(),
            format!("{ISSUES}/{SPEC}/sub_issues"),
            json!({ "sub_issue_id": OPENED_ID }),
        ),
    ]
}

/// The write that opens a new issue titled `title` under `label` for triage,
/// with no parent and no milestone.
fn opened_for_triage(label: &str, title: &str) -> Vec<(String, String, Value)> {
    vec![(
        "POST".to_owned(),
        ISSUES.to_owned(),
        json!({ "title": title, "labels": [label, "needs-triage"] }),
    )]
}

/// The ticket's own pull request and the batch that merged it with a
/// ticket of another spec: the ticket's own names the one that added it.
#[rstest]
#[case::its_own_pull_request(&[merged_pull_request(1500, &[1490])])]
#[case::its_own_beside_a_batch(&[merged_pull_request(1500, &[1490]), merged_pull_request(1510, &[1490, 1491])])]
#[case::its_closing_line_in_a_bulleted_list(&[merged_with_body(1500, "## Tickets\n\n- Closes #1490")])]
#[case::its_closing_line_in_a_numbered_list(&[merged_with_body(1500, "1. Closes #1490")])]
fn test_that_passed_and_failed_on_one_tree_and_configuration_gets_one_flaky_test_issue_under_the_spec_that_added_it(
    #[case] pull_requests: &[Value],
) {
    let workspace = Workspace::from_fixture("empty");
    flaky_and_steady(&workspace);
    workspace.write("tests/sample.rs", FLAKY_TEST_SOURCE);
    let commit = workspace.commit_all();
    let mut routes = added_by(&commit, &json!(pull_requests).to_string());
    routes.push((format!("{ISSUES}/1491"), ticket(1491, Some(212), Some(3))));
    let github = serving_with("[]", "[]", &routes);

    let outcome = scan(&workspace, &github);

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    assert_eq!(
        github.writes().iter().map(summary).collect::<Vec<_>>(),
        opened_under_the_spec("flaky-test", "Flaky test: sample tests::flaky (nextest)")
    );
}

/// The commit `{commit}` in the answers writes the test's source in this
/// file, if any, and has these merged pull requests; ticket 1491 is spec
/// 212's, 1492 has no parent.
#[rstest]
#[case::no_commit_added_it(None, "[]")]
#[case::no_merged_pull_request(
    Some("tests/sample.rs"),
    r#"[{ "number": 1500, "merged_at": null, "body": "Closes #1490" }]"#
)]
#[case::no_closing_line(
    Some("tests/sample.rs"),
    r#"[{ "number": 1500, "merged_at": "2026-09-30T08:00:00Z", "body": "Part of #1490" }]"#
)]
#[case::a_list_marker_with_no_space_after_it(
    Some("tests/sample.rs"),
    r#"[{ "number": 1500, "merged_at": "2026-09-30T08:00:00Z", "body": "-Closes #1490" }]"#
)]
#[case::two_closings_on_one_line(
    Some("tests/sample.rs"),
    r#"[{ "number": 1500, "merged_at": "2026-09-30T08:00:00Z", "body": "Closes #1490, Closes #1491" }]"#
)]
#[case::written_only_outside_a_test_file(
    Some("docs/flaky.md"),
    r#"[{ "number": 1500, "merged_at": "2026-09-30T08:00:00Z", "body": "Closes #1490" }]"#
)]
#[case::tickets_of_two_specs(Some("tests/sample.rs"), r#"[{ "number": 1510, "merged_at": "2026-09-30T08:00:00Z", "body": "Closes #1490\nCloses #1491" }]"#)]
#[case::ticket_of_no_spec(
    Some("tests/sample.rs"),
    r#"[{ "number": 1500, "merged_at": "2026-09-30T08:00:00Z", "body": "Closes #1492" }]"#
)]
fn flaky_test_with_no_owning_spec_found_opens_its_issue_for_triage_with_no_milestone(
    #[case] written_in: Option<&str>,
    #[case] pull_requests: &str,
) {
    let workspace = Workspace::from_fixture("empty");
    flaky_and_steady(&workspace);
    if let Some(file) = written_in {
        workspace.write(file, FLAKY_TEST_SOURCE);
    }
    let commit = workspace.commit_all();
    let mut routes = added_by(&commit, pull_requests);
    routes.push((format!("{ISSUES}/1491"), ticket(1491, Some(212), Some(3))));
    routes.push((format!("{ISSUES}/1492"), ticket(1492, None, Some(3))));
    let github = serving_with("[]", "[]", &routes);

    let outcome = scan(&workspace, &github);

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    assert_eq!(
        github.writes().iter().map(summary).collect::<Vec<_>>(),
        opened_for_triage("flaky-test", "Flaky test: sample tests::flaky (nextest)")
    );
}

/// `test` of `runner` passed, then failed, on one tree.
fn flaky(workspace: &Workspace, runner: &str, test: &str) {
    let run = |outcome| with(test_run(test, outcome, "aaa"), "runner", json!(runner));
    write_store(
        workspace,
        &[
            attempt(10, "2026-10-01T08:00:00Z", vec![run("passed")]),
            attempt(11, "2026-10-02T08:00:00Z", vec![run("failed")]),
        ],
    );
}

/// Each runner names a test its own way; the commit that added it is the
/// first to write that name in a test file, not an earlier one that only
/// mentions it elsewhere, whose pull requests the stand-in does not know.
#[rstest]
#[case::an_rstest_case(
    "nextest",
    "sample tests::opens::case_2_its_own",
    "tests/sample.rs",
    "#[rstest]\n#[case::its_own(2)]\nfn opens(#[case] n: u8) {}\n"
)]
#[case::a_nextest_test_named_like_a_case_with_no_number(
    "nextest",
    "sample tests::case_study",
    "tests/sample.rs",
    "#[test]\nfn case_study() {}\n"
)]
#[case::a_parametrized_pytest_test_in_a_class(
    "pytest",
    "tests/test_page.py::TestTitle::test_title[a-b]",
    "tests/test_page.py",
    "class TestTitle:\n    def test_title(self, a, b):\n        pass\n"
)]
#[case::a_vitest_test(
    "vitest",
    "session > opens a page",
    "src/session.test.ts",
    "describe('session', () => {\n  it('opens a page', () => {});\n});\n"
)]
#[case::a_playwright_test(
    "playwright",
    "tests/page.spec.ts › page › opens",
    "tests/page.spec.ts",
    "test.describe('page', () => {\n  test('opens', async () => {});\n});\n"
)]
fn flaky_test_of_each_runner_gets_its_issue_under_the_spec_whose_test_file_added_it(
    #[case] runner: &str,
    #[case] test: &str,
    #[case] file: &str,
    #[case] source: &str,
) {
    let workspace = Workspace::from_fixture("empty");
    flaky(&workspace, runner, test);
    workspace.write("NOTES.md", source.as_bytes());
    workspace.commit_all();
    workspace.write(file, source.as_bytes());
    let commit = workspace.commit_all();
    let github = serving_with(
        "[]",
        "[]",
        &added_by(
            &commit,
            &json!([merged_pull_request(1500, &[1490])]).to_string(),
        ),
    );

    let outcome = scan(&workspace, &github);

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    assert_eq!(
        github.writes().iter().map(summary).collect::<Vec<_>>(),
        opened_under_the_spec("flaky-test", &format!("Flaky test: {test} ({runner})"))
    );
}

/// An issue that opened but could not join its spec is left for triage, and
/// the scan fails.
#[test]
fn issue_that_cannot_join_its_spec_is_labelled_for_triage_and_fails_the_scan() {
    let workspace = Workspace::from_fixture("empty");
    flaky_and_steady(&workspace);
    workspace.write("tests/sample.rs", FLAKY_TEST_SOURCE);
    let commit = workspace.commit_all();
    let mut routes = added_by(
        &commit,
        &json!([merged_pull_request(1500, &[1490])]).to_string(),
    );
    routes.retain(|(path, _)| !path.ends_with("/sub_issues"));
    routes.push((format!("POST {ISSUES}/1700/labels"), "[]".to_owned()));
    let github = serving_with("[]", "[]", &routes);

    let outcome = scan(&workspace, &github);

    assert_ne!(outcome.code, 0);
    assert!(outcome.stderr.contains("#130"), "{}", outcome.stderr);
    let mut expected =
        opened_under_the_spec("flaky-test", "Flaky test: sample tests::flaky (nextest)");
    expected.push((
        "POST".to_owned(),
        format!("{ISSUES}/1700/labels"),
        json!({ "labels": ["needs-triage"] }),
    ));
    assert_eq!(
        github.writes().iter().map(summary).collect::<Vec<_>>(),
        expected
    );
}

#[test]
fn scan_on_a_shallow_clone_opens_no_issue_and_fails() {
    let source = Workspace::from_fixture("empty");
    source.write("tests/sample.rs", FLAKY_TEST_SOURCE);
    source.commit_all();
    source.write("README.md", b"A second commit.\n");
    source.commit_all();
    let workspace = Workspace::from_fixture("empty");
    workspace.git(&["init", "--quiet"]);
    let from = format!("file://{}", source.path().display());
    workspace.git(&["fetch", "--quiet", "--depth=1", &from, "HEAD"]);
    // The fixture's own README.md is in the way of the fetched one.
    workspace.git(&["checkout", "--quiet", "--force", "FETCH_HEAD"]);
    flaky_and_steady(&workspace);
    let github = serving("[]", "[]");

    let outcome = scan(&workspace, &github);

    assert_ne!(outcome.code, 0);
    assert!(outcome.stderr.contains("shallow"), "{}", outcome.stderr);
    assert_eq!(github.writes(), vec![]);
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
            json!({ "state": "open" })
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
    workspace.commit_all();
    let github = serving("[]", "[]");

    let outcome = scan(&workspace, &github);

    assert_eq!(
        outcome.findings(),
        vec![finding("undeclared-live-check", "saas-sinks snowflake")]
    );
    assert_eq!(outcome.code, 1);
    assert_eq!(
        github.writes().iter().map(summary).collect::<Vec<_>>(),
        opened_for_triage(
            "live-check",
            "Live check: saas-sinks snowflake (area undeclared)"
        )
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
    live_check_results(&workspace, outcomes);
    workspace.commit_all();
    declare_live_check(&workspace);
    let commit = workspace.commit_all();
    let github = serving_with(
        "[]",
        "[]",
        &added_by(
            &commit,
            &json!([merged_pull_request(1500, &[1490])]).to_string(),
        ),
    );

    let outcome = scan(&workspace, &github);

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    assert_eq!(
        github.writes().iter().map(summary).collect::<Vec<_>>(),
        opened_under_the_spec("live-check", "Live check: saas-sinks snowflake (area 6)")
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
