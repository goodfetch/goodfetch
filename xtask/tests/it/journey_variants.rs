//! The tests of each Journey variant (ADR 0063, rule 4; ADR 0094), read from
//! the four runners' listings by `xtask journeys check --listings`, in the
//! Pre-merge suite. A test carries its variant where its runner selects by
//! it: nextest in its path (`journeys::j001::cli`), Vitest and Playwright as
//! the Journey's tag and the surface's, pytest as the `journey` marker's
//! `id` and `surface`. The check fails on a test of an unknown or retired
//! Journey, of a surface its Journey does not list (`foxhound-cli` only on a
//! Collie Journey that lists `cli`), on two tests of one variant, and on a
//! Journey test that carries no single variant; it reports each variant no
//! test carries.

use rstest::rstest;

use crate::workspace::{Outcome, Workspace, finding};

/// The fixture catalog with the committed schema, and the listings of its
/// runners under `listings/`, where every variant of an active Journey has
/// one test.
fn catalog() -> Workspace {
    let workspace = Workspace::from_fixture("journeys");
    add_schema(&workspace);
    workspace
}

/// Writes the Journey schema `xtask codegen` commits into the workspace.
fn add_schema(workspace: &Workspace) {
    let schema = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("journeys/journey.schema.json");
    workspace.write(
        "journeys/journey.schema.json",
        &std::fs::read(schema).unwrap(),
    );
}

fn check(workspace: &Workspace) -> Outcome {
    workspace.xtask(&["journeys", "check", "--listings", "listings"])
}

/// Each `note[untested-variant] <variant>: …` line.
fn untested(outcome: &Outcome) -> Vec<String> {
    outcome
        .stdout
        .lines()
        .filter_map(|line| line.strip_prefix("note[untested-variant] "))
        .filter_map(|rest| rest.split_once(": "))
        .map(|(variant, _)| variant.to_owned())
        .collect()
}

#[test]
fn clean_set_of_tests_passes() {
    let workspace = catalog();

    let outcome = check(&workspace);

    assert_eq!(outcome.findings(), vec![], "{}", outcome.stderr);
    assert_eq!(untested(&outcome), Vec::<String>::new());
    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
}

/// The four runners list a workspace that holds one test of each of four of
/// J-001's variants, and the check reads each listing's variant.
#[test]
fn real_listings_carry_each_runners_variants() {
    let workspace = Workspace::from_fixture("journey-listings");
    workspace.link_installed_runners();
    add_schema(&workspace);

    let listed = workspace.xtask(&["journeys", "listings", "--out", "listings"]);
    let outcome = check(&workspace);

    assert_eq!(listed.code, 0, "{}", listed.stderr);
    assert_eq!(outcome.findings(), vec![], "{}", outcome.stderr);
    assert_eq!(untested(&outcome), vec!["J-001@foxhound-cli".to_owned()]);
    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
}

const NEXTEST: &str = "listings/nextest.json";
const VITEST: &str = "listings/vitest.json";
const PLAYWRIGHT: &str = "listings/playwright.json";
const PYTEST: &str = "listings/pytest.json";

/// The fixture's Journey tests, by the subject a finding gives each.
const COLLIE_CLI: &str = "nextest goodfetch-collie::it journeys::j001::cli";
const FOXHOUND_CLI: &str = "nextest goodfetch-collie::it journeys::j001::foxhound_cli";
const VITEST_SDK: &str =
    "vitest sdks/typescript/foxhound/tests/journeys/J-003.test.ts > sdk-typescript";
const CONSOLE: &str = "playwright console/tests/journeys/J-001.spec.ts › console";
const PYTEST_SDK: &str = "pytest sdks/python/foxhound/tests/journeys/test_j003.py::test_sdk_python";

#[rstest]
#[case::nextest(
    NEXTEST,
    "journeys::j003::cli",
    "journeys::j009::cli",
    "nextest goodfetch-foxhound::it journeys::j009::cli"
)]
#[case::vitest(
    VITEST,
    "[\"J-003\", \"sdk-typescript\"]",
    "[\"J-009\", \"sdk-typescript\"]",
    VITEST_SDK
)]
#[case::playwright(
    PLAYWRIGHT,
    "[\"J-001\", \"console\"]",
    "[\"J-009\", \"console\"]",
    CONSOLE
)]
#[case::pytest(PYTEST, "\"id\": \"J-003\"", "\"id\": \"J-009\"", PYTEST_SDK)]
fn a_test_naming_an_unknown_journey_fails(
    #[case] listing: &str,
    #[case] from: &str,
    #[case] to: &str,
    #[case] subject: &str,
) {
    let workspace = catalog();
    workspace.edit(listing, from, to);

    let outcome = check(&workspace);

    assert_eq!(
        outcome.findings(),
        vec![finding("unknown-journey", subject)],
        "{}",
        outcome.stderr
    );
    assert_eq!(outcome.code, 1);
}

#[rstest]
#[case::nextest(
    NEXTEST,
    "journeys::j003::cli",
    "journeys::j002::cli",
    "nextest goodfetch-foxhound::it journeys::j002::cli"
)]
#[case::pytest(PYTEST, "\"id\": \"J-003\"", "\"id\": \"J-002\"", PYTEST_SDK)]
fn a_test_naming_a_retired_journey_fails(
    #[case] listing: &str,
    #[case] from: &str,
    #[case] to: &str,
    #[case] subject: &str,
) {
    let workspace = catalog();
    workspace.edit(listing, from, to);

    let outcome = check(&workspace);

    assert_eq!(
        outcome.findings(),
        vec![finding("retired-journey", subject)],
        "{}",
        outcome.stderr
    );
    assert_eq!(outcome.code, 1);
}

#[rstest]
#[case::nextest_in_another_binary(
    NEXTEST,
    "\"journeys::j003::cli\": {}",
    "\"journeys::j003::cli\": {},\n        \"journeys::j001::cli\": {}",
    &[COLLIE_CLI, "nextest goodfetch-foxhound::it journeys::j001::cli"],
)]
#[case::vitest_in_another_file(
    VITEST,
    "[\"J-003\", \"adapter-langchain\"]",
    "[\"J-003\", \"sdk-typescript\"]",
    &["vitest adapters/langchain/tests/journeys/J-003.test.ts > adapter-langchain", VITEST_SDK],
)]
fn two_tests_of_one_variant_fail(
    #[case] listing: &str,
    #[case] from: &str,
    #[case] to: &str,
    #[case] subjects: &[&str],
) {
    let workspace = catalog();
    workspace.edit(listing, from, to);

    let outcome = check(&workspace);

    let mut findings = outcome.findings();
    findings.sort_by(|left, right| left.subject.cmp(&right.subject));
    let expected: Vec<_> = subjects
        .iter()
        .map(|subject| finding("duplicate-variant", subject))
        .collect();
    assert_eq!(findings, expected, "{}", outcome.stderr);
    assert_eq!(outcome.code, 1);
}

#[test]
fn foxhound_cli_test_of_a_foxhound_journey_fails() {
    let workspace = catalog();
    workspace.edit(
        NEXTEST,
        "journeys::j003::cli",
        "journeys::j003::foxhound_cli",
    );

    let outcome = check(&workspace);

    assert_eq!(
        outcome.findings(),
        vec![finding(
            "unlisted-variant",
            "nextest goodfetch-foxhound::it journeys::j003::foxhound_cli"
        )],
        "{}",
        outcome.stderr
    );
    assert_eq!(outcome.code, 1);
}

/// J-001 is Collie's; without `cli` it has no `foxhound-cli` variant either.
#[test]
fn foxhound_cli_test_of_a_collie_journey_without_cli_fails() {
    let workspace = catalog();
    workspace.edit("journeys/J-001.toml", "cli = \"M1\"\n", "");

    let outcome = check(&workspace);

    assert_eq!(
        outcome.findings(),
        vec![
            finding("unlisted-variant", COLLIE_CLI),
            finding("unlisted-variant", FOXHOUND_CLI),
        ],
        "{}",
        outcome.stderr
    );
    assert_eq!(outcome.code, 1);
}

#[test]
fn a_test_of_a_surface_its_journey_does_not_list_fails() {
    let workspace = catalog();
    workspace.edit(
        PLAYWRIGHT,
        "[\"J-001\", \"console\"]",
        "[\"J-001\", \"sdk-python\"]",
    );

    let outcome = check(&workspace);

    assert_eq!(
        outcome.findings(),
        vec![finding("unlisted-variant", CONSOLE)],
        "{}",
        outcome.stderr
    );
    assert_eq!(outcome.code, 1);
}

#[rstest]
#[case::nextest_path_past_the_surface(
    NEXTEST,
    "journeys::j001::cli\"",
    "journeys::j001::cli::starts\"",
    "nextest goodfetch-collie::it journeys::j001::cli::starts"
)]
#[case::vitest_two_surfaces(
    VITEST,
    "[\"J-003\", \"sdk-typescript\"]",
    "[\"J-003\", \"sdk-typescript\", \"sdk-python\"]",
    VITEST_SDK
)]
#[case::playwright_no_surface(PLAYWRIGHT, "[\"J-001\", \"console\"]", "[\"J-001\"]", CONSOLE)]
/// `journey("J-003")` lists no keyword.
#[case::pytest_positional_id(
    PYTEST,
    "[{ \"id\": \"J-003\", \"surface\": \"sdk-python\" }]",
    "[{}]",
    PYTEST_SDK
)]
fn journey_test_carrying_no_single_variant_fails(
    #[case] listing: &str,
    #[case] from: &str,
    #[case] to: &str,
    #[case] subject: &str,
) {
    let workspace = catalog();
    workspace.edit(listing, from, to);

    let outcome = check(&workspace);

    assert_eq!(
        outcome.findings(),
        vec![finding("malformed-variant", subject)],
        "{}",
        outcome.stderr
    );
    assert_eq!(outcome.code, 1);
}

/// Playwright reports a test of test support's `journey` with its surface's
/// tag twice: the test's own `@console`, and the one it reads from its
/// group's title, `J-001@console`. A tag carried twice is still one tag.
#[test]
fn playwright_test_carrying_its_surface_tag_twice_carries_one_variant() {
    let workspace = catalog();
    workspace.edit(
        PLAYWRIGHT,
        "[\"J-001\", \"console\"]",
        "[\"console\", \"J-001\", \"console\"]",
    );

    let outcome = check(&workspace);

    assert_eq!(outcome.findings(), vec![], "{}", outcome.stderr);
    assert_eq!(untested(&outcome), Vec::<String>::new());
    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
}

#[test]
fn variant_no_test_carries_is_reported_and_passes() {
    let workspace = catalog();
    workspace.edit(PLAYWRIGHT, "[\"J-001\", \"console\"]", "[]");

    let outcome = check(&workspace);

    assert_eq!(outcome.findings(), vec![], "{}", outcome.stderr);
    assert_eq!(untested(&outcome), vec!["J-001@console".to_owned()]);
    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
}
