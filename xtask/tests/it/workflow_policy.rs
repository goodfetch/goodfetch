//! The workflow policy check (ADR 0108's amendment of 2026-11-22): every job
//! runs on one of GitHub's standard hosted runners, no workflow runs a pull
//! request's code with the base repository's rights, every workflow starts
//! with no permissions, and every Action is pinned by commit.

use std::path::Path;

use rstest::rstest;

use crate::workspace::{Workspace, finding};

#[test]
fn standard_runners_pinned_actions_and_no_top_level_permissions_pass() {
    let workspace = Workspace::from_fixture("workflow-policy");

    let outcome = workspace.xtask(&["workflow-policy"]);

    assert_eq!(outcome.findings(), vec![], "{}", outcome.stderr);
    assert_eq!(outcome.code, 0);
}

#[test]
fn workflows_of_this_repository_pass() {
    let workspace = Workspace::from_fixture("workflow-policy");
    workspace.remove_folder(".github/workflows");
    let workflows = Path::new(env!("CARGO_MANIFEST_DIR")).join("../.github/workflows");
    for entry in std::fs::read_dir(&workflows).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_str().unwrap();
        workspace.write(
            &format!(".github/workflows/{name}"),
            &std::fs::read(&path).unwrap(),
        );
    }

    let outcome = workspace.xtask(&["workflow-policy"]);

    assert_eq!(outcome.findings(), vec![], "{}", outcome.stderr);
    assert_eq!(outcome.code, 0);
}

fn check_with(case: &str) -> crate::workspace::Outcome {
    let workspace = Workspace::from_fixture("workflow-policy");
    workspace.add_fixture_file(
        &format!("workflow-policy-cases/{case}"),
        ".github/workflows/extra.yml",
    );
    workspace.xtask(&["workflow-policy"])
}

#[rstest]
#[case::self_hosted_label("self-hosted-label.yml")]
#[case::larger_runner_label("larger-runner-label.yml")]
#[case::macos_larger_runner_from_the_matrix("macos-larger-runner-from-matrix.yml")]
#[case::runner_group("runner-group.yml")]
#[case::standard_label_beside_another("standard-label-beside-another.yml")]
fn job_off_a_standard_hosted_runner_fails(#[case] workflow: &str) {
    let outcome = check_with(workflow);

    assert_eq!(
        outcome.findings(),
        vec![finding("non-standard-runner", "extra.yml#build")]
    );
    assert_eq!(outcome.code, 1);
}

#[test]
fn runner_chosen_by_an_unresolvable_expression_fails() {
    let outcome = check_with("runner-from-expression.yml");

    assert_eq!(
        outcome.findings(),
        vec![finding("unresolved-runs-on", "extra.yml#build")]
    );
    assert_eq!(outcome.code, 1);
}

#[test]
fn pull_request_target_fails() {
    let outcome = check_with("pull-request-target.yml");

    assert_eq!(
        outcome.findings(),
        vec![finding("pull-request-target", "extra.yml")]
    );
    assert_eq!(outcome.code, 1);
}

#[rstest]
#[case::head_commit("workflow-run-checks-out-the-head-commit.yml")]
#[case::head_repository("workflow-run-checks-out-the-head-repository.yml")]
#[case::pull_request_ref("workflow-run-checks-out-a-pull-request-ref.yml")]
fn workflow_run_checking_out_a_pull_requests_code_fails(#[case] workflow: &str) {
    let outcome = check_with(workflow);

    assert_eq!(
        outcome.findings(),
        vec![finding("checks-out-pull-request", "extra.yml#build")]
    );
    assert_eq!(outcome.code, 1);
}

#[rstest]
#[case::none("without-permissions.yml")]
#[case::granted("permissions-granted-at-the-top.yml")]
#[case::read_all("read-all-permissions.yml")]
fn workflow_without_empty_top_level_permissions_fails(#[case] workflow: &str) {
    let outcome = check_with(workflow);

    assert_eq!(
        outcome.findings(),
        vec![finding("top-level-permissions", "extra.yml")]
    );
    assert_eq!(outcome.code, 1);
}

#[rstest]
#[case::action_by_tag("action-by-tag.yml")]
#[case::action_by_short_commit("action-by-short-commit.yml")]
#[case::docker_image_by_tag("docker-image-by-tag.yml")]
#[case::reusable_workflow_by_branch("reusable-workflow-by-branch.yml")]
fn uses_not_pinned_by_commit_or_digest_fails(#[case] workflow: &str) {
    let outcome = check_with(workflow);

    assert_eq!(
        outcome.findings(),
        vec![finding("unpinned-uses", "extra.yml#build")]
    );
    assert_eq!(outcome.code, 1);
}
