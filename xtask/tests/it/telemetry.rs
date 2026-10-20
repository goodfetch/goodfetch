//! The no-telemetry check (ADR 0043): no key of the settings catalog could
//! turn telemetry on, and no lockfile of the Rust, Python or pnpm workspace
//! holds an analytics client.
//! Covers: story:4-80, row:SC-10

use rstest::rstest;

use crate::workspace::{Workspace, finding};

#[test]
fn a_catalog_and_lockfiles_without_telemetry_pass() {
    let workspace = Workspace::from_fixture("telemetry");

    let outcome = workspace.xtask(&["telemetry"]);

    assert_eq!(outcome.findings(), vec![]);
    assert_eq!(outcome.code, 0);
}

#[rstest]
#[case::a_setting_naming_telemetry(
    "catalog-with-telemetry.json",
    "crates/goodfetch-protocol/generated/catalog.json",
    "telemetry-setting",
    "telemetry.enabled"
)]
#[case::an_analytics_crate(
    "Cargo-with-sentry.lock",
    "Cargo.lock",
    "analytics-client",
    "Cargo.lock"
)]
#[case::an_analytics_python_package(
    "uv-with-posthog.lock",
    "uv.lock",
    "analytics-client",
    "uv.lock"
)]
#[case::an_analytics_npm_package(
    "pnpm-with-sentry.yaml",
    "pnpm-lock.yaml",
    "analytics-client",
    "pnpm-lock.yaml"
)]
fn telemetry_anywhere_fails(
    #[case] fixture: &str,
    #[case] replaces: &str,
    #[case] kind: &str,
    #[case] subject: &str,
) {
    let workspace = Workspace::from_fixture("telemetry");
    workspace.add_fixture_file(&format!("telemetry-cases/{fixture}"), replaces);

    let outcome = workspace.xtask(&["telemetry"]);

    assert_eq!(outcome.findings(), vec![finding(kind, subject)]);
    assert_eq!(outcome.code, 1);
}
