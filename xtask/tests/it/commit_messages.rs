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

/// Writes the committed exceptions file with one entry per `(commit, reason)`.
fn write_exceptions(workspace: &Workspace, entries: &[(&str, &str)]) {
    let text = entries
        .iter()
        .map(|(commit, reason)| {
            format!("[[exception]]\ncommit = \"{commit}\"\nreason = \"{reason}\"\n")
        })
        .collect::<Vec<_>>()
        .join("\n");
    workspace.write("ci/commit-messages.toml", text.as_bytes());
}

const REASON: &str = "merged into other branches before the check ran";

/// A commit the exceptions file lists by its full hash keeps its message, and
/// only that commit: the next one is still judged.
#[test]
fn listed_commit_is_not_judged() {
    let workspace = Workspace::from_fixture("empty");
    let base = workspace.commit_all();
    let listed = commit(&workspace, "wip: raw observations");
    let head = commit(&workspace, "Add a page");
    write_exceptions(&workspace, &[(&listed, REASON)]);

    let outcome = commit_messages(&workspace, &base, &head, &base);

    assert_eq!(
        outcome.findings(),
        vec![finding("unconventional-message", &head)],
        "{}",
        outcome.stderr
    );
    assert_eq!(outcome.code, 1);
}

/// An entry may outlive its commit's range: a later pull request's range does
/// not hold it, and that is no error.
#[test]
fn entry_outside_the_range_passes() {
    let workspace = Workspace::from_fixture("empty");
    workspace.commit_all();
    let earlier = commit(&workspace, "wip: raw observations");
    let base = earlier.clone();
    let head = commit(&workspace, "feat: add a page");
    write_exceptions(&workspace, &[(&earlier, REASON)]);

    let outcome = commit_messages(&workspace, &base, &head, &base);

    assert_eq!(outcome.findings(), vec![], "{}", outcome.stderr);
    assert_eq!(outcome.code, 0);
}

/// Each entry names one commit by its full hash and says why in one line;
/// anything else is refused, never read as a pattern or a pass.
#[rstest]
#[case::short_hash("c3c56422", Some(REASON))]
#[case::not_hexadecimal("zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz", Some(REASON))]
#[case::upper_case("C3C564220000000000000000000000000000000A", Some(REASON))]
#[case::no_reason("c3c5642200000000000000000000000000000000", None)]
#[case::empty_reason("c3c5642200000000000000000000000000000000", Some(" "))]
#[case::reason_of_two_lines(
    "c3c5642200000000000000000000000000000000",
    Some("merged early\\nand kept")
)]
fn invalid_entry_is_an_error(#[case] commit_value: &str, #[case] reason: Option<&str>) {
    let workspace = Workspace::from_fixture("empty");
    let base = workspace.commit_all();
    let head = commit(&workspace, "feat: add a page");
    let reason_line = reason.map_or(String::new(), |reason| format!("reason = \"{reason}\"\n"));
    workspace.write(
        "ci/commit-messages.toml",
        format!("[[exception]]\ncommit = \"{commit_value}\"\n{reason_line}").as_bytes(),
    );

    let outcome = commit_messages(&workspace, &base, &head, &base);

    assert_eq!(
        outcome.findings(),
        vec![finding("invalid-exception", commit_value)],
        "{}",
        outcome.stderr
    );
    assert_eq!(outcome.code, 1);
}

/// `commit-messages` over `base..head`, from `base`, recording into the
/// gate's records `records`.
fn commit_messages_recorded(workspace: &Workspace, base: &str, head: &str) -> Outcome {
    workspace.xtask(&[
        "commit-messages",
        "--base",
        base,
        "--head",
        head,
        "--since",
        base,
        "--records",
        "records",
    ])
}

/// A commit's subject is part of the commit, so a green verdict holds for the
/// next layer on the same rules (ADR 0135's amendment of 2026-11-22).
#[test]
fn a_commit_judged_green_before_is_not_judged_again() {
    let workspace = Workspace::from_fixture("empty");
    let base = workspace.commit_all();
    let pushed = commit(&workspace, "feat: add a page");
    commit_messages_recorded(&workspace, &base, &pushed);
    let head = commit(&workspace, "fix: mend the page");

    let outcome = commit_messages_recorded(&workspace, &base, &head);

    assert_eq!(
        (outcome.code, outcome.findings(), outcome.notes()),
        (0, vec![], vec![finding("judged-green-before", &pushed)]),
        "{}",
        outcome.stderr
    );
}

/// The exceptions are part of the rules a verdict was reached on: once an
/// entry leaves, the commit it kept is judged again, and fails.
#[test]
fn a_verdict_reached_under_other_exceptions_is_judged_again() {
    let workspace = Workspace::from_fixture("empty");
    let base = workspace.commit_all();
    let listed = commit(&workspace, "wip: raw observations");
    write_exceptions(&workspace, &[(&listed, REASON)]);
    let before = commit_messages_recorded(&workspace, &base, &listed);
    write_exceptions(&workspace, &[]);

    let outcome = commit_messages_recorded(&workspace, &base, &listed);

    assert_eq!(before.code, 0, "{}", before.stderr);
    assert_eq!(
        (outcome.code, outcome.findings(), outcome.notes()),
        (1, vec![finding("unconventional-message", &listed)], vec![]),
        "{}",
        outcome.stderr
    );
}
