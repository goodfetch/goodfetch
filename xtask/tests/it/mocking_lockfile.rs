//! The mocking-library check on `uv.lock`: pytest-mock is in no dependency
//! group (ADR 0062).
//! Covers: story:1-107

use crate::workspace::{Workspace, finding};

#[test]
fn lockfile_without_pytest_mock_passes() {
    let workspace = Workspace::from_fixture("uv-lock");

    let outcome = workspace.xtask(&["mocking-lockfile"]);

    assert_eq!(outcome.findings(), vec![]);
    assert_eq!(outcome.code, 0);
}

#[test]
fn pytest_mock_in_any_dependency_group_fails() {
    let workspace = Workspace::from_fixture("uv-lock");
    workspace.add_fixture_file("uv-lock-cases/with-pytest-mock.lock", "uv.lock");

    let outcome = workspace.xtask(&["mocking-lockfile"]);

    assert_eq!(
        outcome.findings(),
        vec![finding("mocking-library", "uv.lock")]
    );
    assert_eq!(outcome.code, 1);
}
