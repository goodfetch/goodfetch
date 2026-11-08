//! `xtask dco`: every commit a pull request adds carries a Developer
//! Certificate of Origin sign-off by its author, with no CLA (ADR 0018).
//! Covers: story:1-58

use crate::workspace::{Workspace, finding};

/// Commits an empty change with the given message paragraphs and returns
/// the new commit.
fn commit(workspace: &Workspace, paragraphs: &[&str]) -> String {
    let mut args = vec!["commit", "--quiet", "--allow-empty"];
    for paragraph in paragraphs {
        args.extend(["--message", paragraph]);
    }
    workspace.git(&args);
    workspace.git(&["rev-parse", "HEAD"]).trim().to_owned()
}

const SIGNED: &str = "Signed-off-by: Fixture <fixture@example.invalid>";

#[test]
fn commits_signed_off_by_their_author_pass() {
    let workspace = Workspace::from_fixture("empty");
    let base = workspace.commit_all();
    commit(&workspace, &["Add a page", SIGNED]);
    let head = commit(&workspace, &["Fix the page", SIGNED]);

    let outcome = workspace.xtask(&["dco", "--base", &base, "--head", &head]);

    assert_eq!(outcome.findings(), vec![], "{}", outcome.stderr);
    assert_eq!(outcome.code, 0);
}

#[test]
fn commit_without_a_sign_off_fails() {
    let workspace = Workspace::from_fixture("empty");
    let base = workspace.commit_all();
    commit(&workspace, &["Add a page", SIGNED]);
    let unsigned = commit(&workspace, &["Fix the page"]);
    let head = commit(&workspace, &["Fix it again", SIGNED]);

    let outcome = workspace.xtask(&["dco", "--base", &base, "--head", &head]);

    assert_eq!(
        outcome.findings(),
        vec![finding("missing-sign-off", &unsigned)]
    );
    assert_eq!(outcome.code, 1);
}

#[test]
fn sign_off_by_someone_else_than_the_author_fails() {
    let workspace = Workspace::from_fixture("empty");
    let base = workspace.commit_all();
    let head = commit(
        &workspace,
        &[
            "Add a page",
            "Signed-off-by: Someone Else <someone@example.invalid>",
        ],
    );

    let outcome = workspace.xtask(&["dco", "--base", &base, "--head", &head]);

    assert_eq!(
        outcome.findings(),
        vec![finding("sign-off-not-author", &head)]
    );
    assert_eq!(outcome.code, 1);
}

#[test]
fn merge_commit_needs_no_sign_off() {
    let workspace = Workspace::from_fixture("empty");
    let base = workspace.commit_all();
    workspace.git(&["checkout", "--quiet", "-b", "topic"]);
    commit(&workspace, &["Add a page", SIGNED]);
    workspace.git(&["checkout", "--quiet", "-"]);
    commit(&workspace, &["Add another page", SIGNED]);
    workspace.git(&[
        "merge",
        "--quiet",
        "--no-ff",
        "--message",
        "Merge topic",
        "topic",
    ]);
    let head = workspace.git(&["rev-parse", "HEAD"]).trim().to_owned();

    let outcome = workspace.xtask(&["dco", "--base", &base, "--head", &head]);

    assert_eq!(outcome.findings(), vec![], "{}", outcome.stderr);
    assert_eq!(outcome.code, 0);
}

/// A Git hook runs with `GIT_DIR` naming its own repository: the check
/// still reads the workspace's commits, where it runs.
#[test]
fn commits_are_read_from_the_workspace_whatever_repository_git_dir_names() {
    let workspace = Workspace::from_fixture("empty");
    let base = workspace.commit_all();
    let head = commit(&workspace, &["Add a page", SIGNED]);
    let other = Workspace::from_fixture("empty");
    other.commit_all();
    let git_dir = other.path().join(".git");

    let outcome = workspace.xtask_with_env(
        &["dco", "--base", &base, "--head", &head],
        &[("GIT_DIR", git_dir.as_os_str())],
    );

    assert_eq!(
        (outcome.code, outcome.findings()),
        (0, vec![]),
        "{}",
        outcome.stderr
    );
}
