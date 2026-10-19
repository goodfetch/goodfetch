//! `xtask features` (ADRs 0072 and 0075): the declarations the scanner reads
//! in each runner's idiom (Rust `Covers:` lines, Vitest tags, Playwright
//! annotations and pytest markers from the runners' listings, and the
//! `covers` lists of Journeys and conformance cases), with their levels and
//! tests, and the Feature check's causes, each failing on its own.

use std::collections::BTreeSet;

use rstest::rstest;

use crate::workspace::{Finding, Workspace, finding};

/// Where the fixture's runner listings sit, as `xtask journeys listings`
/// writes them.
const LISTINGS: &str = "listings";

#[test]
fn scan_reads_each_declaration_with_its_level_and_test() {
    let workspace = Workspace::from_fixture("features");

    let outcome = workspace.xtask(&["features", "scan", "--listings", LISTINGS]);

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    assert_eq!(
        outcome.stdout.lines().collect::<Vec<_>>(),
        vec![
            "event:sample.crawled\tintegration\tplaywright\tpackages/web/tests/crawl.spec.ts › Crawl › crawls the site",
            "event:sample.echoed\tunit\tnextest\tsample store::tests::keeps_a_value::case_1_empty",
            "event:sample.echoed\tunit\tnextest\tsample store::tests::keeps_a_value::case_2_full",
            "event:sample.echoed\tunit\tpytest\tpython/sample/tests/unit/test_store.py::test_keeps_a_value[full]",
            "event:sample.echoed\tconformance\tconformance\tclients start-a-run",
            "operation:sample.crawl\tunit\tnextest\tsample::unit crawls",
            "operation:sample.echo\tunit\tnextest\tsample tests::echo_repeats_the_text",
            "operation:sample.echo\tunit\tvitest\techo > repeats the text",
            "operation:sample.echo\tintegration\tnextest\tsample::integration integrates",
            "operation:sample.echo\tjourney\tjourney\tJ-001",
            "story:1-1\tintegration\tnextest\tsample::integration integrates",
            "story:1-1\tintegration\tplaywright\tpackages/web/tests/crawl.spec.ts › Crawl › shows the page",
            "story:1-1\tconformance\tnextest\tcases::it case_passes",
            "story:1-1\tjourney\tnextest\tsample::it journeys::first::reaches_the_goal",
            "story:1-1\tjourney\tvitest\tsdk-typescript",
            "story:1-1\tjourney\tjourney\tJ-001",
            "story:1-2\tintegration\tnextest\tsample::it store::keeps_a_value_across_processes",
            "story:1-2\tintegration\tnextest\tsample::standalone runs_alone",
            "story:1-2\tintegration\tpytest\tpython/sample/tests/integration/test_echo.py::test_echo_repeats",
            "story:1-2\tconformance\tconformance\tself-test echo-repeats",
        ]
    );
}

#[test]
fn scan_without_listings_reads_rust_journeys_and_conformance_cases() {
    let workspace = Workspace::from_fixture("features");

    let outcome = workspace.xtask(&["features", "scan"]);

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    let sources: BTreeSet<&str> = outcome
        .stdout
        .lines()
        .filter_map(|line| line.split('\t').nth(2))
        .collect();
    assert_eq!(
        sources,
        BTreeSet::from(["conformance", "journey", "nextest"])
    );
}

#[rstest]
#[case::crate_("cases", vec!["story:1-1\tconformance\tnextest\tcases::it case_passes"])]
#[case::npm_package(
    "@sample/web",
    vec![
        "event:sample.crawled\tintegration\tplaywright\tpackages/web/tests/crawl.spec.ts › Crawl › crawls the site",
        "operation:sample.echo\tunit\tvitest\techo > repeats the text",
        "story:1-1\tintegration\tplaywright\tpackages/web/tests/crawl.spec.ts › Crawl › shows the page",
        "story:1-1\tjourney\tvitest\tsdk-typescript",
    ],
)]
#[case::python_package(
    "sample-py",
    vec![
        "event:sample.echoed\tunit\tpytest\tpython/sample/tests/unit/test_store.py::test_keeps_a_value[full]",
        "story:1-2\tintegration\tpytest\tpython/sample/tests/integration/test_echo.py::test_echo_repeats",
    ],
)]
fn scan_of_one_package_prints_only_its_declarations(
    #[case] package: &str,
    #[case] expected: Vec<&str>,
) {
    let workspace = Workspace::from_fixture("features");

    let outcome = workspace.xtask(&[
        "features",
        "scan",
        "--listings",
        LISTINGS,
        "--package",
        package,
    ]);

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    assert_eq!(outcome.stdout.lines().collect::<Vec<_>>(), expected);
}

#[test]
fn check_passes_on_a_clean_workspace() {
    let workspace = Workspace::from_fixture("features");

    let outcome = workspace.xtask(&["features", "check", "--listings", LISTINGS]);

    assert_eq!(outcome.findings(), vec![], "{}", outcome.stdout);
    assert_eq!(notes(&outcome.stdout), vec![], "{}", outcome.stdout);
    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
}

#[test]
fn check_without_listings_notes_a_feature_only_a_runner_listing_declares() {
    let workspace = Workspace::from_fixture("features");

    let outcome = workspace.xtask(&["features", "check"]);

    assert_eq!(outcome.findings(), vec![], "{}", outcome.stdout);
    assert_eq!(
        notes(&outcome.stdout),
        vec![finding("untested", "event:sample.crawled")]
    );
    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
}

#[test]
fn check_with_listings_fails_on_a_feature_no_runner_declares() {
    let workspace = Workspace::from_fixture("features");
    workspace.write(
        "listings/playwright.json",
        br#"{ "config": { "rootDir": "." }, "suites": [] }"#,
    );

    let outcome = workspace.xtask(&["features", "check", "--listings", LISTINGS]);

    assert_eq!(
        outcome.findings(),
        vec![finding("untested", "event:sample.crawled")],
        "{}",
        outcome.stderr
    );
    assert_eq!(outcome.code, 1);
}

/// The `note[<kind>] <subject>: …` lines of a check's output.
fn notes(stdout: &str) -> Vec<Finding> {
    stdout
        .lines()
        .filter_map(|line| {
            let rest = line.strip_prefix("note[")?;
            let (kind, rest) = rest.split_once("] ")?;
            let (subject, _message) = rest.split_once(": ")?;
            Some(finding(kind, subject))
        })
        .collect()
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
#[case::vitest_tag_of_an_unknown_feature(
    "listings/vitest.json",
    "\"operation:sample.echo\"",
    "\"operation:sample.gone\"",
    vec![finding("unknown-feature", "operation:sample.gone")],
)]
#[case::playwright_annotation_of_an_unknown_feature(
    "listings/playwright.json",
    "\"story:1-1\"",
    "\"story:1-9\"",
    vec![finding("unknown-feature", "story:1-9")],
)]
#[case::pytest_marker_of_an_unknown_feature(
    "listings/pytest.json",
    "\"story:1-2\"",
    "\"story:1-9\"",
    vec![finding("unknown-feature", "story:1-9")],
)]
#[case::journey_covering_an_unknown_feature(
    "journeys/J-001.toml",
    "\"story:1-1\"",
    "\"story:1-9\"",
    vec![finding("unknown-feature", "story:1-9")],
)]
#[case::conformance_case_covering_an_unknown_feature(
    "conformance/self-test/cases.toml",
    "\"story:1-2\"",
    "\"story:1-9\"",
    vec![finding("unknown-feature", "story:1-9")],
)]
#[case::client_conformance_case_covering_an_unknown_feature(
    "conformance/clients/cases.toml",
    "\"event:sample.echoed\"",
    "\"event:sample.gone\"",
    vec![finding("unknown-feature", "event:sample.gone")],
)]
#[case::active_journey_covering_a_retired_story(
    "journeys/J-002.toml",
    "status = \"retired\"",
    "status = \"active\"",
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

    let outcome = workspace.xtask(&["features", "check", "--listings", LISTINGS]);

    assert_eq!(outcome.findings(), expected, "{}", outcome.stderr);
    assert_eq!(outcome.code, 1);
}
