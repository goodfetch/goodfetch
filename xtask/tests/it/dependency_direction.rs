//! The dependency-direction check: `goodfetch-test-support` depends on no
//! Goodfetch crate and is only ever a dev-dependency (ADR 0062).
//! Covers: story:1-107

use rstest::rstest;

use crate::workspace::{Workspace, finding};

#[test]
fn test_support_taken_only_as_a_dev_dependency_passes() {
    let workspace = Workspace::from_fixture("dependency-direction");

    let outcome = workspace.xtask(&["dependency-direction"]);

    assert_eq!(outcome.findings(), vec![]);
    assert_eq!(outcome.code, 0);
}

#[rstest]
#[case::normal_dependency("engine-normal-dependency.toml")]
#[case::build_dependency("engine-build-dependency.toml")]
fn test_support_outside_dev_dependencies_fails(#[case] manifest: &str) {
    let workspace = Workspace::from_fixture("dependency-direction");
    workspace.add_fixture_file(
        &format!("dependency-direction-cases/{manifest}"),
        "crates/goodfetch-engine/Cargo.toml",
    );

    let outcome = workspace.xtask(&["dependency-direction"]);

    assert_eq!(
        outcome.findings(),
        vec![finding("test-support-not-dev-only", "goodfetch-engine")]
    );
    assert_eq!(outcome.code, 1);
}

#[test]
fn test_support_depending_on_a_goodfetch_crate_fails() {
    let workspace = Workspace::from_fixture("dependency-direction");
    workspace.add_fixture_file(
        "dependency-direction-cases/test-support-depends-on-engine.toml",
        "crates/goodfetch-test-support/Cargo.toml",
    );

    let outcome = workspace.xtask(&["dependency-direction"]);

    assert_eq!(
        outcome.findings(),
        vec![finding(
            "test-support-depends-on-goodfetch",
            "goodfetch-test-support"
        )]
    );
    assert_eq!(outcome.code, 1);
}
