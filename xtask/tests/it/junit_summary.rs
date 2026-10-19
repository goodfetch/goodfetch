//! The failure summary on a job's page (ADR 0061; TESTING.md, Sharding,
//! caching and reports): `xtask junit-summary` reads the job's JUnit files
//! and writes each failed test, its failure's first lines and the seeds it
//! printed, so a contributor reads the failure without opening the logs.
//! The fixtures are JUnit files each runner wrote for a sample that fails.

use std::fmt::Write as _;

use rstest::rstest;

use crate::workspace::{Outcome, Workspace, finding};

const SUMMARY: &str = "summary.md";

/// `xtask junit-summary` on the JUnit files of `folder`.
fn summarize(workspace: &Workspace, folder: &str) -> Outcome {
    workspace.xtask(&["junit-summary", "--junit", folder, "--summary", SUMMARY])
}

#[rstest]
#[case::nextest("nextest")]
#[case::vitest("vitest")]
#[case::playwright("playwright")]
#[case::pytest("pytest")]
fn failed_tests_are_listed_with_their_first_lines_and_seeds(#[case] runner: &str) {
    let workspace = Workspace::from_fixture("junit-summary");

    let outcome = summarize(&workspace, runner);

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    insta::assert_snapshot!(runner, workspace.read_text(SUMMARY));
}

#[test]
fn passing_set_writes_that_no_test_failed() {
    let workspace = Workspace::from_fixture("junit-summary");

    let outcome = summarize(&workspace, "passing");

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    assert_eq!(
        workspace.read_text(SUMMARY),
        "### Failed tests\n\nNo test failed in 2 JUnit files.\n"
    );
}

/// The stress run (`just stress`) puts each count's JUnit files in a
/// `repeat-<count>/` folder beside the job's other files.
#[test]
fn failed_test_of_a_repeat_folder_is_listed() {
    let workspace = Workspace::from_fixture("junit-summary");
    let failing = workspace.read("vitest/vitest.xml").unwrap();
    workspace.write("passing/repeat-20/vitest.xml", &failing);

    let outcome = summarize(&workspace, "passing");

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    assert!(
        workspace
            .read_text(SUMMARY)
            .starts_with("### Failed tests\n\n1 test failed in 3 JUnit files.\n"),
        "{}",
        workspace.read_text(SUMMARY)
    );
}

#[test]
fn job_without_junit_files_says_it_wrote_none() {
    let workspace = Workspace::from_fixture("junit-summary");

    let outcome = summarize(&workspace, "target/junit");

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    assert_eq!(
        workspace.read_text(SUMMARY),
        "### Failed tests\n\nThe job wrote no JUnit file.\n"
    );
}

#[rstest]
#[case::empty("")]
#[case::truncated(
    r#"<testsuites><testsuite name="sample"><testcase name="steady" classname="sample">"#
)]
#[case::mismatched_end_tag(r#"<testsuites><testsuite name="sample"></testcase></testsuites>"#)]
#[case::not_junit("<html><body>No tests ran.</body></html>")]
#[case::undeclared_entity(
    r#"<testsuites><testsuite name="sample"><testcase name="steady"><failure>&nbsp;</failure></testcase></testsuite></testsuites>"#
)]
fn malformed_file_is_reported_and_fails_the_command(#[case] contents: &str) {
    let workspace = Workspace::from_fixture("junit-summary");
    workspace.write("nextest/vitest.xml", contents.as_bytes());

    let outcome = summarize(&workspace, "nextest");

    assert_eq!(outcome.code, 1, "{}", outcome.stderr);
    assert_eq!(
        outcome.findings(),
        vec![finding("malformed-junit", "nextest/vitest.xml")]
    );
    let summary = workspace.read_text(SUMMARY);
    let malformed = summary
        .split_once("### Malformed JUnit files\n\n")
        .map(|(_, section)| section);
    assert!(
        malformed.is_some_and(|section| section.starts_with("- `nextest/vitest.xml`: ")),
        "{summary}"
    );
}

#[test]
fn malformed_file_leaves_the_other_files_failures_listed() {
    let workspace = Workspace::from_fixture("junit-summary");
    workspace.write("nextest/vitest.xml", b"<testsuites>");

    summarize(&workspace, "nextest");

    let summary = workspace.read_text(SUMMARY);
    let headings: Vec<&str> = summary
        .lines()
        .filter(|line| line.starts_with('#'))
        .collect();
    assert_eq!(
        headings,
        [
            "### Failed tests",
            "#### `sample tests::title_is_read`",
            "### Malformed JUnit files"
        ]
    );
}

/// A nextest JUnit file of `count` failed tests, `test_0` upward.
fn failed_tests(count: usize) -> String {
    let mut cases = String::new();
    for n in 0..count {
        let _ = write!(
            cases,
            r#"<testcase name="test_{n}" classname="sample"><failure message="failed"/></testcase>"#
        );
    }
    format!(r#"<testsuites><testsuite name="sample">{cases}</testsuite></testsuites>"#)
}

#[test]
fn page_lists_at_most_a_hundred_failed_tests_and_counts_the_rest() {
    let workspace = Workspace::from_fixture("junit-summary");
    workspace.write("many/nextest-ci.xml", failed_tests(103).as_bytes());

    let outcome = summarize(&workspace, "many");

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    let summary = workspace.read_text(SUMMARY);
    let listed = summary
        .lines()
        .filter(|line| line.starts_with("#### "))
        .count();
    assert_eq!(listed, 100);
    assert_eq!(
        summary.lines().last(),
        Some("3 more failed tests are in the JUnit files the job uploads.")
    );
}
