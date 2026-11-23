//! "all green": the pull request's required check passes only when every job
//! of the tier succeeded and no earlier attempt of its run had a failed test
//! step (ADRs 0061 and 0064).
//! Covers: story:1-51

use rstest::rstest;

use crate::github::GitHub;
use crate::workspace::{Finding, Outcome, Workspace, finding};

const PASSED_NEEDS: &str = r#"{
  "quick": { "result": "success", "outputs": {} },
  "pre-merge": { "result": "success", "outputs": {} }
}"#;

const ATTEMPT_1_JOBS: &str =
    "/repos/goodfetch/goodfetch/actions/runs/42/attempts/1/jobs?per_page=100&page=1";

fn all_green(workspace: &Workspace, needs: &str, attempt: &str, api: &str) -> Outcome {
    workspace.write("needs.json", needs.as_bytes());
    workspace.xtask(&[
        "all-green",
        "--needs",
        "needs.json",
        "--repository",
        "goodfetch/goodfetch",
        "--run-id",
        "42",
        "--attempt",
        attempt,
        "--api-url",
        api,
    ])
}

#[test]
fn first_attempt_whose_jobs_all_succeeded_passes() {
    let workspace = Workspace::from_fixture("empty");
    let github = GitHub::serving(&[]);

    let outcome = all_green(&workspace, PASSED_NEEDS, "1", github.url());

    assert_eq!(outcome.findings(), vec![]);
    assert_eq!(outcome.code, 0);
}

#[rstest]
#[case::failed("failure")]
#[case::cancelled("cancelled")]
#[case::skipped("skipped")]
fn job_that_did_not_succeed_fails(#[case] result: &str) {
    let workspace = Workspace::from_fixture("empty");
    let github = GitHub::serving(&[]);
    let needs = format!(
        r#"{{ "quick": {{ "result": "success" }}, "pre-merge": {{ "result": "{result}" }} }}"#
    );

    let outcome = all_green(&workspace, &needs, "1", github.url());

    assert_eq!(
        outcome.findings(),
        vec![finding("job-not-successful", "pre-merge")]
    );
    assert_eq!(outcome.code, 1);
}

#[rstest]
#[case::pre_merge("pre-merge")]
#[case::visual("visual")]
fn job_the_selection_skipped_passes(#[case] job: &str) {
    let workspace = Workspace::from_fixture("empty");
    let github = GitHub::serving(&[]);
    let needs = format!(
        r#"{{
      "affected": {{ "result": "success", "outputs": {{ "jobs": "[\"docs\"]" }} }},
      "quick": {{ "result": "success", "outputs": {{}} }},
      "{job}": {{ "result": "skipped", "outputs": {{}} }}
    }}"#
    );

    let outcome = all_green(&workspace, &needs, "1", github.url());

    assert_eq!(outcome.findings(), vec![]);
    assert_eq!(outcome.code, 0);
}

/// The hosted tier's `lint` and `quick` jobs run the steps their `plan` job
/// schedules for each, in its output named after the job (ADR 0135's
/// amendment of 2026-11-22): with none scheduled the job is skipped, and
/// passes; scheduled steps, no output for the job or a failed plan never let
/// a skip stand in for a pass.
#[rstest]
#[case::none_planned_for_quick(
    "quick",
    r#"{ "result": "success", "outputs": { "lint": "[\"lint\"]", "quick": "[]" } }"#,
    vec![]
)]
#[case::none_planned_for_lint(
    "lint",
    r#"{ "result": "success", "outputs": { "lint": "[]", "quick": "[\"quick\"]" } }"#,
    vec![]
)]
#[case::steps_planned(
    "quick",
    r#"{ "result": "success", "outputs": { "lint": "[]", "quick": "[\"quick\"]" } }"#,
    vec![finding("job-not-successful", "quick")]
)]
#[case::no_output_for_the_job(
    "lint",
    r#"{ "result": "success", "outputs": { "quick": "[]" } }"#,
    vec![finding("job-not-successful", "lint")]
)]
#[case::plan_failed(
    "quick",
    r#"{ "result": "failure", "outputs": {} }"#,
    vec![finding("job-not-successful", "plan"), finding("job-not-successful", "quick")]
)]
fn job_skipped_for_an_empty_plan_passes(
    #[case] job: &str,
    #[case] plan: &str,
    #[case] expected: Vec<Finding>,
) {
    let workspace = Workspace::from_fixture("empty");
    let github = GitHub::serving(&[]);
    let needs =
        format!(r#"{{ "plan": {plan}, "{job}": {{ "result": "skipped", "outputs": {{}} }} }}"#);

    let outcome = all_green(&workspace, &needs, "1", github.url());

    assert_eq!(outcome.findings(), expected);
}

#[test]
fn skipped_job_the_selection_never_chooses_fails() {
    let workspace = Workspace::from_fixture("empty");
    let github = GitHub::serving(&[]);
    let needs = r#"{
      "affected": { "result": "success", "outputs": { "jobs": "[\"pre-merge\"]" } },
      "quick": { "result": "skipped", "outputs": {} },
      "pre-merge": { "result": "success", "outputs": {} }
    }"#;

    let outcome = all_green(&workspace, needs, "1", github.url());

    assert_eq!(
        outcome.findings(),
        vec![finding("job-not-successful", "quick")]
    );
    assert_eq!(outcome.code, 1);
}

#[rstest]
#[case::selected(
    r#"{ "result": "success", "outputs": { "jobs": "[\"pre-merge\"]" } }"#,
    vec![finding("job-not-successful", "pre-merge")]
)]
#[case::selection_failed(
    r#"{ "result": "failure", "outputs": {} }"#,
    vec![
        finding("job-not-successful", "affected"),
        finding("job-not-successful", "pre-merge"),
    ]
)]
fn skipped_job_the_selection_did_not_skip_fails(
    #[case] affected: &str,
    #[case] expected: Vec<Finding>,
) {
    let workspace = Workspace::from_fixture("empty");
    let github = GitHub::serving(&[]);
    let needs = format!(
        r#"{{ "affected": {affected}, "pre-merge": {{ "result": "skipped", "outputs": {{}} }} }}"#
    );

    let outcome = all_green(&workspace, &needs, "1", github.url());

    assert_eq!(outcome.findings(), expected);
    assert_eq!(outcome.code, 1);
}

#[test]
fn rerun_after_a_failed_test_step_fails() {
    let workspace = Workspace::from_fixture("empty");
    let github = GitHub::serving(&[(
        ATTEMPT_1_JOBS,
        r#"{ "total_count": 2, "jobs": [
          { "name": "quick", "conclusion": "success",
            "steps": [ { "name": "Test: just quick all", "conclusion": "success" } ] },
          { "name": "pre-merge", "conclusion": "failure",
            "steps": [ { "name": "Start the VM", "conclusion": "success" },
                       { "name": "Test: just test pre-merge", "conclusion": "failure" } ] }
        ] }"#,
    )]);

    let outcome = all_green(&workspace, PASSED_NEEDS, "2", github.url());

    assert_eq!(
        outcome.findings(),
        vec![finding(
            "failed-test-step-in-earlier-attempt",
            "attempt-1/pre-merge"
        )]
    );
    assert_eq!(outcome.code, 1);
}

/// GitHub lists at most 100 jobs a page; a failed test step on the second
/// page counts as one on the first.
#[test]
fn failed_test_step_on_a_later_page_of_jobs_fails() {
    let workspace = Workspace::from_fixture("empty");
    let passed = r#"{ "name": "cell", "conclusion": "success", "steps": [] }"#;
    let first_page = format!(
        r#"{{ "total_count": 101, "jobs": [{}] }}"#,
        vec![passed; 100].join(", ")
    );
    let second_page = r#"{ "total_count": 101, "jobs": [
      { "name": "last-cell", "conclusion": "failure",
        "steps": [ { "name": "Test: Rust", "conclusion": "failure" } ] }
    ] }"#;
    let github = GitHub::serving(&[
        (ATTEMPT_1_JOBS, &first_page),
        (
            "/repos/goodfetch/goodfetch/actions/runs/42/attempts/1/jobs?per_page=100&page=2",
            second_page,
        ),
    ]);

    let outcome = all_green(&workspace, PASSED_NEEDS, "2", github.url());

    assert_eq!(
        outcome.findings(),
        vec![finding(
            "failed-test-step-in-earlier-attempt",
            "attempt-1/last-cell"
        )]
    );
    assert_eq!(outcome.code, 1);
}

#[test]
fn rerun_after_a_failure_outside_test_steps_passes() {
    let workspace = Workspace::from_fixture("empty");
    let github = GitHub::serving(&[(
        ATTEMPT_1_JOBS,
        r#"{ "total_count": 1, "jobs": [
          { "name": "pre-merge", "conclusion": "failure",
            "steps": [ { "name": "Start the VM", "conclusion": "failure" },
                       { "name": "Test: just test pre-merge", "conclusion": "skipped" } ] }
        ] }"#,
    )]);

    let outcome = all_green(&workspace, PASSED_NEEDS, "2", github.url());

    assert_eq!(outcome.findings(), vec![]);
    assert_eq!(outcome.code, 0);
}

#[test]
fn unreadable_earlier_attempt_is_an_error_never_a_pass() {
    let workspace = Workspace::from_fixture("empty");
    let github = GitHub::serving(&[]);

    let outcome = all_green(&workspace, PASSED_NEEDS, "2", github.url());

    assert_eq!(outcome.code, 2);
}
