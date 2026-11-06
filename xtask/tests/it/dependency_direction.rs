//! The dependency-direction check: `goodfetch-test-support` depends on no
//! Goodfetch crate and is only ever a dev-dependency (ADR 0062); the Proving
//! ground depends on no engine crate, nor on `goodfetch-conformance` or
//! `goodfetch-protocol`; `goodfetch-conformance` depends only on
//! `goodfetch-protocol` (ADR 0032, STYLE.md); `xtask` builds no Goodfetch
//! crate but `goodfetch-protocol` (ADR 0072). The fixture's members all take
//! `goodfetch-workspace-hack`, which no rule counts (ADR 0094).
//! Covers: story:1-107
//! Covers: story:1-67

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

/// Covers: story:2-42
#[rstest]
#[case::engine("proving-ground-depends-on-engine.toml")]
#[case::conformance("proving-ground-depends-on-conformance.toml")]
#[case::protocol("proving-ground-depends-on-protocol.toml")]
#[case::protocol_as_a_build_dependency("proving-ground-build-depends-on-protocol.toml")]
#[case::engine_as_a_dev_dependency("proving-ground-dev-depends-on-engine.toml")]
fn proving_ground_depending_on_a_goodfetch_crate_fails(#[case] manifest: &str) {
    let workspace = Workspace::from_fixture("dependency-direction");
    workspace.add_fixture_file(
        &format!("dependency-direction-cases/{manifest}"),
        "proving-ground/Cargo.toml",
    );

    let outcome = workspace.xtask(&["dependency-direction"]);

    assert_eq!(
        outcome.findings(),
        vec![finding(
            "proving-ground-depends-on-goodfetch",
            "goodfetch-proving-ground"
        )]
    );
    assert_eq!(outcome.code, 1);
}

#[test]
fn conformance_depending_on_more_than_the_protocol_fails() {
    let workspace = Workspace::from_fixture("dependency-direction");
    workspace.add_fixture_file(
        "dependency-direction-cases/conformance-depends-on-engine.toml",
        "crates/goodfetch-conformance/Cargo.toml",
    );

    let outcome = workspace.xtask(&["dependency-direction"]);

    assert_eq!(
        outcome.findings(),
        vec![finding(
            "conformance-depends-on-more-than-protocol",
            "goodfetch-conformance"
        )]
    );
    assert_eq!(outcome.code, 1);
}

#[rstest]
#[case::normal_dependency("xtask-depends-on-engine.toml")]
#[case::build_dependency("xtask-build-depends-on-engine.toml")]
fn xtask_building_a_goodfetch_crate_fails(#[case] manifest: &str) {
    let workspace = Workspace::from_fixture("dependency-direction");
    workspace.add_fixture_file(
        &format!("dependency-direction-cases/{manifest}"),
        "xtask/Cargo.toml",
    );

    let outcome = workspace.xtask(&["dependency-direction"]);

    assert_eq!(
        outcome.findings(),
        vec![finding("xtask-builds-a-goodfetch-crate", "xtask")]
    );
    assert_eq!(outcome.code, 1);
}
