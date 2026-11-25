//! `xtask chromium surfaces check` (ADR 0074; TESTING.md, The Goodfetch
//! Chromium): every patch file under `chromium/patches/` has a row in
//! `chromium/surfaces.toml`, and every row names a test its runner lists.
//! One fixture per refused case, each one edit away from a clean fixture
//! that passes. The local tooling declares no Feature.

use rstest::rstest;

use crate::workspace::{Outcome, Workspace, finding};

const TABLE: &str = "chromium/surfaces.toml";

fn check(workspace: &Workspace) -> Outcome {
    workspace.xtask(&["chromium", "surfaces", "check"])
}

fn a_patch_with_no_row(workspace: &Workspace) {
    workspace.write(
        "chromium/patches/goodfetch/0002-fonts.patch",
        b"--- a\n+++ b\n",
    );
}

fn a_row_naming_no_test(workspace: &Workspace) {
    workspace.edit(
        TABLE,
        r#"tests = [{ runner = "nextest", id = "sample tests::the_build_launches" }]"#,
        "tests = []",
    );
}

fn a_row_naming_a_test_that_does_not_exist(workspace: &Workspace) {
    workspace.edit(TABLE, "tests::canvas_is_seeded", "tests::canvas_is_gone");
}

fn a_row_naming_a_patch_that_does_not_exist(workspace: &Workspace) {
    workspace.remove("chromium/patches/goodfetch/0001-build-fix.patch");
}

#[test]
fn a_clean_table_passes() {
    let workspace = Workspace::from_fixture("chromium-surfaces");

    let outcome = check(&workspace);

    assert_eq!(outcome.code, 0, "{}{}", outcome.stdout, outcome.stderr);
    assert_eq!(outcome.findings(), []);
}

#[rstest]
#[case::a_patch_with_no_row(a_patch_with_no_row, "patch-without-row", "goodfetch/0002-fonts.patch")]
#[case::a_row_naming_no_test(a_row_naming_no_test, "row-without-test", "build fixes")]
#[case::a_row_naming_a_test_that_does_not_exist(
    a_row_naming_a_test_that_does_not_exist,
    "test-missing",
    "canvas.toDataURL sample tests::canvas_is_gone"
)]
#[case::a_row_naming_a_patch_that_does_not_exist(
    a_row_naming_a_patch_that_does_not_exist,
    "patch-missing",
    "build fixes goodfetch/0001-build-fix.patch"
)]
fn a_refused_case_fails(#[case] edit: fn(&Workspace), #[case] kind: &str, #[case] subject: &str) {
    let workspace = Workspace::from_fixture("chromium-surfaces");
    edit(&workspace);

    let outcome = check(&workspace);

    assert_eq!(outcome.code, 1, "{}{}", outcome.stdout, outcome.stderr);
    assert_eq!(outcome.findings(), [finding(kind, subject)]);
}

/// Without patches the check lists no test, so the Quick suite builds
/// nothing for it while `chromium/` is empty: a Cargo that cannot start
/// fails a listing, which shows the empty case's pass ran none.
#[test]
fn an_empty_chromium_folder_passes_and_lists_no_test() {
    let with_no_cargo = |workspace: &Workspace| {
        let no_cargo = workspace.path().join("no-cargo-here");
        workspace
            .xtask_with_env(
                &["chromium", "surfaces", "check"],
                &[("CARGO", no_cargo.as_os_str())],
            )
            .code
    };

    let empty = with_no_cargo(&Workspace::from_fixture("empty"));
    let table = with_no_cargo(&Workspace::from_fixture("chromium-surfaces"));

    assert_eq!((empty, table == 0), (0, false));
}
