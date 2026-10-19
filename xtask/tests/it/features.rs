//! `xtask features` (ADRs 0072 and 0075): the declarations the scanner reads
//! from Rust `Covers:` lines, with their levels and nextest names, and the
//! Feature check's causes, each failing on its own.

use rstest::rstest;

use crate::workspace::{Finding, Workspace, finding};

#[test]
fn scan_reads_each_declaration_with_its_level_and_nextest_name() {
    let workspace = Workspace::from_fixture("features");

    let outcome = workspace.xtask(&["features", "scan"]);

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    assert_eq!(
        outcome.stdout.lines().collect::<Vec<_>>(),
        vec![
            "event:sample.echoed\tunit\tsample store::tests::keeps_a_value::case_1_empty",
            "event:sample.echoed\tunit\tsample store::tests::keeps_a_value::case_2_full",
            "operation:sample.crawl\tunit\tsample::unit crawls",
            "operation:sample.echo\tunit\tsample tests::echo_repeats_the_text",
            "operation:sample.echo\tintegration\tsample::integration integrates",
            "story:1-1\tintegration\tsample::integration integrates",
            "story:1-1\tconformance\tcases::it case_passes",
            "story:1-1\tjourney\tsample::it journeys::first::reaches_the_goal",
            "story:1-2\tintegration\tsample::it store::keeps_a_value_across_processes",
            "story:1-2\tintegration\tsample::standalone runs_alone",
        ]
    );
}

#[test]
fn scan_of_one_package_prints_only_its_declarations() {
    let workspace = Workspace::from_fixture("features");

    let outcome = workspace.xtask(&["features", "scan", "--package", "cases"]);

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    assert_eq!(
        outcome.stdout,
        "story:1-1\tconformance\tcases::it case_passes\n"
    );
}

#[test]
fn check_passes_on_a_clean_workspace() {
    let workspace = Workspace::from_fixture("features");

    let outcome = workspace.xtask(&["features", "check"]);

    assert_eq!(outcome.findings(), vec![], "{}", outcome.stderr);
    assert_eq!(outcome.code, 0);
}

const STORIES: &str = "features/stories/1.toml";
const AREAS: &str = "features/areas.toml";
const EXCEPTIONS: &str = "features/exceptions.toml";
const STANDALONE: &str = "crates/sample/tests/standalone.rs";

#[rstest]
#[case::generated_feature_no_test_declares(
    "crates/sample/tests/unit/main.rs",
    "/// Covers: operation:sample.crawl\n",
    "",
    vec![finding("untested", "operation:sample.crawl")],
)]
#[case::declaration_of_an_unknown_feature(
    STANDALONE,
    "story:1-2",
    "story:1-9",
    vec![finding("unknown-feature", "story:1-9")],
)]
#[case::declaration_of_a_retired_story(
    STANDALONE,
    "story:1-2",
    "story:1-3",
    vec![finding("retired-feature", "story:1-3")],
)]
#[case::story_without_journey_missing_its_reason(
    STORIES,
    "not_journey = \"part\"\nstatus = \"active\"",
    "status = \"active\"",
    vec![finding("not-journey-missing", "story:1-2")],
)]
#[case::story_with_an_unknown_reason(
    STORIES,
    "not_journey = \"part\"\nstatus = \"active\"",
    "not_journey = \"goal\"\nstatus = \"active\"",
    vec![finding("schema", STORIES)],
)]
#[case::story_with_journey_and_a_reason(
    STORIES,
    "journey = true\n",
    "journey = true\nnot_journey = \"actor\"\n",
    vec![finding("not-journey-extra", "story:1-1")],
)]
#[case::retired_story_without_a_reason(
    STORIES,
    "reason = \"Merged into story 1.\"\n",
    "",
    vec![finding("retired-without-reason", "story:1-3")],
)]
#[case::stories_file_with_an_unknown_field(
    STORIES,
    "milestone = \"M2 Next\"",
    "milestone = \"M2 Next\"\nowner = \"area 1\"",
    vec![finding("schema", STORIES)],
)]
#[case::generated_feature_no_area_owns(
    AREAS,
    "prefix = \"sample_\"",
    "prefix = \"other_\"",
    vec![finding("no-area", "error:sample_refused")],
)]
#[case::generated_feature_two_areas_own(
    AREAS,
    "prefix = \"sample_\"",
    "prefix = \"sample_\"\n\n[[area]]\nnumber = 2\nkind = \"error\"\nprefix = \"sample\"",
    vec![finding("two-areas", "error:sample_refused")],
)]
#[case::exception_of_an_unknown_feature(
    EXCEPTIONS,
    "error:sample_refused",
    "error:sample_gone",
    vec![
        finding("exception-unknown", "error:sample_gone"),
        finding("untested", "error:sample_refused"),
    ],
)]
#[case::exception_of_a_feature_a_test_declares(
    EXCEPTIONS,
    "error:sample_refused",
    "operation:sample.crawl",
    vec![
        finding("exception-tested", "operation:sample.crawl"),
        finding("untested", "error:sample_refused"),
    ],
)]
#[case::exception_missing_its_link(
    EXCEPTIONS,
    "link = \"https://example.invalid/sample\"\n",
    "",
    vec![
        finding("schema", EXCEPTIONS),
        finding("untested", "error:sample_refused"),
    ],
)]
fn check_fails_on_each_cause(
    #[case] file: &str,
    #[case] from: &str,
    #[case] to: &str,
    #[case] expected: Vec<Finding>,
) {
    let workspace = Workspace::from_fixture("features");
    workspace.edit(file, from, to);

    let outcome = workspace.xtask(&["features", "check"]);

    assert_eq!(outcome.findings(), expected, "{}", outcome.stderr);
    assert_eq!(outcome.code, 1);
}
