//! The self-hosted allowlist check: every job that may reach a self-hosted
//! runner is on the committed allowlist (ADRs 0108 and 0135).
//! Covers: story:1-98

use rstest::rstest;

use crate::workspace::{Workspace, finding};

#[test]
fn listed_and_hosted_jobs_pass() {
    let workspace = Workspace::from_fixture("self-hosted");

    let outcome = workspace.xtask(&["self-hosted", "check"]);

    assert_eq!(outcome.findings(), vec![]);
    assert_eq!(outcome.code, 0);
}

#[rstest]
#[case::self_hosted_label("self-hosted-label.yml")]
#[case::owner_machine_in_a_list("owner-machine-in-list.yml")]
#[case::macos_label_in_a_group("macos-label-in-group.yml")]
#[case::arm64_from_the_matrix("arm64-from-matrix.yml")]
fn job_reaching_a_self_hosted_runner_off_the_list_fails(#[case] workflow: &str) {
    let workspace = Workspace::from_fixture("self-hosted");
    workspace.add_fixture_file(
        &format!("self-hosted-cases/{workflow}"),
        ".github/workflows/extra.yml",
    );

    let outcome = workspace.xtask(&["self-hosted", "check"]);

    assert_eq!(
        outcome.findings(),
        vec![finding("off-allowlist", "extra.yml#build")]
    );
    assert_eq!(outcome.code, 1);
}

#[rstest]
#[case::another_condition("listed-job-other-condition.yml")]
#[case::other_events("listed-job-other-events.yml")]
fn listed_job_that_differs_from_its_entry_fails(#[case] workflow: &str) {
    let workspace = Workspace::from_fixture("self-hosted");
    workspace.add_fixture_file(
        &format!("self-hosted-cases/{workflow}"),
        ".github/workflows/pull-request.yml",
    );

    let outcome = workspace.xtask(&["self-hosted", "check"]);

    assert_eq!(
        outcome.findings(),
        vec![finding("allowlist-mismatch", "pull-request.yml#quick")]
    );
    assert_eq!(outcome.code, 1);
}

#[test]
fn export_lists_each_entry_once_per_event_and_ref() {
    let workspace = Workspace::from_fixture("self-hosted");

    let outcome = workspace.xtask(&["self-hosted", "export"]);

    assert_eq!(
        outcome.stdout,
        "repository\tgoodfetch/goodfetch\n\
         job\tpull-request.yml\tquick\tpull_request\trefs/pull/*/merge\n"
    );
    assert_eq!(outcome.code, 0);
}

#[test]
fn runner_chosen_by_an_unresolvable_expression_fails() {
    let workspace = Workspace::from_fixture("self-hosted");
    workspace.add_fixture_file(
        "self-hosted-cases/runner-from-expression.yml",
        ".github/workflows/extra.yml",
    );

    let outcome = workspace.xtask(&["self-hosted", "check"]);

    assert_eq!(
        outcome.findings(),
        vec![finding("unresolved-runs-on", "extra.yml#build")]
    );
    assert_eq!(outcome.code, 1);
}
