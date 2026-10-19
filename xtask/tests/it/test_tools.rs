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
