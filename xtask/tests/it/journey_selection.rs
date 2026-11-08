//! `xtask journeys select`: the runners' filters for the variants chosen by
//! Journey ID, surface, milestone and area (ADR 0063), from the working
//! tree's catalog or a Git tag's (ADR 0066). Each runner's filter is one
//! `<runner>\t<argument>` line, which `just journeys` hands to `just test`:
//! a nextest filterset on test paths, Vitest's tag filter, pytest's `-m` and
//! Playwright's `--grep`. Values of one flag add variants; different flags
//! intersect. Retired Journeys have no variant to select.

use rstest::rstest;

use crate::workspace::{Outcome, Workspace};

fn select(workspace: &Workspace, flags: &[&str]) -> Outcome {
    let mut arguments = vec!["journeys", "select"];
    arguments.extend_from_slice(flags);
    workspace.xtask(&arguments)
}

/// The printed `(runner, argument)` pairs.
fn filters(outcome: &Outcome) -> Vec<(String, String)> {
    outcome
        .stdout
        .lines()
        .filter_map(|line| line.split_once('\t'))
        .map(|(runner, argument)| (runner.to_owned(), argument.to_owned()))
        .collect()
}

fn pairs(expected: &[(&str, String)]) -> Vec<(String, String)> {
    expected
        .iter()
        .map(|(runner, argument)| ((*runner).to_owned(), argument.clone()))
        .collect()
}

const J001_CLI: &str = "test(/(^|::)journeys::j001::cli$/)";
const J001_FOXHOUND_CLI: &str = "test(/(^|::)journeys::j001::foxhound_cli$/)";
const J003_CLI: &str = "test(/(^|::)journeys::j003::cli$/)";
const J003_SDK_RUST: &str = "test(/(^|::)journeys::j003::sdk_rust$/)";
const J001_CONSOLE: &str = r"--grep=(?=.*\s@J-001(?:\s|$))(?=.*\s@console(?:\s|$))";
const J003_SDK_TYPESCRIPT: &str = "--tags-filter=(J-003 && sdk-typescript)";
const J003_SDK_PYTHON: &str = "journey(id=\"J-003\", surface=\"sdk-python\")";

/// The fixture catalog: J-001 is Collie's, area 3, `cli` and `console` at
/// M1, so also `foxhound-cli`; J-002 is retired; J-003 is Foxhound's, area 6,
/// `cli` at M1, the three SDKs at M2 and `adapter-langchain` at M4.
#[rstest]
#[case::id(&["--id", "J-001"], vec![
    ("nextest", format!("--filterset={J001_CLI} | {J001_FOXHOUND_CLI}")),
    ("playwright", J001_CONSOLE.to_owned()),
])]
#[case::surface(&["--surface", "cli"], vec![
    ("nextest", format!("--filterset={J001_CLI} | {J003_CLI}")),
])]
#[case::foxhound_cli(&["--surface", "foxhound-cli"], vec![
    ("nextest", format!("--filterset={J001_FOXHOUND_CLI}")),
])]
#[case::vitest_surface(&["--surface", "sdk-typescript"], vec![("vitest", J003_SDK_TYPESCRIPT.to_owned())])]
#[case::pytest_surface(&["--surface", "sdk-python"], vec![("pytest", "-m".to_owned()), ("pytest", J003_SDK_PYTHON.to_owned())])]
#[case::milestone(&["--milestone", "M2"], vec![
    ("nextest", format!("--filterset={J003_SDK_RUST}")),
    ("vitest", J003_SDK_TYPESCRIPT.to_owned()),
    ("pytest", "-m".to_owned()),
    ("pytest", J003_SDK_PYTHON.to_owned()),
])]
#[case::area(&["--area", "3"], vec![
    ("nextest", format!("--filterset={J001_CLI} | {J001_FOXHOUND_CLI}")),
    ("playwright", J001_CONSOLE.to_owned()),
])]
#[case::values_of_one_flag_add(&["--surface", "console", "--surface", "sdk-rust"], vec![
    ("nextest", format!("--filterset={J003_SDK_RUST}")),
    ("playwright", J001_CONSOLE.to_owned()),
])]
#[case::flags_intersect(&["--area", "6", "--milestone", "M1"], vec![
    ("nextest", format!("--filterset={J003_CLI}")),
])]
#[case::id_and_surface_intersect(&["--id", "J-001", "--surface", "cli"], vec![
    ("nextest", format!("--filterset={J001_CLI}")),
])]
fn each_flag_prints_the_filter_of_its_variants_per_runner(
    #[case] flags: &[&str],
    #[case] expected: Vec<(&str, String)>,
) {
    let workspace = Workspace::from_fixture("journeys");

    let outcome = select(&workspace, flags);

    assert_eq!(filters(&outcome), pairs(&expected), "{}", outcome.stderr);
    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
}

/// No runner runs `adapter-*` variants yet, so they are named on stderr and
/// left out of the filters.
#[test]
fn variant_no_runner_runs_yet_is_left_out() {
    let workspace = Workspace::from_fixture("journeys");

    let outcome = select(&workspace, &["--milestone", "M4"]);

    assert_eq!(filters(&outcome), vec![], "{}", outcome.stderr);
    assert!(
        outcome.stderr.contains("J-003@adapter-langchain"),
        "{}",
        outcome.stderr
    );
    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
}

#[rstest]
#[case::retired_journey(&["--id", "J-002"])]
#[case::unknown_journey(&["--id", "J-009"])]
#[case::empty_intersection(&["--id", "J-001", "--surface", "sdk-rust"])]
fn selection_of_no_variant_fails(#[case] flags: &[&str]) {
    let workspace = Workspace::from_fixture("journeys");

    let outcome = select(&workspace, flags);

    assert_eq!(filters(&outcome), vec![]);
    assert_eq!(outcome.code, 1, "{}", outcome.stderr);
}

#[test]
fn surface_adr_0063_does_not_list_is_refused() {
    let workspace = Workspace::from_fixture("journeys");

    let outcome = select(&workspace, &["--surface", "desktop"]);

    assert_eq!(filters(&outcome), vec![]);
    assert_eq!(outcome.code, 2, "{}", outcome.stderr);
}

/// The tag's catalog is read, not the working tree's: there J-001 still
/// lists `console`.
#[test]
fn tag_selects_from_the_tags_catalog() {
    let workspace = Workspace::from_fixture("journeys");
    workspace.commit_all();
    workspace.git(&["tag", "v0.1.0"]);
    workspace.edit("journeys/J-001.toml", "console = \"M1\"\n", "");

    let outcome = select(&workspace, &["--id", "J-001", "--tag", "v0.1.0"]);

    assert_eq!(
        filters(&outcome),
        pairs(&[
            (
                "nextest",
                format!("--filterset={J001_CLI} | {J001_FOXHOUND_CLI}")
            ),
            ("playwright", J001_CONSOLE.to_owned()),
        ]),
        "{}",
        outcome.stderr
    );
    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
}

/// The fixture's runners' listings, as `xtask journeys listings` writes them,
/// in which a nextest test carries J-001@cli alone.
fn write_listings(workspace: &Workspace) {
    workspace.write(
        "listings/nextest.json",
        br#"{ "rust-suites": { "journeyed::it": { "package-name": "journeyed", "kind": "test", "binary-name": "it", "testcases": { "journeys::j001::cli": {} } } } }"#,
    );
    workspace.write("listings/vitest.json", b"[]");
    workspace.write("listings/pytest.json", br#"{ "tests": [] }"#);
    workspace.write(
        "listings/playwright.json",
        br#"{ "config": { "rootDir": "." }, "suites": [] }"#,
    );
}

/// A printed nextest filter runs the chosen variant's test and no other,
/// as `just journeys` runs it through `just test`.
#[test]
fn a_printed_nextest_filter_runs_exactly_the_chosen_variants_test() {
    let workspace = Workspace::from_fixture("journey-listings");
    workspace.append(
        "tests/it/journeys/j001.rs",
        "\n#[test]\nfn foxhound_cli() {}\n",
    );
    workspace.append("tests/it/main.rs", "\n#[test]\nfn not_a_journey() {}\n");
    workspace.write(
        ".config/nextest.toml",
        b"[profile.default.junit]\npath = \"junit.xml\"\n",
    );
    let selected = select(&workspace, &["--surface", "cli"]);
    assert_eq!(selected.code, 0, "{}", selected.stderr);
    let filter: Vec<String> = filters(&selected)
        .into_iter()
        .filter(|(runner, _)| runner == "nextest")
        .map(|(_, argument)| argument)
        .collect();

    let output = workspace
        .command("cargo")
        .args(["nextest", "run", "--no-fail-fast"])
        .args(&filter)
        .output()
        .unwrap();

    assert!(output.status.success(), "{output:?}");
    let report = workspace.read_text("target/nextest/default/junit.xml");
    let ran: Vec<&str> = report
        .split("<testcase name=\"")
        .skip(1)
        .filter_map(|case| case.split('"').next())
        .collect();
    assert_eq!(ran, ["journeys::j001::cli"]);
}

/// With the runners' listings, each chosen variant no test carries is named,
/// and a choice none of whose variants has a test fails rather than run no
/// test and pass.
#[rstest]
#[case::some_carried(
    &["--id", "J-001"],
    0,
    &["J-001@console", "J-001@sdk-python", "J-001@sdk-typescript", "J-001@foxhound-cli"]
)]
#[case::none_carried(&["--surface", "sdk-python"], 1, &["J-001@sdk-python"])]
fn select_with_listings_names_each_chosen_variant_no_test_carries(
    #[case] flags: &[&str],
    #[case] code: i32,
    #[case] untested: &[&str],
) {
    let workspace = Workspace::from_fixture("journey-listings");
    write_listings(&workspace);

    let outcome = select(&workspace, &[flags, &["--listings", "listings"]].concat());

    assert_eq!(outcome.code, code, "{}", outcome.stderr);
    let named: Vec<&str> = outcome
        .stderr
        .lines()
        .filter_map(|line| line.strip_prefix("note[no-test] "))
        .filter_map(|line| line.split(':').next())
        .collect();
    assert_eq!(named, untested, "{}", outcome.stderr);
}
