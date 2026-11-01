//! The received-requests check: a test reads the Proving ground's requests
//! only through `ReceivedRequests`, which says whether a count keeps the
//! engine's robots.txt reads and the browser's icons (ADRs 0064 and 0086).

use rstest::rstest;

use crate::workspace::{Workspace, finding};

/// A test that counts what `catalog.test` received, reading the
/// observations' requests itself.
const RAW_COUNT: &str = r#"fn received(proving_ground: &ProvingGround) -> usize {
    proving_ground.observations().unwrap()["requests"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|request| request["host"] == "catalog.test")
        .count()
}
"#;

/// The same count through the helper, beside a Run report's own
/// `requests`.
const THROUGH_THE_HELPER: &str = r#"fn received(proving_ground: &ProvingGround, report: &Value) -> (usize, Value) {
    let observations = proving_ground.observations().unwrap();
    (
        ReceivedRequests::of(&observations)
            .of_the_case()
            .filter(|request| request["host"] == "catalog.test")
            .count(),
        report["result"]["requests"].clone(),
    )
}
"#;

#[rstest]
#[case::in_a_products_integration_test("crates/goodfetch-collie/tests/it/pages.rs", RAW_COUNT, 2)]
#[case::in_a_conformance_case(
    "crates/goodfetch-conformance/src/protocol/cases/pages.rs",
    r#"fn received(site: &Site) -> usize {
    site.proving_ground
        .observations()
        .ok()
        .and_then(|observations| observations["requests"].as_array().cloned())
        .unwrap_or_default()
        .len()
}
"#,
    5
)]
#[case::through_get(
    "crates/foxhound/goodfetch-foxhound/tests/it/pages.rs",
    r#"fn received(observations: &Value) -> usize {
    observations.get("requests").and_then(Value::as_array).map_or(0, Vec::len)
}
"#,
    2
)]
fn a_read_of_the_observations_requests_outside_the_helper_fails(
    #[case] path: &str,
    #[case] source: &str,
    #[case] line: usize,
) {
    let workspace = Workspace::from_fixture("empty");
    workspace.write(path, source.as_bytes());

    let outcome = workspace.xtask(&["received-requests"]);

    assert_eq!(
        outcome.findings(),
        vec![finding("raw-requests-read", &format!("{path}:{line}"))]
    );
    assert_eq!(outcome.code, 1);
}

#[rstest]
#[case::through_the_helper("crates/goodfetch-collie/tests/it/pages.rs", THROUGH_THE_HELPER)]
#[case::the_test_support_helper(
    "crates/goodfetch-test-support/src/received_requests.rs",
    RAW_COUNT
)]
#[case::the_conformance_helper("crates/goodfetch-conformance/src/proving_ground.rs", RAW_COUNT)]
#[case::a_run_reports_requests_with_no_observations(
    "crates/foxhound/goodfetch-crawl/src/start/tests/report.rs",
    r#"fn succeeded(report: &Value) -> &Value {
    &report["requests"]["succeeded"]
}
"#
)]
#[case::the_proving_grounds_own_tests("proving-ground/tests/it/observations.rs", RAW_COUNT)]
fn a_read_through_the_helper_or_of_another_record_passes(#[case] path: &str, #[case] source: &str) {
    let workspace = Workspace::from_fixture("empty");
    workspace.write(path, source.as_bytes());

    let outcome = workspace.xtask(&["received-requests"]);

    assert_eq!(outcome.findings(), vec![]);
    assert_eq!(outcome.code, 0);
}
