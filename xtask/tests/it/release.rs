//! `xtask release prepare`: one Product version in every released manifest
//! of both Products, in each registry's form, with the internal crates kept
//! at `0.0.x` (ADRs 0030, 0032 and 0090).
//! Covers: story:1-5

use rstest::rstest;

use crate::workspace::Workspace;

/// The fixture's manifests and lockfiles a release versions; the
/// workspace's `Cargo.toml` pins the Collie client crate both SDK crates
/// take (ADR 0095).
const RELEASED: [&str; 12] = [
    "Cargo.lock",
    "Cargo.toml",
    "crates/foxhound/goodfetch-foxhound/Cargo.toml",
    "crates/goodfetch-collie-client/Cargo.toml",
    "crates/goodfetch-collie/Cargo.toml",
    "sdks/python/collie-client/pyproject.toml",
    "sdks/python/collie/pyproject.toml",
    "sdks/python/foxhound/pyproject.toml",
    "sdks/typescript/collie-client/package.json",
    "sdks/typescript/collie/package.json",
    "sdks/typescript/foxhound/package.json",
    "uv.lock",
];

/// The fixture committed, so `git diff` shows what the command changed, and
/// tagged as `1.4.0`, which a patch's formats are compared with.
fn committed() -> Workspace {
    let workspace = Workspace::from_fixture("release");
    workspace.commit_all();
    workspace.git(&["tag", "v1.4.0"]);
    workspace
}

/// What the command changed, each changed line with its file, without the
/// blob hashes.
fn changes(workspace: &Workspace) -> String {
    workspace
        .git(&["diff", "--no-color", "--unified=0"])
        .lines()
        .filter(|line| !line.starts_with("index ") && !line.starts_with("diff --git "))
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn prepare_versions_only_the_released_manifests() {
    let workspace = committed();

    let outcome = workspace.xtask(&["release", "prepare", "--version", "1.4.0"]);

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    let changed = workspace.git(&["diff", "--name-only"]);
    assert_eq!(changed.lines().collect::<Vec<_>>(), RELEASED);
}

#[rstest]
#[case::stable("1.4.0")]
#[case::candidate("1.4.0-rc.2")]
#[case::patch("1.4.3")]
#[case::nightly("1.5.0-dev.7")]
fn prepare_writes_the_version_in_the_form_of_each_registry(#[case] version: &str) {
    let workspace = committed();

    let outcome = workspace.xtask(&["release", "prepare", "--version", version]);

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    insta::assert_snapshot!(version, changes(&workspace));
}

#[rstest]
#[case::two_numbers("1.4")]
#[case::prefixed("v1.4.0")]
#[case::leading_zero("1.04.0")]
#[case::candidate_of_a_patch("1.4.1-rc.1")]
#[case::candidate_zero("1.4.0-rc.0")]
#[case::other_pre_release("1.4.0-beta.1")]
#[case::build_metadata("1.4.0+abc")]
fn prepare_refuses_a_version_outside_the_release_forms(#[case] version: &str) {
    let workspace = committed();

    let outcome = workspace.xtask(&["release", "prepare", "--version", version]);

    assert_eq!(outcome.code, 2, "{}", outcome.stdout);
    assert!(
        outcome.stderr.contains("is not a release version"),
        "{}",
        outcome.stderr
    );
    assert_eq!(workspace.git(&["status", "--porcelain"]), "");
}

#[test]
fn prepare_writes_nothing_when_the_client_crate_is_not_pinned() {
    let workspace = committed();
    workspace.edit("Cargo.toml", ", version = \"=0.0.0\"", "");
    workspace.commit_all();

    let outcome = workspace.xtask(&["release", "prepare", "--version", "1.4.0"]);

    assert_eq!(outcome.code, 2, "{}", outcome.stdout);
    assert!(
        outcome.stderr.contains("goodfetch-collie-client"),
        "{}",
        outcome.stderr
    );
    assert_eq!(workspace.git(&["status", "--porcelain"]), "");
}

#[test]
fn prepare_writes_nothing_when_a_released_manifest_is_missing() {
    let workspace = committed();
    workspace.remove("sdks/python/foxhound/pyproject.toml");

    let outcome = workspace.xtask(&["release", "prepare", "--version", "1.4.0"]);

    assert_eq!(outcome.code, 2, "{}", outcome.stdout);
    assert!(
        outcome
            .stderr
            .contains("sdks/python/foxhound/pyproject.toml"),
        "{}",
        outcome.stderr
    );
    assert_eq!(
        workspace.git(&["status", "--porcelain"]),
        " D sdks/python/foxhound/pyproject.toml\n"
    );
}
