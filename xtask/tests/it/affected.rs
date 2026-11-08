//! `xtask affected` reads a change from git, as CI's base and head or as the
//! local working tree against the merge base with `origin/main` (or with the
//! branch `--since` names), and selects
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
    jobs=["pre-merge","sinks","perf"]
    crates=goodfetch-engine goodfetch-frontier
    vitest-packages=
    pytest-packages=
    sinks=goodfetch-sink-s3
    sinks-touched=
    bench-targets=goodfetch-engine/engine
    models=
    gate=full
    "#);
}

#[test]
fn change_to_an_executable_selects_the_crates_whose_tests_run_it() {
    let workspace = repository();
    workspace.write("proving-ground/src/main.rs", b"fn main() { }\n");
    commit_all(&workspace, "Change the Proving ground");

    let outcome = workspace.xtask(&["affected", "--base", "main", "--head", "change"]);

    assert_eq!(
        value(&outcome, "crates"),
        "goodfetch-conformance goodfetch-proving-ground"
    );
}

/// The fixture with a TypeScript and a Python package whose tests run the
/// Proving ground's executable, and the Proving ground depending on the
/// engine through `table`, committed and tagged `with-runs`.
fn with_packages_running_the_proving_ground(workspace: &Workspace, table: &str) {
    workspace.write(
        "sdks/typescript/foxhound/package.json",
        br#"{
  "name": "@goodfetch/foxhound",
  "version": "0.0.0",
  "private": true,
  "dependencies": { "@goodfetch/collie-client": "workspace:*" },
  "goodfetch": { "runs": ["goodfetch-proving-ground"] }
}
"#,
    );
    workspace.write(
        "sdks/python/foxhound/pyproject.toml",
        br#"[project]
name = "goodfetch-foxhound"
version = "0.0.0"
dependencies = ["goodfetch_collie_client>=0.0.0"]

[tool.goodfetch]
runs = ["goodfetch-proving-ground"]
"#,
    );
    workspace.write(
        "proving-ground/Cargo.toml",
        format!(
            r#"[package]
name = "goodfetch-proving-ground"
version = "0.0.0"
edition = "2024"
publish = false

[{table}]
goodfetch-engine = {{ path = "../crates/goodfetch-engine" }}
"#
        )
        .as_bytes(),
    );
    commit_all(workspace, "Run the Proving ground from the packages' tests");
    workspace.git(&["tag", "with-runs"]);
}

#[test]
fn change_to_an_executable_selects_the_packages_whose_tests_run_it() {
    let workspace = repository();
    with_packages_running_the_proving_ground(&workspace, "dependencies");
    workspace.write("proving-ground/src/main.rs", b"fn main() { }\n");
    commit_all(&workspace, "Change the Proving ground");

    let outcome = workspace.xtask(&["affected", "--base", "with-runs", "--head", "change"]);

    assert_eq!(
        (
            value(&outcome, "vitest-packages"),
            value(&outcome, "pytest-packages")
        ),
        (
            "sdks/typescript/foxhound".to_owned(),
            "sdks/python/foxhound".to_owned()
        )
    );
}

/// What an executable is built from changes it, and so the tests that run
/// it; a dev-dependency is no part of its build.
#[rstest]
#[case::through_a_dependency(
    "dependencies",
    "goodfetch-conformance goodfetch-engine goodfetch-frontier goodfetch-proving-ground",
    "sdks/typescript/foxhound"
)]
#[case::not_through_a_dev_dependency(
    "dev-dependencies",
    "goodfetch-engine goodfetch-frontier goodfetch-proving-ground",
    ""
)]
fn change_to_what_an_executable_is_built_from_selects_the_tests_that_run_it(
    #[case] table: &str,
    #[case] crates: &str,
    #[case] vitest_packages: &str,
) {
    let workspace = repository();
    with_packages_running_the_proving_ground(&workspace, table);
    workspace.write("crates/goodfetch-engine/src/lib.rs", b"//! Changed.\n");
    commit_all(&workspace, "Change the engine");

    let outcome = workspace.xtask(&["affected", "--base", "with-runs", "--head", "change"]);

    assert_eq!(
        (
            value(&outcome, "crates"),
            value(&outcome, "vitest-packages")
        ),
        (crates.to_owned(), vitest_packages.to_owned())
    );
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
        (value(&outcome, "crates"), value(&outcome, "jobs")),
        (
            "goodfetch-sink-s3".to_owned(),
            r#"["pre-merge","sinks"]"#.to_owned()
        )
    );
}

#[test]
fn local_mode_reads_uncommitted_and_untracked_changes() {
    let workspace = repository();
    change_the_working_tree(&workspace);

    let outcome = workspace.xtask(&["affected", "--local"]);

    assert_eq!(outcome.code, 0);
    insta::assert_snapshot!(outcome.stdout, @r#"
    jobs=["pre-merge","sinks"]
    crates=goodfetch-frontier
    vitest-packages=sdks/typescript/collie-client sdks/typescript/foxhound
    pytest-packages=sdks/python/collie-client sdks/python/foxhound
    sinks=goodfetch-sink-s3
    sinks-touched=
    bench-targets=
    models=
    gate=full
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

/// A ticket's working tree against the integration branch it goes into: what
/// that branch already holds is not the ticket's change.
#[test]
fn local_mode_since_a_branch_reads_the_working_tree_against_its_merge_base() {
    let workspace = repository();
    workspace.write(
        "crates/foxhound/sinks/goodfetch-sink-s3/src/main.rs",
        b"fn main() { }\n",
    );
    commit_all(&workspace, "Change a Sink on the integration branch");
    workspace.git(&["branch", "integration"]);
    workspace.write("docs/index.md", b"# Changed docs\n");

    let since = workspace.xtask(&["affected", "--local", "--since", "integration"]);
    let against_main = workspace.xtask(&["affected", "--local"]);

    assert_eq!(
        (value(&since, "crates"), value(&since, "gate")),
        (String::new(), "docs".to_owned())
    );
    assert_eq!(
        (value(&against_main, "crates"), value(&against_main, "gate")),
        ("goodfetch-sink-s3".to_owned(), "full".to_owned())
    );
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

/// A TypeScript and a Python package that declare no other member, beside
/// the fixture's, committed and tagged `with-other`; `setup` adds what the
/// case reads first.
fn with_other_packages(workspace: &Workspace, setup: impl FnOnce(&Workspace)) {
    workspace.write(
        "sdks/typescript/other/package.json",
        br#"{ "name": "@goodfetch/other", "version": "0.0.0", "private": true }"#,
    );
    workspace.write(
        "sdks/python/other/pyproject.toml",
        b"[project]\nname = \"goodfetch-other\"\nversion = \"0.0.0\"\n",
    );
    setup(workspace);
    commit_all(workspace, "Add the other packages");
    workspace.git(&["tag", "with-other"]);
}

/// The packages a change reaches, by `xtask affected --base with-other`.
fn reached_packages(workspace: &Workspace) -> (String, String) {
    let outcome = workspace.xtask(&["affected", "--base", "with-other", "--head", "change"]);
    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    (
        value(&outcome, "vitest-packages"),
        value(&outcome, "pytest-packages"),
    )
}

/// What the root runner configs import, and a pytest plugin, run in every
/// package's tests, whatever the package declares (the incremental gate's
/// review, F1).
#[test]
fn a_package_every_run_of_its_runner_loads_reaches_every_package_of_its_ecosystem() {
    let workspace = repository();
    with_other_packages(&workspace, |workspace| {
        workspace.write(
            "vitest.config.ts",
            b"import { CaseLogReporter } from \"@goodfetch/collie-client/case-log\";\nexport default {};\n",
        );
        workspace.write(
            "sdks/python/collie-client/pyproject.toml",
            b"[project]\nname = \"goodfetch-collie-client\"\nversion = \"0.0.0\"\n\n[project.entry-points.pytest11]\nplugin = \"goodfetch_collie_client.plugin\"\n",
        );
    });
    workspace.write(
        "sdks/typescript/collie-client/src/index.ts",
        b"export {};\n",
    );
    workspace.write(
        "sdks/python/collie-client/src/goodfetch_collie_client/plugin.py",
        b"",
    );
    commit_all(&workspace, "Change the loaded packages");

    assert_eq!(
        reached_packages(&workspace),
        (
            "sdks/typescript/collie-client sdks/typescript/foxhound sdks/typescript/other"
                .to_owned(),
            "sdks/python/collie-client sdks/python/foxhound sdks/python/other".to_owned()
        )
    );
}

/// A package that imports a member it does not declare, through the root
/// package's hoisted dependency, is reached by it as by a declared one.
#[test]
fn an_undeclared_import_of_a_member_reaches_the_importer() {
    let workspace = repository();
    with_other_packages(&workspace, |workspace| {
        workspace.write(
            "sdks/typescript/other/tests/frames.spec.ts",
            b"import { frames } from '@goodfetch/collie-client/frames';\n",
        );
        workspace.write(
            "sdks/python/other/tests/test_frames.py",
            b"\"\"\"Frames.\"\"\"\n\nfrom goodfetch_collie_client.frames import frames\n",
        );
    });
    workspace.write(
        "sdks/typescript/collie-client/src/frames.ts",
        b"export {};\n",
    );
    workspace.write(
        "sdks/python/collie-client/src/goodfetch_collie_client/frames.py",
        b"",
    );
    commit_all(&workspace, "Change the frames");

    assert_eq!(
        reached_packages(&workspace),
        (
            "sdks/typescript/collie-client sdks/typescript/foxhound sdks/typescript/other"
                .to_owned(),
            "sdks/python/collie-client sdks/python/foxhound sdks/python/other".to_owned()
        )
    );
}

const TESTING: &str = "# Testing\n\n## Writing a test\n\nProse.\n\n## Quality register\n\n| Row | Limit |\n|---|---|\n| a | 1 |\n";

/// The fixture with TESTING.md on `origin/main`.
fn repository_with_testing() -> Workspace {
    let workspace = repository();
    workspace.write("TESTING.md", TESTING.as_bytes());
    commit_all(&workspace, "Add TESTING.md");
    workspace.git(&["update-ref", "refs/remotes/origin/main", "HEAD"]);
    workspace
}

/// The parts of a changed document are read from git, its base's version
/// against its head's: a prose edit of TESTING.md takes the docs gate, and
/// an edit of its Quality register runs perf, in either mode (hygiene
/// backlog E1).
#[rstest]
#[case::prose(("Prose.", "Reworded prose."), "docs")]
#[case::the_quality_register(("| a | 1 |", "| a | 2 |"), "full")]
fn a_change_to_testing_md_takes_the_gate_of_the_parts_it_changes(
    #[case] edit: (&str, &str),
    #[case] gate: &str,
) {
    let workspace = repository_with_testing();
    workspace.write("TESTING.md", TESTING.replace(edit.0, edit.1).as_bytes());

    let local = workspace.xtask(&["affected", "--local"]);
    commit_all(&workspace, "Edit TESTING.md");
    let committed = workspace.xtask(&["affected", "--base", "origin/main", "--head", "HEAD"]);

    assert_eq!(local.code, 0, "{}", local.stderr);
    assert_eq!(
        (value(&local, "gate"), value(&committed, "gate")),
        (gate.to_owned(), gate.to_owned())
    );
}
