//! `xtask test-tools`, which `just test` runs before any runner starts
//! (ADRs 0062 and 0094): `build` builds the commit's executables that tests
//! run, those the workspace has, with the tests' own profile, and writes the
//! test tool manifest of their absolute paths; `path` reads one for a script.

use std::path::Path;

use serde_json::Value;

use crate::workspace::Workspace;

fn test_tools(workspace: &Workspace) -> Value {
    test_tools_with(workspace, &[])
}

fn test_tools_with(workspace: &Workspace, options: &[&str]) -> Value {
    let mut arguments = vec![
        "test-tools",
        "build",
        "--cargo-profile",
        "dev",
        "--out",
        "target/test-tools.json",
    ];
    arguments.extend_from_slice(options);
    let outcome = workspace.xtask(&arguments);
    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    serde_json::from_str(&workspace.read_text("target/test-tools.json")).unwrap()
}

/// `xtask test-tools path <tool>` on the manifest `text`.
fn path(text: &str, tool: &str) -> crate::workspace::Outcome {
    let workspace = Workspace::from_fixture("empty");
    workspace.write("tools.json", text.as_bytes());
    workspace.xtask(&["test-tools", "path", tool, "--manifest", "tools.json"])
}

#[test]
fn path_prints_the_tools_executable() {
    let outcome = path(
        r#"{"proving_ground": "/built/goodfetch-proving-ground"}"#,
        "proving_ground",
    );

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    assert_eq!(outcome.stdout, "/built/goodfetch-proving-ground\n");
}

#[test]
fn a_tool_the_manifest_does_not_name_fails() {
    let outcome = path(r#"{"collie": "/built/goodfetch-collie"}"#, "test_env");

    assert_ne!(outcome.code, 0, "{}", outcome.stdout);
    assert!(
        outcome.stderr.contains("names no executable of test_env"),
        "{}",
        outcome.stderr
    );
}

/// A renamed entry is an error at once, naming the entry, never a tool the
/// manifest seems to lack.
#[test]
fn an_entry_no_tool_has_fails_to_parse() {
    let outcome = path(
        r#"{"proving_ground": "/built/goodfetch-proving-ground", "provinggrounds": "/built/x"}"#,
        "proving_ground",
    );

    assert_ne!(outcome.code, 0, "{}", outcome.stdout);
    assert!(outcome.stderr.contains("parsing"), "{}", outcome.stderr);
    assert!(
        outcome.stderr.contains("provinggrounds"),
        "{}",
        outcome.stderr
    );
}

#[test]
fn the_manifest_names_each_executable_the_workspace_has_by_its_built_absolute_path() {
    let workspace = Workspace::from_fixture("test-tools");

    let manifest = test_tools(&workspace);

    let entries: Vec<&String> = manifest.as_object().unwrap().keys().collect();
    assert_eq!(entries, ["collie", "test_env"]);
    let path = Path::new(manifest["test_env"].as_str().unwrap());
    assert!(path.is_absolute(), "{}", path.display());
    assert!(path.is_file(), "{}", path.display());
    assert_eq!(path.file_stem().unwrap(), "goodfetch-test-env");
}

/// The input record `build` writes beside the manifest.
fn input_record(workspace: &Workspace) -> Value {
    serde_json::from_str(&workspace.read_text("target/test-tools.inputs.json")).unwrap()
}

/// What the tools were built from: the fixture's sources and the files that
/// decide their build, each with its size and time, and nothing of the
/// target folder.
#[test]
fn the_input_record_names_the_workspace_and_each_source_of_its_tools() {
    let workspace = Workspace::from_fixture("test-tools");

    test_tools(&workspace);

    let record = input_record(&workspace);
    assert_eq!(
        std::fs::canonicalize(record["root"].as_str().unwrap()).unwrap(),
        std::fs::canonicalize(workspace.path()).unwrap()
    );
    let inputs: Vec<&str> = record["inputs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|input| input[0].as_str().unwrap())
        .collect();
    assert_eq!(
        inputs,
        [
            "Cargo.lock",
            "Cargo.toml",
            "crates/goodfetch-collie/Cargo.toml",
            "crates/goodfetch-collie/src/main.rs",
            "crates/goodfetch-test-support/Cargo.toml",
            "crates/goodfetch-test-support/src/bin/goodfetch-test-env.rs",
            "crates/goodfetch-test-support/src/lib.rs",
        ]
    );
    let source = &record["inputs"][3];
    let metadata = std::fs::metadata(workspace.path().join(source[0].as_str().unwrap())).unwrap();
    assert_eq!(source[1], metadata.len());
}

/// The record's writer and its reader, `goodfetch-test-support`, agree: a
/// test reads the tools as built, is refused them once a merge or an edit
/// changed a source, and reads them again once they are rebuilt.
#[test]
fn a_test_reads_the_built_tools_until_a_source_of_theirs_changes() {
    use goodfetch_test_support::{Tools, ToolsError};

    let workspace = Workspace::from_fixture("test-tools");
    let manifest = workspace.path().join("target/test-tools.json");
    test_tools(&workspace);
    let built = Tools::read_built_from(Some(&manifest), workspace.path());
    workspace.append(
        "crates/goodfetch-test-support/src/lib.rs",
        "\npub fn merged() {}\n",
    );

    let changed = Tools::read_built_from(Some(&manifest), workspace.path());

    assert!(built.is_ok(), "{}", built.unwrap_err());
    let error = changed.unwrap_err();
    assert!(
        matches!(&error, ToolsError::Stale { input, .. }
            if input == Path::new("crates/goodfetch-test-support/src/lib.rs")),
        "{error}"
    );
    test_tools(&workspace);
    let rebuilt = Tools::read_built_from(Some(&manifest), workspace.path());
    assert!(rebuilt.is_ok(), "{}", rebuilt.unwrap_err());
}

/// The fixture's tools were built from the fixture's copy, not from the
/// tree these tests were built from.
#[test]
fn a_test_of_another_tree_is_refused_the_built_tools() {
    use goodfetch_test_support::{Tools, ToolsError};

    let workspace = Workspace::from_fixture("test-tools");
    let manifest = workspace.path().join("target/test-tools.json");
    test_tools(&workspace);

    let read = Tools::read(Some(&manifest));

    let error = read.unwrap_err();
    assert!(matches!(error, ToolsError::OtherTree { .. }), "{error}");
}

#[test]
fn a_workspace_without_those_executables_gets_an_empty_manifest() {
    let workspace = Workspace::from_fixture("test-tools");
    workspace.remove_folder("crates/goodfetch-collie");
    workspace.remove_folder("crates/goodfetch-test-support");

    let manifest = test_tools(&workspace);

    assert_eq!(manifest, serde_json::json!({}));
}

#[test]
fn a_named_tool_alone_is_built() {
    let workspace = Workspace::from_fixture("test-tools");

    let manifest = test_tools_with(&workspace, &["--tool", "test_env"]);

    let entries: Vec<&String> = manifest.as_object().unwrap().keys().collect();
    assert_eq!(entries, ["test_env"]);
}

/// A skew run's engine (`GOODFETCH_TEST_ENGINE`) is each Product's entry.
#[test]
fn an_engine_is_both_products_executable() {
    let workspace = Workspace::from_fixture("test-tools");
    let engine = workspace.path().join("engine");

    let manifest = test_tools_with(&workspace, &["--engine", engine.to_str().unwrap()]);

    assert_eq!(manifest["collie"], engine.to_str().unwrap());
    assert_eq!(manifest["foxhound"], engine.to_str().unwrap());
    assert!(
        manifest["test_env"]
            .as_str()
            .unwrap()
            .ends_with("goodfetch-test-env"),
        "{manifest}"
    );
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
    let mut arguments = vec!["test-tools", "build", "--cargo-profile", "dev"];
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
