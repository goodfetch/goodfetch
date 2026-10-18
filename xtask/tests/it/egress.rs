//! Which jobs run under ADR 0043's outbound-traffic deny in CI: every job is
//! declared under it or outside it, and a job under it denies before its
//! first test step, runs the Egress canary right after, and checks the
//! deny's log at its end.
//! Covers: story:1-69

use rstest::rstest;

use crate::workspace::{Workspace, finding};

#[test]
fn declared_jobs_with_the_deny_before_their_tests_pass() {
    let workspace = Workspace::from_fixture("egress");

    let outcome = workspace.xtask(&["egress", "check"]);

    assert_eq!(outcome.findings(), vec![]);
    assert_eq!(outcome.code, 0);
}

#[test]
fn job_not_declared_under_or_outside_the_deny_fails() {
    let workspace = Workspace::from_fixture("egress");
    workspace.add_fixture_file(
        "egress-cases/undeclared-job.yml",
        ".github/workflows/extra.yml",
    );

    let outcome = workspace.xtask(&["egress", "check"]);

    assert_eq!(
        outcome.findings(),
        vec![finding("undeclared-job", "extra.yml#build")]
    );
    assert_eq!(outcome.code, 1);
}

#[rstest]
#[case::without_the_deny("quick-without-deny.yml", "missing-deny", "pull-request.yml#quick")]
#[case::test_step_before_the_deny(
    "quick-test-before-deny.yml",
    "test-before-deny",
    "pull-request.yml#quick"
)]
#[case::canary_not_right_after_the_deny(
    "quick-canary-not-after-deny.yml",
    "missing-canary",
    "pull-request.yml#quick"
)]
#[case::log_check_skipped_on_failure(
    "quick-check-not-always.yml",
    "missing-egress-check",
    "pull-request.yml#quick"
)]
#[case::outside_job_running_the_deny(
    "all-green-with-deny.yml",
    "deny-outside-rule",
    "pull-request.yml#all-green"
)]
fn job_breaking_the_deny_order_fails(
    #[case] workflow: &str,
    #[case] kind: &str,
    #[case] subject: &str,
) {
    let workspace = Workspace::from_fixture("egress");
    workspace.add_fixture_file(
        &format!("egress-cases/{workflow}"),
        ".github/workflows/pull-request.yml",
    );

    let outcome = workspace.xtask(&["egress", "check"]);

    assert_eq!(outcome.findings(), vec![finding(kind, subject)]);
    assert_eq!(outcome.code, 1);
}

#[rstest]
#[case::declared_twice(
    "[[outside]]\njob = \"pull-request.yml#quick\"\nreason = \"It runs none of the product.\"\n",
    "declared-twice",
    "pull-request.yml#quick"
)]
#[case::declared_without_a_job(
    "[[deny]]\njob = \"release.yml#test\"\n",
    "unknown-job",
    "release.yml#test"
)]
fn declaration_that_matches_no_single_job_fails(
    #[case] declaration: &str,
    #[case] kind: &str,
    #[case] subject: &str,
) {
    let workspace = Workspace::from_fixture("egress");
    workspace.append("ci/egress/endpoints.toml", declaration);

    let outcome = workspace.xtask(&["egress", "check"]);

    assert_eq!(outcome.findings(), vec![finding(kind, subject)]);
    assert_eq!(outcome.code, 1);
}
