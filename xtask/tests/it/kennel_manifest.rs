//! The Kennel's manifest against the root's (ADR 0141): its lints, its
//! `rust-version` and the version of each dependency both declare are the
//! root's; its own version, features and dependencies stay its own.

use rstest::rstest;

use crate::workspace::{Workspace, finding};

#[test]
fn a_kennel_manifest_keeping_the_roots_rules_passes() {
    let workspace = Workspace::from_fixture("kennel-manifest");

    let outcome = workspace.xtask(&["kennel-manifest"]);

    assert_eq!(outcome.findings(), vec![]);
    assert_eq!(outcome.code, 0);
}

#[rstest]
#[case::an_older_kennel_rust_version(
    "kennel/Cargo.toml",
    r#"rust-version = "1.97""#,
    r#"rust-version = "1.95""#,
    "rust-version",
    "workspace.package.rust-version"
)]
#[case::a_root_rust_version_the_kennel_does_not_follow(
    "Cargo.toml",
    r#"rust-version = "1.97""#,
    r#"rust-version = "1.98""#,
    "rust-version",
    "workspace.package.rust-version"
)]
#[case::another_version_of_a_shared_dependency(
    "kennel/Cargo.toml",
    r#"tokio = { version = "1.53.3""#,
    r#"tokio = { version = "1.52.0""#,
    "dependency-version",
    "workspace.dependencies.tokio"
)]
#[case::a_root_version_the_kennel_does_not_follow(
    "Cargo.toml",
    r#"toml = "1.1.8""#,
    r#"toml = "1.2.0""#,
    "dependency-version",
    "workspace.dependencies.toml"
)]
#[case::other_lints(
    "kennel/Cargo.toml",
    r#"unwrap_used = "deny""#,
    r#"unwrap_used = "warn""#,
    "lints",
    "workspace.lints"
)]
fn a_kennel_manifest_drifting_from_the_roots_fails(
    #[case] file: &str,
    #[case] from: &str,
    #[case] to: &str,
    #[case] kind: &str,
    #[case] subject: &str,
) {
    let workspace = Workspace::from_fixture("kennel-manifest");
    workspace.edit(file, from, to);

    let outcome = workspace.xtask(&["kennel-manifest"]);

    assert_eq!(outcome.findings(), vec![finding(kind, subject)]);
    assert_eq!(outcome.code, 1);
}
