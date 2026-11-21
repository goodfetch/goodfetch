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
            finding("unread-surface", "cancel-a-run sdk-python"),
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
#[case::surface_listing_modes_of_its_own(
    CASES,
    "modes = [\"attached\"]",
    "modes = { cli = [\"daemon\"] }",
    vec![
        finding("missing", "attached-engine-stops cli daemon"),
        finding(
            "unlisted",
            "goodfetch-collie::it client_conformance::attached_engine_stops::attached",
        ),
    ],
)]
#[case::modes_of_a_surface_the_case_does_not_list(
    CASES,
    "modes = [\"attached\"]",
    "modes = { cli = [\"attached\"], sdk-rust = [\"daemon\"] }",
    vec![finding("modes-unlisted-surface", "attached-engine-stops sdk-rust")],
)]
#[case::test_behind_a_cfg_its_case_does_not_declare(
    ATTACHED_ENGINE_STOPS,
    "#[test]\nfn attached() {}\n",
    "#[cfg(unix)]\n#[test]\nfn attached() {}\n",
    vec![
        finding("gated", "goodfetch-collie::it client_conformance::attached_engine_stops::attached"),
        finding("missing", "attached-engine-stops cli attached"),
    ],
)]
#[case::module_behind_a_cfg_its_case_does_not_declare(
    COLLIE_CASES,
    "mod hello_refused;\n",
    "#[cfg(unix)]\nmod hello_refused;\n",
    vec![
        finding(
            "gated",
            "goodfetch-collie::it client_conformance::hello_refused::refuses_a_newer_major::case_1_attached",
        ),
        finding(
            "gated",
            "goodfetch-collie::it client_conformance::hello_refused::refuses_a_newer_major::case_2_daemon",
        ),
        finding("missing", "hello-refused cli attached"),
        finding("missing", "hello-refused cli daemon"),
    ],
)]
#[case::ignored_test(
    ATTACHED_ENGINE_STOPS,
    "#[test]\nfn attached() {}\n",
    "#[test]\n#[ignore = \"flaky\"]\nfn attached() {}\n",
    vec![
        finding("gated", "goodfetch-collie::it client_conformance::attached_engine_stops::attached"),
        finding("missing", "attached-engine-stops cli attached"),
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

#[test]
fn a_test_behind_the_cfg_its_case_declares_implements_it() {
    let workspace = Workspace::from_fixture("clients");
    workspace.edit(
        CASES,
        "modes = [\"attached\"]",
        "modes = [\"attached\"]\nplatforms = [\"unix\"]",
    );
    workspace.edit(
        ATTACHED_ENGINE_STOPS,
        "#[test]\nfn attached() {}\n",
        "#[cfg(unix)]\n#[test]\nfn attached() {}\n",
    );

    let outcome = workspace.xtask(&["clients", "check"]);

    assert_eq!(outcome.findings(), vec![], "{}", outcome.stdout);
    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
}

#[test]
fn each_surface_runs_in_the_modes_a_table_gives_it_and_one_left_out_in_every_mode_it_has() {
    let workspace = Workspace::from_fixture("clients");
    workspace.edit(
        CASES,
        "script = \"hello/refused.json\"",
        "script = \"hello/refused.json\"\nmodes = { sdk-rust = [\"daemon\"] }",
    );
    workspace.edit(
        CASES,
        "modes = [\"attached\"]",
        "modes = { cli = [\"attached\"] }",
    );

    let outcome = workspace.xtask(&["clients", "check"]);

    assert_eq!(outcome.findings(), vec![], "{}", outcome.stdout);
    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
}

#[test]
fn check_fails_when_the_case_list_is_missing() {
    let workspace = Workspace::from_fixture("clients");
    workspace.remove(CASES);

    let outcome = workspace.xtask(&["clients", "check"]);

    assert_eq!(outcome.findings(), vec![finding("missing-list", CASES)]);
    assert_eq!(outcome.code, 1);
}

/// Where the fixture's runner listings sit, as `xtask journeys listings`
/// writes them; the check reads the `sdk-typescript` surface's tests from
/// Vitest's and the `sdk-python` surface's from pytest's.
const LISTINGS: &str = "listings";
const VITEST: &str = "listings/vitest.json";
const PYTEST: &str = "listings/pytest.json";

#[test]
fn check_with_listings_passes_on_a_clean_workspace() {
    let workspace = Workspace::from_fixture("clients");

    let outcome = workspace.xtask(&["clients", "check", "--listings", LISTINGS]);

    assert_eq!(outcome.findings(), vec![], "{}", outcome.stdout);
    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
}

#[test]
fn check_with_listings_notes_only_the_surfaces_whose_tests_it_does_not_read_yet() {
    let workspace = Workspace::from_fixture("clients");

    let outcome = workspace.xtask(&["clients", "check", "--listings", LISTINGS]);

    assert_eq!(
        outcome.notes(),
        vec![finding("unread-surface", "hello-refused sdk-rust")]
    );
}

#[rstest]
#[case::case_listing_sdk_typescript_with_no_vitest_test(
    CASES,
    "surfaces = [\"cli\", \"sdk-rust\"]",
    "surfaces = [\"cli\", \"sdk-rust\", \"sdk-typescript\"]",
    vec![
        finding("missing", "hello-refused sdk-typescript attached"),
        finding("missing", "hello-refused sdk-typescript daemon"),
    ],
)]
#[case::case_missing_in_one_mode(
    VITEST,
    "\"tags\": [\"client_case/start-a-run\", \"daemon\"]",
    "\"tags\": [\"story:1-4\"]",
    vec![finding("missing", "start-a-run sdk-typescript daemon")],
)]
#[case::case_carried_only_by_a_test_outside_the_sdk_packages(
    VITEST,
    "\"name\": \"runs > starts a run attached\",\n    \"file\": \"sdks/typescript/collie-client/tests/runs.test.ts\",\n    \"tags\": [\"client_case/start-a-run\", \"attached\"]",
    "\"name\": \"runs > starts a run attached\",\n    \"file\": \"sdks/typescript/collie-client/tests/runs.test.ts\",\n    \"tags\": []",
    vec![finding("missing", "start-a-run sdk-typescript attached")],
)]
#[case::test_carrying_an_unknown_case(
    VITEST,
    "\"tags\": [\"story:1-4\"]",
    "\"tags\": [\"client_case/gone-case\", \"attached\"]",
    vec![finding(
        "unknown-case",
        "vitest sdks/typescript/collie-client/src/framing.test.ts > frames a message",
    )],
)]
#[case::test_on_a_surface_the_case_does_not_list(
    VITEST,
    "\"tags\": [\"story:1-4\"]",
    "\"tags\": [\"client_case/hello-refused\", \"attached\"]",
    vec![finding(
        "unlisted",
        "vitest sdks/typescript/collie-client/src/framing.test.ts > frames a message",
    )],
)]
#[case::test_carrying_no_mode(
    VITEST,
    "\"tags\": [\"story:1-4\"]",
    "\"tags\": [\"client_case/start-a-run\"]",
    vec![finding(
        "no-mode",
        "vitest sdks/typescript/collie-client/src/framing.test.ts > frames a message",
    )],
)]
#[case::skipped_test_carrying_a_case(
    VITEST,
    "\"tags\": [\"client_case/start-a-run\", \"daemon\"]",
    "\"tags\": [\"client_case/start-a-run\", \"daemon\"],\n    \"mode\": \"skip\"",
    vec![
        finding("gated", "vitest sdks/typescript/collie-client/tests/runs.test.ts > runs > starts a run on a Daemon"),
        finding("missing", "start-a-run sdk-typescript daemon"),
    ],
)]
#[case::test_carrying_two_modes(
    VITEST,
    "\"tags\": [\"story:1-4\"]",
    "\"tags\": [\"client_case/start-a-run\", \"attached\", \"daemon\"]",
    vec![finding(
        "no-mode",
        "vitest sdks/typescript/collie-client/src/framing.test.ts > frames a message",
    )],
)]
fn check_with_listings_fails_on_each_vitest_cause(
    #[case] file: &str,
    #[case] from: &str,
    #[case] to: &str,
    #[case] expected: Vec<Finding>,
) {
    let workspace = Workspace::from_fixture("clients");
    workspace.edit(file, from, to);

    let outcome = workspace.xtask(&["clients", "check", "--listings", LISTINGS]);

    assert_eq!(outcome.findings(), expected, "{}", outcome.stderr);
    assert_eq!(outcome.code, 1);
}

/// The pytest test of the fixture that carries no case.
const FRAMING: &str =
    "pytest sdks/python/collie-client/tests/test_framing.py::test_frames_a_message";

#[rstest]
#[case::case_listing_sdk_python_with_no_pytest_test(
    CASES,
    "surfaces = [\"cli\", \"sdk-rust\"]",
    "surfaces = [\"cli\", \"sdk-rust\", \"sdk-python\"]",
    vec![
        finding("missing", "hello-refused sdk-python attached"),
        finding("missing", "hello-refused sdk-python daemon"),
    ],
)]
#[case::case_missing_in_one_mode(
    PYTEST,
    "{ \"id\": \"cancel-a-run\", \"mode\": \"daemon\" }",
    "",
    vec![finding("missing", "cancel-a-run sdk-python daemon")],
)]
#[case::case_carried_only_by_a_test_outside_the_sdk_packages(
    PYTEST,
    "\"id\": \"sdks/python/collie-client/tests/test_runs.py::test_cancels_a_run[attached]\"",
    "\"id\": \"test-support/python/tests/test_runs.py::test_cancels_a_run[attached]\"",
    vec![finding("missing", "cancel-a-run sdk-python attached")],
)]
#[case::test_carrying_an_unknown_case(
    PYTEST,
    "\"client_case\": []",
    "\"client_case\": [{\"id\": \"gone-case\", \"mode\": \"attached\"}]",
    vec![finding("unknown-case", FRAMING)],
)]
#[case::test_on_a_surface_the_case_does_not_list(
    PYTEST,
    "\"client_case\": []",
    "\"client_case\": [{\"id\": \"hello-refused\", \"mode\": \"attached\"}]",
    vec![finding("unlisted", FRAMING)],
)]
#[case::test_in_a_mode_the_surface_does_not_have(
    PYTEST,
    "\"client_case\": []",
    "\"client_case\": [{\"id\": \"cancel-a-run\", \"mode\": \"linked\"}]",
    vec![finding("unlisted", FRAMING)],
)]
#[case::test_carrying_no_mode(
    PYTEST,
    "\"client_case\": []",
    "\"client_case\": [{\"id\": \"cancel-a-run\"}]",
    vec![finding("no-mode", FRAMING)],
)]
#[case::marker_naming_no_case(
    PYTEST,
    "\"client_case\": []",
    "\"client_case\": [{\"mode\": \"attached\"}]",
    vec![finding("malformed", FRAMING)],
)]
#[case::marker_with_another_keyword(
    PYTEST,
    "\"client_case\": []",
    "\"client_case\": [{\"id\": \"cancel-a-run\", \"mode\": \"attached\", \"surface\": \"sdk-python\"}]",
    vec![finding("malformed", FRAMING)],
)]
#[case::skipped_test_carrying_a_case(
    PYTEST,
    "{ \"id\": \"cancel-a-run\", \"mode\": \"daemon\" }],\n      \"skipped\": false",
    "{ \"id\": \"cancel-a-run\", \"mode\": \"daemon\" }],\n      \"skipped\": true",
    vec![
        finding("gated", "pytest sdks/python/collie-client/tests/test_runs.py::test_cancels_a_run[daemon]"),
        finding("missing", "cancel-a-run sdk-python daemon"),
    ],
)]
fn check_with_listings_fails_on_each_pytest_cause(
    #[case] file: &str,
    #[case] from: &str,
    #[case] to: &str,
    #[case] expected: Vec<Finding>,
) {
    let workspace = Workspace::from_fixture("clients");
    workspace.edit(file, from, to);

    let outcome = workspace.xtask(&["clients", "check", "--listings", LISTINGS]);

    assert_eq!(outcome.findings(), expected, "{}", outcome.stderr);
    assert_eq!(outcome.code, 1);
}
