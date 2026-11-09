//! The test tool manifest that `just test` writes (ADR 0094): a test finds
//! the commit's executables through it, and fails when it or an entry is
//! missing.

use std::path::Path;

use goodfetch_test_support::{TestEnv, Tool, Tools, ToolsError, tools};
use rstest::rstest;

use crate::TEST_ENV;

#[test]
fn an_unset_manifest_fails() {
    let read = Tools::read(None);

    let error = read.unwrap_err();
    assert!(matches!(error, ToolsError::Unset), "{error}");
}

#[test]
fn a_missing_manifest_fails() {
    let env = TestEnv::new();

    let read = Tools::read(Some(&env.folder().join("tools.json")));

    let error = read.unwrap_err();
    assert!(matches!(error, ToolsError::Unreadable { .. }), "{error}");
}

#[rstest]
#[case::collie(Tool::Collie)]
#[case::foxhound(Tool::Foxhound)]
#[case::proving_ground(Tool::ProvingGround)]
#[case::conformance(Tool::Conformance)]
#[case::stub_gateway(Tool::StubGateway)]
fn a_missing_entry_fails(#[case] tool: Tool) {
    let env = TestEnv::new();
    let manifest = env.folder().join("tools.json");
    std::fs::write(&manifest, r#"{"test_env": "/bin/goodfetch-test-env"}"#).unwrap();
    let tools = Tools::read(Some(&manifest)).unwrap();

    let path = tools.path(tool);

    let error = path.unwrap_err();
    assert!(
        matches!(error, ToolsError::Missing(missing) if missing == tool),
        "{error}"
    );
}

#[test]
fn an_entry_of_the_manifest_is_its_path() {
    let env = TestEnv::new();
    let manifest = env.folder().join("tools.json");
    std::fs::write(
        &manifest,
        r#"{"collie": "/work/target/debug/goodfetch-collie"}"#,
    )
    .unwrap();
    let tools = Tools::read(Some(&manifest)).unwrap();

    let path = tools.path(Tool::Collie);

    assert_eq!(
        path.unwrap(),
        Path::new("/work/target/debug/goodfetch-collie")
    );
}

/// `just test` builds the commit's executables with the tests' own profile,
/// so the manifest names the same `goodfetch-test-env` Cargo built for these
/// tests.
#[test]
fn the_manifest_just_test_writes_lists_this_commits_test_env() {
    let tools = tools().unwrap();

    let path = tools.path(Tool::TestEnv);

    assert_eq!(path.unwrap(), Path::new(TEST_ENV));
}
