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

/// A module's unit tests may sit in its own `tests/` folder below `src/`,
/// as `src/engine/tests/run.rs` does: they are unit tests (ADR 0075).
#[test]
fn scan_reads_a_test_in_a_tests_folder_below_src_as_a_unit_test() {
    let workspace = Workspace::from_fixture("features");
    workspace.append("crates/sample/src/store/tests.rs", "mod deep;\n");
    workspace.write(
        "crates/sample/src/store/tests/deep.rs",
        b"/// Covers: story:1-2\n#[test]\nfn keeps_a_deep_value() {}\n",
    );

    let outcome = workspace.xtask(&["features", "scan", "--package", "sample"]);

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    assert!(
        outcome.stdout.lines().any(|line| line
            == "story:1-2\tunit\tnextest\tsample store::tests::deep::keeps_a_deep_value"),
        "{}",
        outcome.stdout
    );
}

/// rstest gives a case the attributes written between the case before it
/// and its own `#[case]`: a `Covers:` line there declares for that case
/// only, so each case of a table can declare the row it reaches. A line
/// above `#[rstest]` or below the last case declares for every case.
#[test]
fn scan_reads_a_covers_line_between_rstest_cases_for_the_case_below_it_only() {
    let workspace = Workspace::from_fixture("features");
    workspace.append("crates/sample/src/store/tests.rs", "mod table;\n");
    workspace.write(
        "crates/sample/src/store/tests/table.rs",
        b"/// Covers: story:1-1\n\
          #[rstest::rstest]\n\
          /// Covers: story:1-2\n\
          #[case::first(1)]\n\
          #[case::second(2)]\n\
          /// Covers: operation:sample.echo\n\
          #[case::third(3)]\n\
          /// Covers: operation:sample.crawl\n\
          fn counts(#[case] number: u8) {\n    let _ = number;\n}\n",
    );

    let outcome = workspace.xtask(&["features", "scan", "--package", "sample"]);

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    assert_eq!(
        outcome
            .stdout
            .lines()
            .filter(|line| line.contains("store::tests::table::") && !line.starts_with("event:"))
            .collect::<Vec<_>>(),
        vec![
            "operation:sample.crawl\tunit\tnextest\tsample store::tests::table::counts::case_1_first",
            "operation:sample.crawl\tunit\tnextest\tsample store::tests::table::counts::case_2_second",
            "operation:sample.crawl\tunit\tnextest\tsample store::tests::table::counts::case_3_third",
            "operation:sample.echo\tunit\tnextest\tsample store::tests::table::counts::case_3_third",
            "story:1-1\tunit\tnextest\tsample store::tests::table::counts::case_1_first",
            "story:1-1\tunit\tnextest\tsample store::tests::table::counts::case_2_second",
            "story:1-1\tunit\tnextest\tsample store::tests::table::counts::case_3_third",
            "story:1-2\tunit\tnextest\tsample store::tests::table::counts::case_1_first",
        ]
    );
}

/// A module whose file a `#[path]` attribute names is read from that file,
/// relative to the declaring file's folder, and its own child modules from
/// the named file's folder, as rustc reads them.
#[test]
fn scan_reads_a_module_from_the_file_its_path_attribute_names() {
    let workspace = Workspace::from_fixture("features");
    workspace.append(
        "crates/sample/src/store/tests.rs",
        "#[path = \"elsewhere/away.rs\"]\nmod away;\n",
    );
    workspace.write(
        "crates/sample/src/store/elsewhere/away.rs",
        b"/// Covers: story:1-2\n#[test]\nfn keeps_away() {}\n\nmod inner;\n",
    );
    workspace.write(
        "crates/sample/src/store/elsewhere/inner.rs",
        b"/// Covers: story:1-2\n#[test]\nfn keeps_inside() {}\n",
    );

    let outcome = workspace.xtask(&["features", "scan", "--package", "sample"]);

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    let names: Vec<&str> = outcome
        .stdout
        .lines()
        .filter_map(|line| line.strip_prefix("story:1-2\tunit\tnextest\tsample store::tests::"))
        .collect();
    assert_eq!(names, vec!["away::inner::keeps_inside", "away::keeps_away"]);
}

/// A module with no file fails the scan, rather than leaving its tests'
/// declarations unread; one behind a `cfg` may lack its file on this
/// platform, and declares nothing.
#[rstest]
#[case::no_file("mod missing;\n", 2)]
#[case::behind_a_cfg("#[cfg(target_os = \"none\")]\nmod missing;\n", 0)]
fn scan_fails_on_a_module_with_no_file_unless_a_cfg_hides_it(
    #[case] declaration: &str,
    #[case] code: i32,
) {
    let workspace = Workspace::from_fixture("features");
    workspace.append("crates/sample/src/store/tests.rs", declaration);

    let outcome = workspace.xtask(&["features", "scan", "--package", "sample"]);

    assert_eq!(outcome.code, code, "{}", outcome.stderr);
    if code != 0 {
        assert!(outcome.stderr.contains("missing"), "{}", outcome.stderr);
    }
}

/// rstest names a `#[values]` test's cases by argument, value number and
/// sanitized value, one module per argument in order, inside each
/// `#[case]`'s. The expected names are `cargo nextest list`'s for these
/// tests under rstest 0.27, so a declaration names the test nextest runs.
#[test]
fn scan_names_an_rstest_values_test_as_nextest_does() {
    let workspace = Workspace::from_fixture("features");
    workspace.append("crates/sample/src/store/tests.rs", "mod values;\n");
    workspace.write(
        "crates/sample/src/store/tests/values.rs",
        r#"//! Covers: story:1-2
use rstest::rstest;

#[rstest]
fn one_list(#[values("delivery.write_ids", "b")] key: &str) {}

#[rstest]
fn two_lists(
    #[values("delivery.write_ids_and_epochs", "x")] key: &str,
    #[values(Layer::OperatorPolicy, Layer::Run)] layer: Layer,
) {}

#[rstest]
#[case::first(1)]
#[case(2)]
fn cases_and_values(#[case] n: u8, #[values(-1, 10)] mut m: i32) {}

#[rstest]
fn padded(#[values(0, 1, 2, 3, 4, 5, 6, 7, 8, 9)] d: u8) {}

#[rstest]
fn described(
    #[values(
        /// forty two
        42,
        "a very long value that goes on and on and on and on past sixty-four characters",
        vec![1, 2],
        Some(  (1 ,2) ),
        'é'
    )]
    v: impl std::fmt::Debug,
) {}
"#
        .as_bytes(),
    );

    let outcome = workspace.xtask(&["features", "scan", "--package", "sample"]);

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    let names: Vec<&str> = outcome
        .stdout
        .lines()
        .filter_map(|line| line.strip_prefix("story:1-2\tunit\tnextest\tsample "))
        .collect();
    assert_eq!(
        names,
        vec![
            "store::tests::values::cases_and_values::case_1_first::m_1__1",
            "store::tests::values::cases_and_values::case_1_first::m_2_10",
            "store::tests::values::cases_and_values::case_2::m_1__1",
            "store::tests::values::cases_and_values::case_2::m_2_10",
            "store::tests::values::described::v_1_forty_two",
            "store::tests::values::described::v_2___averylongvaluethatgoesonandonandonandonpastsixty_fourcharacter",
            "store::tests::values::described::v_3_vec__1_2_",
            "store::tests::values::described::v_4_Some__1_2__",
            "store::tests::values::described::v_5___é__",
            "store::tests::values::one_list::key_1___delivery_write_ids__",
            "store::tests::values::one_list::key_2___b__",
            "store::tests::values::padded::d_01_0",
            "store::tests::values::padded::d_02_1",
            "store::tests::values::padded::d_03_2",
            "store::tests::values::padded::d_04_3",
            "store::tests::values::padded::d_05_4",
            "store::tests::values::padded::d_06_5",
            "store::tests::values::padded::d_07_6",
            "store::tests::values::padded::d_08_7",
            "store::tests::values::padded::d_09_8",
            "store::tests::values::padded::d_10_9",
            "store::tests::values::two_lists::key_1___delivery_write_ids_and_epochs__::layer_1_Layer__OperatorPolicy",
            "store::tests::values::two_lists::key_1___delivery_write_ids_and_epochs__::layer_2_Layer__Run",
            "store::tests::values::two_lists::key_2___x__::layer_1_Layer__OperatorPolicy",
            "store::tests::values::two_lists::key_2___x__::layer_2_Layer__Run",
        ]
    );
}

/// A `#[files]` argument names one case per matched file, relative to the
/// package's folder and sorted by path, without folders or dot files;
/// `#[values]` lists come first. The expected names are `cargo nextest
/// list`'s under rstest 0.27 for the same files.
#[test]
fn scan_names_an_rstest_files_test_as_nextest_does() {
    let workspace = Workspace::from_fixture("features");
    for file in [
        "crates/sample/data/a-b.txt",
        "crates/sample/data/.hidden.txt",
        "crates/sample/data/c.txt",
        "crates/sample/data/sub/inner.txt",
        "conformance/x.v1.json",
    ] {
        workspace.write(file, b"");
    }
    workspace.append("crates/sample/src/store/tests.rs", "mod files;\n");
    workspace.write(
        "crates/sample/src/store/tests/files.rs",
        br#"//! Covers: story:1-2
use rstest::rstest;

#[rstest]
fn reads(#[files("data/*")] path: PathBuf, #[values(1)] n: u8) {}

#[rstest]
#[case::one(1)]
fn shared(#[case] n: u8, #[files("../../conformance/*.json")] #[files("data/c.txt")] path: PathBuf) {}
"#,
    );

    let outcome = workspace.xtask(&["features", "scan", "--package", "sample"]);

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    let names: Vec<&str> = outcome
        .stdout
        .lines()
        .filter_map(|line| line.strip_prefix("story:1-2\tunit\tnextest\tsample "))
        .collect();
    assert_eq!(
        names,
        vec![
            "store::tests::files::reads::n_1_1::path_1_data_a_b_txt",
            "store::tests::files::reads::n_1_1::path_2_data_c_txt",
            "store::tests::files::shared::case_1_one::path_1__UP__UP_conformance_x_v1_json",
            "store::tests::files::shared::case_1_one::path_2_data_c_txt",
        ]
    );
}

/// An rstest form whose nextest names the scanner cannot derive fails the
/// scan rather than declaring under a name nextest never runs.
#[rstest]
#[case::destructured_values("#[rstest] fn pairs(#[values((1, 2))] (a, b): (u8, u8)) {}")]
#[case::values_in_the_rstest_attribute("#[rstest(n => [1, 2])] fn counts(n: u8) {}")]
#[case::files_glob_naming_a_variable(
    r#"#[rstest] fn reads(#[files("${DATA}/*.txt")] path: PathBuf) {}"#
)]
#[case::files_glob_matching_nothing(
    r#"#[rstest] fn reads(#[files("none/*.txt")] path: PathBuf) {}"#
)]
fn scan_fails_on_an_rstest_form_it_cannot_name(#[case] test: &str) {
    let workspace = Workspace::from_fixture("features");
    workspace.append("crates/sample/src/store/tests.rs", "mod odd;\n");
    workspace.write(
        "crates/sample/src/store/tests/odd.rs",
        format!("//! Covers: story:1-2\nuse rstest::rstest;\n{test}\n").as_bytes(),
    );

    let outcome = workspace.xtask(&["features", "scan", "--package", "sample"]);

    assert_eq!(outcome.code, 2, "{}", outcome.stdout);
    assert!(
        outcome.stderr.contains("store/tests/odd.rs"),
        "{}",
        outcome.stderr
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
    assert_eq!(outcome.notes(), vec![], "{}", outcome.stdout);
    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
}

/// `just features` asks the check about the changed packages alone: a
/// package's findings are those of the Features its tests declare and of
/// the generated Features its sources define, the package named or given
/// by its folder; repository-wide findings are not any package's.
#[rstest]
#[case::the_declaring_crate(&["--package", "sample"], vec![finding("unknown-feature", "operation:sample.unlisted")])]
#[case::the_declaring_crates_folder(&["--package", "crates/sample"], vec![finding("unknown-feature", "operation:sample.unlisted")])]
#[case::the_defining_crate(&["--package", "goodfetch-protocol"], vec![finding("untested", "operation:sample.crawl")])]
#[case::another_package(&["--package", "@sample/web"], vec![])]
fn check_of_some_packages_reports_their_findings_alone(
    #[case] packages: &[&str],
    #[case] expected: Vec<Finding>,
) {
    let workspace = Workspace::from_fixture("features");
    workspace.edit(
        "crates/sample/src/lib.rs",
        "/// Covers: operation:sample.echo",
        "/// Covers: operation:sample.echo, operation:sample.unlisted",
    );
    workspace.edit(
        "crates/sample/tests/unit/main.rs",
        "/// Covers: operation:sample.crawl",
        "/// Covers: story:1-1",
    );

    let outcome =
        workspace.xtask(&[&["features", "check", "--listings", LISTINGS], packages].concat());

    assert_eq!(outcome.findings(), expected, "{}", outcome.stdout);
}

#[test]
fn check_without_listings_notes_a_feature_only_a_runner_listing_declares() {
    let workspace = Workspace::from_fixture("features");

    let outcome = workspace.xtask(&["features", "check"]);

    assert_eq!(outcome.findings(), vec![], "{}", outcome.stdout);
    assert_eq!(
        outcome.notes(),
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

/// The real runners list each idiom's declaration: a Vitest tag inherited
/// from its suite, a Playwright annotation of its suite, a pytest marker.
#[test]
fn real_listings_carry_each_runners_declarations() {
    let workspace = Workspace::from_fixture("feature-listings");
    workspace.link_installed_runners();

    let listed = workspace.xtask(&["journeys", "listings", "--out", LISTINGS]);
    let outcome = workspace.xtask(&["features", "scan", "--listings", LISTINGS]);

    assert_eq!(listed.code, 0, "{}", listed.stderr);
    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    assert_eq!(
        outcome.stdout.lines().collect::<Vec<_>>(),
        vec![
            "event:sample.echoed\tintegration\tplaywright\tweb/tests/crawl.spec.ts › Crawl › crawls",
            "operation:sample.echo\tintegration\tvitest\techo > repeats the text",
            "story:1-1\tintegration\tnextest\tdeclaring::it declares",
            "story:1-2\tintegration\tpytest\tpython/tests/integration/test_echo.py::test_echo",
        ]
    );
}

/// The Vitest configuration declares the inventory's Feature IDs as tags,
/// so a tag naming another Feature fails the listing.
#[test]
fn vitest_tag_the_inventory_does_not_list_fails_the_listing() {
    let workspace = Workspace::from_fixture("feature-listings");
    workspace.link_installed_runners();
    workspace.edit(
        "web/tests/echo.test.ts",
        "\"operation:sample.echo\"",
        "\"operation:sample.gone\"",
    );

    let listed = workspace.xtask(&["journeys", "listings", "--out", LISTINGS]);

    assert_ne!(listed.code, 0, "{}", listed.stderr);
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
#[case::story_with_an_unknown_milestone(
    STORIES,
    "milestone = \"M2 Next\"",
    "milestone = \"M5 Later\"",
    vec![finding("milestone-unknown", "story:1-2")],
)]
#[case::active_story_with_a_reason(
    STORIES,
    "journey = true\nstatus = \"active\"",
    "journey = true\nstatus = \"active\"\nreason = \"Merged into story 2.\"",
    vec![finding("active-with-reason", "story:1-1")],
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

/// A matrix row's milestone must be one the report judges, as a story's
/// must: else the Quick check passes and the report then refuses it.
#[test]
fn check_fails_on_a_row_with_an_unknown_milestone() {
    let workspace = Workspace::from_fixture("features");
    workspace.write(
        "features/rows.toml",
        b"[[row]]\nid = \"CO-09\"\nmilestone = \"M5 Later\"\n",
    );

    let outcome = workspace.xtask(&["features", "check", "--listings", LISTINGS]);

    assert_eq!(
        outcome.findings(),
        vec![finding("milestone-unknown", "row:CO-09")],
        "{}",
        outcome.stderr
    );
    assert_eq!(outcome.code, 1);
}
