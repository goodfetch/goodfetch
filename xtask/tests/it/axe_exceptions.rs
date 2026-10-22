//! The axe exception file (ADR 0073): every entry of
//! `console/axe-exceptions.toml` has its rule, selector, pages, reason and
//! issue, and its issue is still an open issue of the repository. Issues are read
//! from a local stand-in for GitHub, as the quarantine list's check reads
//! them.

use rstest::rstest;

use crate::github::GitHub;
use crate::workspace::{Outcome, Workspace, finding};

const ISSUE: &str = "/repos/goodfetch/goodfetch/issues/1701";

const OPEN_ISSUE: &str = r#"{ "number": 1701, "state": "open", "labels": [] }"#;

const ENTRY: &str = "[[exception]]\nrule = \"color-contrast\"\nselector = \"#brand\"\npages = [\"/runs/:run_id\"]\nreason = \"axe reads the gradient behind the text as its colour\"\nissue = 1701\n";

fn check(workspace: &Workspace, github: &GitHub) -> Outcome {
    workspace.xtask(&["axe-exceptions", "check", "--api-url", github.url()])
}

#[test]
fn entry_with_an_open_issue_passes() {
    let workspace = Workspace::from_fixture("empty");
    workspace.write("console/axe-exceptions.toml", ENTRY.as_bytes());
    let github = GitHub::serving(&[(ISSUE, OPEN_ISSUE)]);

    let outcome = check(&workspace, &github);

    assert_eq!(outcome.findings(), vec![]);
    assert_eq!(outcome.code, 0);
}

#[test]
fn entry_whose_issue_is_closed_fails() {
    let workspace = Workspace::from_fixture("empty");
    workspace.write("console/axe-exceptions.toml", ENTRY.as_bytes());
    let github = GitHub::serving(&[(
        ISSUE,
        r#"{ "number": 1701, "state": "closed", "labels": [] }"#,
    )]);

    let outcome = check(&workspace, &github);

    assert_eq!(
        outcome.findings(),
        vec![finding("axe-exception-issue-closed", "exception 1")]
    );
    assert_eq!(outcome.code, 1);
}

#[rstest]
#[case::absent(&[])]
#[case::a_pull_request(&[(
    ISSUE,
    r#"{ "number": 1701, "state": "open", "labels": [],
         "pull_request": { "url": "https://api.github.com/repos/goodfetch/goodfetch/pulls/1701" } }"#,
)])]
fn entry_whose_issue_does_not_exist_fails(#[case] routes: &[(&str, &str)]) {
    let workspace = Workspace::from_fixture("empty");
    workspace.write("console/axe-exceptions.toml", ENTRY.as_bytes());
    let github = GitHub::serving(routes);

    let outcome = check(&workspace, &github);

    assert_eq!(
        outcome.findings(),
        vec![finding("axe-exception-issue-missing", "exception 1")]
    );
    assert_eq!(outcome.code, 1);
}

#[rstest]
#[case::rule("rule = \"color-contrast\"\n")]
#[case::selector("selector = \"#brand\"\n")]
#[case::pages("pages = [\"/runs/:run_id\"]\n")]
#[case::reason("reason = \"axe reads the gradient behind the text as its colour\"\n")]
#[case::issue("issue = 1701\n")]
fn entry_missing_a_field_fails(#[case] removed: &str) {
    let workspace = Workspace::from_fixture("empty");
    workspace.write(
        "console/axe-exceptions.toml",
        ENTRY.replace(removed, "").as_bytes(),
    );
    let github = GitHub::serving(&[(ISSUE, OPEN_ISSUE)]);

    let outcome = check(&workspace, &github);

    assert_eq!(
        outcome.findings(),
        vec![finding("axe-exception-field-missing", "exception 1")]
    );
    assert_eq!(outcome.code, 1);
}

#[rstest]
#[case::rule("rule = \"color-contrast\"", "rule = \"\"")]
#[case::selector("selector = \"#brand\"", "selector = \"  \"")]
#[case::reason(
    "reason = \"axe reads the gradient behind the text as its colour\"",
    "reason = \"\""
)]
fn entry_with_an_empty_field_fails(#[case] field: &str, #[case] empty: &str) {
    let workspace = Workspace::from_fixture("empty");
    workspace.write(
        "console/axe-exceptions.toml",
        ENTRY.replace(field, empty).as_bytes(),
    );
    let github = GitHub::serving(&[(ISSUE, OPEN_ISSUE)]);

    let outcome = check(&workspace, &github);

    assert_eq!(
        outcome.findings(),
        vec![finding("axe-exception-field-missing", "exception 1")]
    );
    assert_eq!(outcome.code, 1);
}

#[test]
fn entry_naming_no_page_fails() {
    let workspace = Workspace::from_fixture("empty");
    workspace.write(
        "console/axe-exceptions.toml",
        ENTRY
            .replace("pages = [\"/runs/:run_id\"]", "pages = []")
            .as_bytes(),
    );
    let github = GitHub::serving(&[(ISSUE, OPEN_ISSUE)]);

    let outcome = check(&workspace, &github);

    assert_eq!(
        outcome.findings(),
        vec![finding("axe-exception-field-missing", "exception 1")]
    );
    assert_eq!(outcome.code, 1);
}
