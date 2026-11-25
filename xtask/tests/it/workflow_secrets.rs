//! The workflow policy's secrets rule (ADR 0076): a secret reaches only the
//! jobs `ci/secrets.toml` lists for it, never a whole workflow or a called
//! workflow by inheritance, and a job listed for a secret reads it.
//! Covers: story:1-76

use rstest::rstest;

use crate::workspace::{Workspace, finding};

#[test]
fn secret_read_only_by_its_listed_jobs_passes() {
    let workspace = Workspace::from_fixture("workflow-policy");

    let outcome = workspace.xtask(&["workflow-policy"]);

    assert_eq!(outcome.findings(), vec![], "{}", outcome.stderr);
    assert_eq!(outcome.code, 0);
}

#[rstest]
#[case::by_name("secret-unlisted.yml")]
#[case::by_index("secret-unlisted-by-index.yml")]
fn job_reading_a_secret_not_listed_for_it_fails(#[case] workflow: &str) {
    let workspace = Workspace::from_fixture("workflow-policy");
    workspace.add_fixture_file(
        &format!("workflow-policy-cases/{workflow}"),
        ".github/workflows/extra.yml",
    );

    let outcome = workspace.xtask(&["workflow-policy"]);

    assert_eq!(
        outcome.findings(),
        vec![finding("unlisted-secret", "extra.yml#build")]
    );
    assert_eq!(outcome.code, 1);
}

#[test]
fn secret_in_the_workflows_own_env_fails() {
    let workspace = Workspace::from_fixture("workflow-policy");
    workspace.add_fixture_file(
        "workflow-policy-cases/secret-in-workflow-env.yml",
        ".github/workflows/extra.yml",
    );

    let outcome = workspace.xtask(&["workflow-policy"]);

    assert_eq!(
        outcome.findings(),
        vec![finding("workflow-wide-secret", "extra.yml")]
    );
    assert_eq!(outcome.code, 1);
}

#[test]
fn called_workflow_inheriting_every_secret_fails() {
    let workspace = Workspace::from_fixture("workflow-policy");
    workspace.add_fixture_file(
        "workflow-policy-cases/secrets-inherited.yml",
        ".github/workflows/extra.yml",
    );

    let outcome = workspace.xtask(&["workflow-policy"]);

    assert_eq!(
        outcome.findings(),
        vec![finding("inherited-secrets", "extra.yml#call")]
    );
    assert_eq!(outcome.code, 1);
}

#[test]
fn job_listed_for_a_secret_it_does_not_read_fails() {
    let workspace = Workspace::from_fixture("workflow-policy");
    workspace.edit(
        "ci/secrets.toml",
        r#"jobs = ["after-run.yml#report"]"#,
        r#"jobs = ["after-run.yml#report", "pull-request.yml#quick"]"#,
    );

    let outcome = workspace.xtask(&["workflow-policy"]);

    assert_eq!(
        outcome.findings(),
        vec![finding("unread-secret", "pull-request.yml#quick")]
    );
    assert_eq!(outcome.code, 1);
}
