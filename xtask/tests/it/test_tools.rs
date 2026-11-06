//! `xtask test-tools`, which `just test` runs before any runner starts
//! (ADRs 0062 and 0094): it builds the commit's executables that tests run,
//! those the workspace has, with the tests' own profile, and writes the test
//! tool manifest of their absolute paths.

use std::path::Path;

use serde_json::Value;

use crate::workspace::Workspace;

fn test_tools(workspace: &Workspace) -> Value {
    let outcome = workspace.xtask(&[
        "test-tools",
        "--cargo-profile",
        "dev",
        "--out",
        "target/test-tools.json",
    ]);
    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    serde_json::from_str(&workspace.read_text("target/test-tools.json")).unwrap()
}

#[test]
fn manifest_lists_each_executable_the_workspace_has() {
    let workspace = Workspace::from_fixture("test-tools");

    let manifest = test_tools(&workspace);

    let entries: Vec<&String> = manifest.as_object().unwrap().keys().collect();
    assert_eq!(entries, ["collie", "test_env"]);
}

#[test]
fn each_entry_is_the_absolute_path_of_the_built_executable() {
    let workspace = Workspace::from_fixture("test-tools");

    let manifest = test_tools(&workspace);

    let path = Path::new(manifest["test_env"].as_str().unwrap());
    assert!(path.is_absolute(), "{}", path.display());
    assert!(path.is_file(), "{}", path.display());
    assert_eq!(path.file_stem().unwrap(), "goodfetch-test-env");
}

#[test]
fn a_workspace_without_those_executables_gets_an_empty_manifest() {
    let workspace = Workspace::from_fixture("test-tools");
    workspace.remove_folder("crates/goodfetch-collie");
    workspace.remove_folder("crates/goodfetch-test-support");

    let manifest = test_tools(&workspace);

    assert_eq!(manifest, serde_json::json!({}));
}

/// Two copies of one fixture alive at once build into target folders of
/// their own, so the tests that build a fixture crate can run in parallel
/// without one copy's build replacing the other's (`.config/nextest.toml`,
/// the `fixture-builds` group).
#[test]
fn copies_alive_at_once_build_into_target_folders_of_their_own() {
    let first = Workspace::from_fixture("test-tools");
    let second = Workspace::from_fixture("test-tools");

    let first_manifest = test_tools(&first);
    let second_manifest = test_tools(&second);

    assert_ne!(first_manifest["test_env"], second_manifest["test_env"]);
}

/// A static musl build runs through the musl toolchain's Cargo,
/// `ci/musl/cargo` (Zig through cargo-zigbuild, ADR 0024's note), as the
/// Release suite's musl cells build: here a stand-in that records what it
/// was asked and fails.
#[cfg(unix)]
#[rstest::rstest]
#[case::named_target(&["--target", "aarch64-unknown-linux-musl"])]
#[cfg_attr(target_os = "linux", case::shipped_target_on_linux(&["--shipped-target"]))]
fn a_musl_build_runs_through_the_musl_toolchains_cargo(#[case] target: &[&str]) {
    use std::os::unix::fs::PermissionsExt as _;

    let workspace = Workspace::from_fixture("test-tools");
    workspace.write(
        "ci/musl/cargo",
        b"#!/bin/sh\necho \"$@\" > \"$(dirname \"$0\")/asked\"\nexit 1\n",
    );
    let script = workspace.path().join("ci/musl/cargo");
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    let mut arguments = vec!["test-tools", "--cargo-profile", "dev"];
    arguments.extend_from_slice(target);
    arguments.extend(["--out", "target/test-tools.json"]);

    let outcome = workspace.xtask(&arguments);

    assert_ne!(outcome.code, 0, "{}", outcome.stderr);
    let asked = workspace.read_text("ci/musl/asked");
    assert!(
        asked.starts_with("build ") && asked.contains("-unknown-linux-musl "),
        "{asked}"
    );
}
