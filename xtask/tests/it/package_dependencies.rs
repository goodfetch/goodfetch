//! The declared-dependency check of the npm and PyPI packages under `sdks/`,
//! `adapters/` and `integrations/` (ADR 0095): Foxhound's depend on no Collie
//! package but the Collie client package, Collie's on no Foxhound package,
//! and the client package on neither Product's package.
//! Covers: story:1-120, story:11-1012

use rstest::rstest;

use crate::workspace::{Workspace, finding};

#[test]
fn packages_within_their_products_pass() {
    let workspace = Workspace::from_fixture("package-dependencies");

    let outcome = workspace.xtask(&["package-dependencies"]);

    assert_eq!(outcome.findings(), vec![], "{}", outcome.stderr);
    assert_eq!(outcome.code, 0);
}

#[rstest]
#[case::foxhound_npm_package_on_collies(
    "foxhound-npm-on-collie.json",
    "sdks/typescript/foxhound/package.json",
    "foxhound-depends-on-collie",
    "@goodfetch/foxhound"
)]
#[case::foxhound_pypi_package_on_collies(
    "foxhound-pypi-on-collie.toml",
    "sdks/python/foxhound/pyproject.toml",
    "foxhound-depends-on-collie",
    "goodfetch-foxhound"
)]
#[case::foxhound_integration_extra_on_collies(
    "foxhound-integration-extra-on-collie.toml",
    "integrations/foxhound/airflow/pyproject.toml",
    "foxhound-depends-on-collie",
    "goodfetch-foxhound-airflow"
)]
#[case::collie_adapter_on_foxhounds(
    "collie-adapter-on-foxhound.json",
    "adapters/collie/langchain/package.json",
    "collie-depends-on-foxhound",
    "@goodfetch/collie-langchain"
)]
#[case::client_npm_package_on_a_collie_platform_package(
    "client-npm-on-collie-platform.json",
    "sdks/typescript/collie-client/package.json",
    "client-depends-on-product",
    "@goodfetch/collie-client"
)]
#[case::client_pypi_package_on_foxhounds(
    "client-pypi-on-foxhound.toml",
    "sdks/python/collie-client/pyproject.toml",
    "client-depends-on-product",
    "goodfetch-collie-client"
)]
fn package_depending_across_products_fails(
    #[case] manifest: &str,
    #[case] relative: &str,
    #[case] kind: &str,
    #[case] package: &str,
) {
    let workspace = Workspace::from_fixture("package-dependencies");
    workspace.add_fixture_file(&format!("package-dependencies-cases/{manifest}"), relative);

    let outcome = workspace.xtask(&["package-dependencies"]);

    assert_eq!(
        outcome.findings(),
        vec![finding(kind, package)],
        "{}",
        outcome.stderr
    );
    assert_eq!(outcome.code, 1);
}

#[test]
fn package_of_neither_product_fails() {
    let workspace = Workspace::from_fixture("package-dependencies");
    workspace.add_fixture_file(
        "package-dependencies-cases/unplaced.json",
        "sdks/typescript/shared/package.json",
    );

    let outcome = workspace.xtask(&["package-dependencies"]);

    assert_eq!(
        outcome.findings(),
        vec![finding(
            "package-of-no-product",
            "sdks/typescript/shared/package.json"
        )]
    );
    assert_eq!(outcome.code, 1);
}

#[test]
fn the_smoke_scripts_package_is_of_no_product_and_passes() {
    let workspace = Workspace::from_fixture("package-dependencies");
    workspace.add_fixture_file(
        "package-dependencies-cases/smoke.json",
        "sdks/typescript/smoke/package.json",
    );

    let outcome = workspace.xtask(&["package-dependencies"]);

    assert_eq!(outcome.findings(), vec![], "{}", outcome.stderr);
    assert_eq!(outcome.code, 0);
}
