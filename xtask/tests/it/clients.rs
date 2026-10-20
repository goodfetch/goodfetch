//! `xtask clients check` (ADR 0069): every Client conformance case of
//! `conformance/clients/` is implemented on each surface it lists, in each
//! mode that surface has, and no test carries an unknown case, each cause
//! failing on its own.

use rstest::rstest;

use crate::workspace::{Finding, Workspace, finding};

const CASES: &str = "conformance/clients/cases.toml";
const COLLIE_CASES: &str = "crates/goodfetch-collie/tests/it/client_conformance.rs";
const HELLO_REFUSED: &str = "crates/goodfetch-collie/tests/it/client_conformance/hello_refused.rs";
const ATTACHED_ENGINE_STOPS: &str =
    "crates/goodfetch-collie/tests/it/client_conformance/attached_engine_stops.rs";

#[test]
fn check_passes_on_a_clean_workspace() {
    let workspace = Workspace::from_fixture("clients");

    let outcome = workspace.xtask(&["clients", "check"]);

    assert_eq!(outcome.findings(), vec![], "{}", outcome.stdout);
    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
}

#[test]
fn check_notes_each_listed_surface_whose_tests_it_does_not_read_yet() {
    let workspace = Workspace::from_fixture("clients");

    let outcome = workspace.xtask(&["clients", "check"]);

    assert_eq!(
        outcome.notes(),
        vec![
            finding("unread-surface", "hello-refused sdk-rust"),
            finding("unread-surface", "start-a-run sdk-typescript"),
        ]
    );
}

#[rstest]
#[case::case_missing_on_a_listed_surface(
    CASES,
    "surfaces = [\"sdk-typescript\"]",
    "surfaces = [\"sdk-typescript\", \"cli\"]",
    vec![
        finding("missing", "start-a-run cli attached"),
        finding("missing", "start-a-run cli daemon"),
    ],
)]
#[case::case_missing_in_one_mode(
    HELLO_REFUSED,
    "#[case::daemon(\"daemon\")]\n",
    "",
    vec![finding("missing", "hello-refused cli daemon")],
)]
#[case::case_missing_from_the_other_products_crate(
    "crates/goodfetch-foxhound/tests/it/main.rs",
    "        #[test]\n        fn daemon() {}\n",
    "",
    vec![finding("missing", "crawl-resumes cli daemon")],
)]
#[case::test_carrying_an_unknown_case(
    COLLIE_CASES,
    "mod hello_refused;\n",
    "mod hello_refused;\n\nmod gone_case {\n    #[test]\n    fn attached() {}\n}\n",
    vec![finding(
        "unknown-case",
        "goodfetch-collie::it client_conformance::gone_case::attached",
    )],
)]
#[case::test_on_a_surface_the_case_does_not_list(
    COLLIE_CASES,
    "mod hello_refused;\n",
    "mod hello_refused;\n\nmod start_a_run {\n    #[test]\n    fn attached() {}\n}\n",
    vec![finding(
        "unlisted",
        "goodfetch-collie::it client_conformance::start_a_run::attached",
    )],
)]
#[case::test_in_a_mode_the_case_does_not_list(
    ATTACHED_ENGINE_STOPS,
    "fn attached() {}\n",
    "fn attached() {}\n\n#[test]\nfn daemon() {}\n",
    vec![finding(
        "unlisted",
        "goodfetch-collie::it client_conformance::attached_engine_stops::daemon",
    )],
)]
#[case::test_carrying_no_mode(
    ATTACHED_ENGINE_STOPS,
    "fn attached() {}\n",
    "fn attached() {}\n\n#[test]\nfn stops_with_its_client() {}\n",
    vec![finding(
        "no-mode",
        "goodfetch-collie::it client_conformance::attached_engine_stops::stops_with_its_client",
    )],
)]
#[case::case_whose_modes_are_on_no_surface_it_lists(
    CASES,
    "modes = [\"attached\"]",
    "modes = [\"linked\"]",
    vec![
        finding("no-mode-on-surface", "attached-engine-stops cli"),
        finding(
            "unlisted",
            "goodfetch-collie::it client_conformance::attached_engine_stops::attached",
        ),
    ],
)]
#[case::two_cases_with_one_id(
    CASES,
    "id = \"start-a-run\"",
    "id = \"hello-refused\"",
    vec![finding("duplicate-case", "hello-refused")],
)]
#[case::case_list_naming_an_unknown_surface(
    CASES,
    "surfaces = [\"sdk-typescript\"]",
    "surfaces = [\"sdk-go\"]",
    vec![finding("schema", CASES)],
)]
fn check_fails_on_each_cause(
    #[case] file: &str,
    #[case] from: &str,
    #[case] to: &str,
    #[case] expected: Vec<Finding>,
) {
    let workspace = Workspace::from_fixture("clients");
    workspace.edit(file, from, to);

    let outcome = workspace.xtask(&["clients", "check"]);

    assert_eq!(outcome.findings(), expected, "{}", outcome.stderr);
    assert_eq!(outcome.code, 1);
}
