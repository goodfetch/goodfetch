//! `xtask affected` reads a change from git, as CI's base and head or as the
//! local working tree against the merge base with `origin/main`, and selects
//! from the repository's own Cargo, pnpm and uv manifests (ADR 0061). The
//! selection rules themselves are unit tests of the `affected` module, in the
//! Quick suite.
//! Covers: story:1-52

use rstest::rstest;

use crate::workspace::{Outcome, Workspace};

/// The fixture as a git repository whose `main` is also `origin/main`.
pub(crate) fn repository() -> Workspace {
    let workspace = Workspace::from_fixture("affected");
    workspace.git(&["init", "--quiet", "--initial-branch=main"]);
    workspace.git(&["add", "--all"]);
    workspace.git(&["commit", "--quiet", "--message", "Add the fixture"]);
    workspace.git(&["update-ref", "refs/remotes/origin/main", "main"]);
    workspace.git(&["switch", "--quiet", "--create", "change"]);
    workspace
}

pub(crate) fn commit_all(workspace: &Workspace, message: &str) {
    workspace.git(&["add", "--all"]);
    workspace.git(&["commit", "--quiet", "--message", message]);
}

/// The value of the output's `key=value` line for `key`.
pub(crate) fn value(outcome: &Outcome, key: &str) -> String {
    let prefix = format!("{key}=");
    outcome
        .stdout
        .lines()
        .find_map(|line| line.strip_prefix(&prefix))
        .unwrap_or_else(|| panic!("no `{key}` line in:\n{}", outcome.stdout))
        .to_owned()
}

/// A tracked file changed but not staged, a new file staged, and a new file
/// git does not know yet.
fn change_the_working_tree(workspace: &Workspace) {
    workspace.write(
        "sdks/typescript/collie-client/package.json",
        br#"{ "name": "@goodfetch/collie-client", "version": "0.1.0", "private": true }"#,
    );
    workspace.write(
        "crates/foxhound/goodfetch-frontier/src/crawl.rs",
        b"//! Crawls.\n",
    );
    workspace.git(&["add", "crates/foxhound/goodfetch-frontier/src/crawl.rs"]);
    workspace.write(
        "sdks/python/collie-client/src/goodfetch_collie_client/__init__.py",
        b"",
    );
}

#[test]
fn base_and_head_mode_selects_what_the_head_changed() {
    let workspace = repository();
    workspace.write("crates/goodfetch-engine/src/lib.rs", b"//! Changed.\n");
    commit_all(&workspace, "Change the engine");

    let outcome = workspace.xtask(&["affected", "--base", "main", "--head", "change"]);

    assert_eq!(outcome.code, 0);
    insta::assert_snapshot!(outcome.stdout, @r#"
    products=collie foxhound
    jobs=["pre-merge","sinks","perf"]
    nextest-filter=rdeps(=goodfetch-engine)
    crates=goodfetch-engine goodfetch-frontier
    vitest-packages=
    pytest-packages=
    sinks=goodfetch-sink-s3
    sinks-touched=
    bench-targets=goodfetch-engine/engine
    models=
    "#);
}

#[test]
fn base_and_head_mode_ignores_what_the_base_changed_since_the_merge_base() {
    let workspace = repository();
    workspace.write(
        "crates/foxhound/sinks/goodfetch-sink-s3/src/main.rs",
        b"fn main() { }\n",
    );
    commit_all(&workspace, "Change a Sink");
    workspace.git(&["switch", "--quiet", "main"]);
    workspace.write("docs/index.md", b"# Changed docs\n");
    commit_all(&workspace, "Change the docs");

    let outcome = workspace.xtask(&["affected", "--base", "main", "--head", "change"]);

    assert_eq!(
        (value(&outcome, "products"), value(&outcome, "jobs")),
        ("foxhound".to_owned(), r#"["pre-merge","sinks"]"#.to_owned())
    );
}

#[test]
fn local_mode_reads_uncommitted_and_untracked_changes() {
    let workspace = repository();
    change_the_working_tree(&workspace);

    let outcome = workspace.xtask(&["affected", "--local"]);

    assert_eq!(outcome.code, 0);
    insta::assert_snapshot!(outcome.stdout, @r#"
    products=collie foxhound
    jobs=["pre-merge","sinks"]
    nextest-filter=rdeps(=goodfetch-frontier)
    crates=goodfetch-frontier
    vitest-packages=sdks/typescript/collie-client sdks/typescript/foxhound
    pytest-packages=sdks/python/collie-client sdks/python/foxhound
    sinks=goodfetch-sink-s3
    sinks-touched=
    bench-targets=
    models=
    "#);
}

#[test]
fn local_mode_gives_the_same_selection_as_the_base_and_head_mode() {
    let workspace = repository();
    change_the_working_tree(&workspace);
    let local = workspace.xtask(&["affected", "--local"]);
    commit_all(&workspace, "Commit the working tree");

    let committed = workspace.xtask(&["affected", "--base", "origin/main", "--head", "HEAD"]);

    assert_eq!(committed.stdout, local.stdout);
}

#[test]
fn local_mode_without_origin_main_is_an_error() {
    let workspace = repository();
    workspace.git(&["update-ref", "-d", "refs/remotes/origin/main"]);

    let outcome = workspace.xtask(&["affected", "--local"]);

    assert_eq!(outcome.code, 2);
}

/// The visual job runs when a change reaches a package holding a visual test,
/// found where the `visual` projects look: never in a `fixtures/` folder.
#[rstest]
#[case::reaching_a_visual_test(
    "sdks/typescript/foxhound/src/run.visual.spec.ts",
    r#"["pre-merge","sinks","visual"]"#
)]
#[case::reaching_only_a_fixture_named_like_one(
    "sdks/typescript/foxhound/tests/fixtures/page.visual.spec.ts",
    r#"["pre-merge","sinks"]"#
)]
fn change_reaching_a_package_with_a_visual_test_selects_the_visual_job(
    #[case] visual_test: &str,
    #[case] jobs: &str,
) {
    let workspace = repository();
    workspace.write(visual_test, b"// A visual test.\n");
    commit_all(&workspace, "Add the visual test");
    workspace.git(&["tag", "with-visual-test"]);
    workspace.write(
        "sdks/typescript/collie-client/src/index.ts",
        b"// Changed.\n",
    );
    commit_all(&workspace, "Change the client the Foxhound SDK depends on");

    let outcome = workspace.xtask(&["affected", "--base", "with-visual-test", "--head", "change"]);

    assert_eq!(outcome.code, 0);
    assert_eq!(value(&outcome, "jobs"), jobs);
}
