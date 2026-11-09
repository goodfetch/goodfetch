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

/// `ci/perf.sh`, `ci/linux.sh` and `ci/vm/in-container.sh` run in a container
/// whose workspace another user owns, and name it `safe.directory` through
/// the environment, which is the only place Git reads it from there beside
/// the command line. Git hands on `git -c` the second way shown here.
#[rstest::rstest]
#[case::count_and_key(&[
    ("GIT_CONFIG_COUNT", "1"),
    ("GIT_CONFIG_KEY_0", "safe.directory"),
    ("GIT_CONFIG_VALUE_0", "*"),
])]
#[case::command_line_parameters(&[("GIT_CONFIG_PARAMETERS", "'safe.directory=*'")])]
fn a_workspace_another_user_owns_is_read_when_the_environment_trusts_it(
    #[case] trust: &[(&str, &str)],
) {
    let workspace = Workspace::from_fixture("empty");
    let base = workspace.commit_all();
    let head = commit(&workspace, &["Add a page", SIGNED]);
    let mut variables = vec![("GIT_TEST_ASSUME_DIFFERENT_OWNER", std::ffi::OsStr::new("1"))];
    variables.extend(
        trust
            .iter()
            .map(|(name, value)| (*name, std::ffi::OsStr::new(value))),
    );

    let outcome = workspace.xtask_with_env(&["dco", "--base", &base, "--head", &head], &variables);

    assert_eq!(
        (outcome.code, outcome.findings()),
        (0, vec![]),
        "{}",
        outcome.stderr
    );
}

/// The other half of the test above: without the trust, Git refuses the same
/// workspace, so that test proves the variables reach it.
#[test]
fn a_workspace_another_user_owns_is_refused_when_nothing_trusts_it() {
    let workspace = Workspace::from_fixture("empty");
    let base = workspace.commit_all();
    let head = commit(&workspace, &["Add a page", SIGNED]);

    let outcome = workspace.xtask_with_env(
        &["dco", "--base", &base, "--head", &head],
        &[("GIT_TEST_ASSUME_DIFFERENT_OWNER", std::ffi::OsStr::new("1"))],
    );

    assert_ne!(outcome.code, 0);
    assert!(
        outcome.stderr.contains("dubious ownership"),
        "{}",
        outcome.stderr
    );
}
