//! `xtask commit-messages`: every commit a pull request adds after the
//! recorded starting point has a Conventional Commits subject, which
//! `xtask release` reads through git-cliff to write the changelog.

use rstest::rstest;

use crate::workspace::{Outcome, Workspace, finding};

/// Commits an empty change with `message` and returns the new commit.
fn commit(workspace: &Workspace, message: &str) -> String {
    workspace.git(&["commit", "--quiet", "--allow-empty", "--message", message]);
    workspace.git(&["rev-parse", "HEAD"]).trim().to_owned()
}

fn commit_messages(workspace: &Workspace, base: &str, head: &str, since: &str) -> Outcome {
    workspace.xtask(&[
        "commit-messages",
        "--base",
        base,
        "--head",
        head,
        "--since",
        since,
    ])
}

#[rstest]
#[case::type_only("fix: keep the cache when a save fails")]
#[case::scope("feat(xtask): check commit messages")]
#[case::breaking("refactor(protocol)!: rename the sample's field")]
#[case::every_other_type("docs(adr): record the owner's decision")]
fn conventional_subject_passes(#[case] message: &str) {
    let workspace = Workspace::from_fixture("empty");
    let base = workspace.commit_all();
    let head = commit(&workspace, message);

    let outcome = commit_messages(&workspace, &base, &head, &base);

    assert_eq!(outcome.findings(), vec![], "{}", outcome.stderr);
    assert_eq!(outcome.code, 0);
}

#[rstest]
#[case::no_type("Keep the cache when a save fails")]
#[case::unknown_type("fixed: keep the cache when a save fails")]
#[case::no_space_after_the_colon("fix:keep the cache")]
#[case::empty_description("fix(ci): ")]
#[case::capitalised_type("Fix: keep the cache when a save fails")]
fn unconventional_subject_fails(#[case] message: &str) {
    let workspace = Workspace::from_fixture("empty");
    let base = workspace.commit_all();
    let head = commit(&workspace, message);

    let outcome = commit_messages(&workspace, &base, &head, &base);

    assert_eq!(
        outcome.findings(),
        vec![finding("unconventional-message", &head)]
    );
    assert_eq!(outcome.code, 1);
}

/// Commits before the starting point keep their messages: history is never
/// rewritten for the rule.
#[test]
fn commit_before_the_starting_point_is_not_judged() {
    let workspace = Workspace::from_fixture("empty");
    let base = workspace.commit_all();
    let since = commit(&workspace, "Add a page");
    let head = commit(&workspace, "feat: add another page");

    let outcome = commit_messages(&workspace, &base, &head, &since);

    assert_eq!(outcome.findings(), vec![], "{}", outcome.stderr);
    assert_eq!(outcome.code, 0);
}

#[test]
fn merge_commit_is_not_judged() {
    let workspace = Workspace::from_fixture("empty");
    let base = workspace.commit_all();
    workspace.git(&["checkout", "--quiet", "-b", "topic"]);
    commit(&workspace, "feat: add a page");
    workspace.git(&["checkout", "--quiet", "-"]);
    commit(&workspace, "feat: add another page");
    workspace.git(&[
        "merge",
        "--quiet",
        "--no-ff",
        "--message",
        "Merge topic",
        "topic",
    ]);
    let head = workspace.git(&["rev-parse", "HEAD"]).trim().to_owned();

    let outcome = commit_messages(&workspace, &base, &head, &base);

    assert_eq!(outcome.findings(), vec![], "{}", outcome.stderr);
    assert_eq!(outcome.code, 0);
}

#[test]
fn unknown_starting_point_is_an_error_never_a_pass() {
    let workspace = Workspace::from_fixture("empty");
    let base = workspace.commit_all();
    let head = commit(&workspace, "Add a page");

    let outcome = commit_messages(
        &workspace,
        &base,
        &head,
        "0000000000000000000000000000000000000001",
    );

    assert_eq!(outcome.findings(), vec![]);
    assert_eq!(outcome.code, 2);
}
