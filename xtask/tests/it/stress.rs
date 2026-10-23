//! The stress run before merge (ADR 0064): each new or changed test of a
//! pull request runs 20 times, and a test it takes off the quarantine list
//! 200 times. `xtask stress select` compares each runner's listing of the
//! fixture workspace at its head with the listing at the base; the runners
//! list the tests for real.
//! This spec's stories stay outside the Feature inventory (#325, Out of
//! Scope), so the module declares no Feature.

use crate::workspace::Workspace;

/// How many times a new or changed test runs, and a test taken off the
/// quarantine list (ADR 0064).
const CHANGED: &str = "20";
const UNQUARANTINED: &str = "200";

/// The base: the fixture committed, with each runner's installed packages.
fn base() -> (Workspace, String) {
    let workspace = Workspace::from_fixture("stress");
    workspace.install_runners();
    let base = workspace.commit_all();
    (workspace, base)
}

/// In each runner: adds a test to the `changed` file and renames another
/// there, edits the helper the `imports` tests import, and takes the `flaky`
/// test off the quarantine list; then commits.
fn change(workspace: &Workspace) {
    workspace.edit(
        "src/changed.rs",
        "fn renamed_before() {}",
        "fn renamed_after() {}\n\n    #[test]\n    fn added() {}",
    );
    for file in ["web/changed.test.ts", "e2e/changed.spec.ts"] {
        workspace.edit(
            file,
            "test(\"renamed before\"",
            "test(\"added\", () => {\n    expect(1).toBe(1);\n  });\n\n  test(\"renamed after\"",
        );
    }
    for file in ["web/helper.ts", "e2e/helper.ts"] {
        workspace.write(file, b"export const one = 2 - 1;\n");
    }
    workspace.edit(
        "tests/test_changed.py",
        "def test_renamed_before()",
        "def test_added() -> None:\n    \"\"\"Passes.\"\"\"\n\n\ndef test_renamed_after()",
    );
    workspace.write("quarantine.toml", b"");
    workspace.commit_all();
}

/// `xtask stress select`'s selection, as `(count, runner, test ID)`, sorted.
fn select(workspace: &Workspace, args: &[&str]) -> Vec<(String, String, String)> {
    let outcome = workspace.xtask(&[&["stress", "select"], args].concat());
    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    let mut selection: Vec<(String, String, String)> = outcome
        .stdout
        .lines()
        .map(|line| {
            let mut fields = line.splitn(3, '\t').map(str::to_owned);
            (
                fields.next().unwrap(),
                fields.next().unwrap(),
                fields.next().unwrap(),
            )
        })
        .collect();
    selection.sort();
    selection
}

fn selected(count: &str, runner: &str, test: &str) -> (String, String, String) {
    (count.to_owned(), runner.to_owned(), test.to_owned())
}

/// What the change selects when every runner's base listing is right: the
/// `steady` files' `unchanged` tests are not run.
fn change_selection() -> Vec<(String, String, String)> {
    let mut expected = vec![
        selected(CHANGED, "nextest", "stressed changed::tests::added"),
        selected(CHANGED, "nextest", "stressed changed::tests::renamed_after"),
        selected(CHANGED, "nextest", "stressed changed::tests::kept"),
        selected(UNQUARANTINED, "nextest", "stressed steady::tests::flaky"),
        selected(CHANGED, "vitest", "changed > added"),
        selected(CHANGED, "vitest", "changed > renamed after"),
        selected(CHANGED, "vitest", "changed > kept"),
        selected(CHANGED, "vitest", "imports > helper"),
        selected(UNQUARANTINED, "vitest", "steady > flaky"),
        selected(
            CHANGED,
            "playwright",
            "e2e/changed.spec.ts › changed › added",
        ),
        selected(
            CHANGED,
            "playwright",
            "e2e/changed.spec.ts › changed › renamed after",
        ),
        selected(
            CHANGED,
            "playwright",
            "e2e/changed.spec.ts › changed › kept",
        ),
        selected(
            CHANGED,
            "playwright",
            "e2e/imports.spec.ts › imports › helper",
        ),
        selected(
            UNQUARANTINED,
            "playwright",
            "e2e/steady.spec.ts › steady › flaky",
        ),
        selected(CHANGED, "pytest", "tests/test_changed.py::test_added"),
        selected(
            CHANGED,
            "pytest",
            "tests/test_changed.py::test_renamed_after",
        ),
        selected(CHANGED, "pytest", "tests/test_changed.py::test_kept"),
        selected(UNQUARANTINED, "pytest", "tests/test_steady.py::test_flaky"),
    ];
    expected.sort();
    expected
}

/// No saved listing exists, so the base's listings are built from a
/// checkout of the base.
#[test]
fn new_renamed_and_changed_tests_run_20_times_and_unquarantined_ones_200() {
    let (workspace, base) = base();
    change(&workspace);

    let selection = select(&workspace, &["--base", &base]);

    assert_eq!(selection, change_selection());
}

/// The cache-warming job saves the base's listings. Its pytest listing is
/// then cut short of `test_unchanged`, which the selection takes as new: the
/// saved listing is what is read.
#[test]
fn listing_saved_for_the_base_commit_is_read_instead_of_listing_the_base() {
    let (workspace, base) = base();
    let saved = workspace.xtask(&["stress", "listings", "--out", "saved"]);
    assert_eq!(saved.code, 0, "{}", saved.stderr);
    workspace.edit(
        &format!("saved/{base}/pytest.txt"),
        "tests/test_steady.py::test_unchanged\n",
        "",
    );
    change(&workspace);

    let selection = select(&workspace, &["--base", &base, "--saved", "saved"]);

    let mut expected = change_selection();
    expected.push(selected(
        CHANGED,
        "pytest",
        "tests/test_steady.py::test_unchanged",
    ));
    expected.sort();
    assert_eq!(selection, expected);
}

/// A test still on the quarantine list runs in no suite, so not 20 times
/// either, though its file changed.
#[test]
fn quarantined_test_is_not_stressed() {
    let (workspace, base) = base();
    change(&workspace);
    workspace.write(
        "quarantine.toml",
        b"[[test]]\nrunner = \"pytest\"\nid = \"tests/test_changed.py::test_kept\"\nissue = 1601\ndate = 2026-10-18\n",
    );
    workspace.commit_all();

    let selection = select(&workspace, &["--base", &base]);

    let mut expected = change_selection();
    expected.retain(|(_, _, test)| test != "tests/test_changed.py::test_kept");
    assert_eq!(selection, expected);
}

/// A runner may hold no test, as Playwright before the first Playwright
/// test exists: it lists none, and the others' tests are still chosen.
#[test]
fn runner_with_no_test_lists_none() {
    let (workspace, base) = base();
    change(&workspace);
    workspace.remove_folder("e2e");
    workspace.commit_all();

    let selection = select(&workspace, &["--base", &base]);

    let mut expected = change_selection();
    expected.retain(|(_, runner, _)| runner != "playwright");
    assert_eq!(selection, expected);
}

/// The incremental gate (ADR 0135) stresses only the members the change
/// since an earlier gate run reaches: the crate `stressed` here, so only
/// nextest lists and only its new and changed tests run.
#[test]
fn within_members_only_their_new_and_changed_tests_run() {
    let (workspace, base) = base();
    change(&workspace);

    let selection = select(&workspace, &["--base", &base, "--within", "stressed"]);

    let mut expected = change_selection();
    expected.retain(|(_, runner, _)| runner == "nextest");
    assert_eq!(selection, expected);
}

/// The tests no workspace member owns (the fixture's `web/`, `e2e/` and
/// `tests/`) are stressed within `:unowned`, which every runner but nextest
/// lists.
#[test]
fn within_the_unowned_tests_their_new_and_changed_tests_run() {
    let (workspace, base) = base();
    change(&workspace);

    let selection = select(&workspace, &["--base", &base, "--within", ":unowned"]);

    let mut expected = change_selection();
    expected.retain(|(_, runner, _)| runner != "nextest");
    assert_eq!(selection, expected);
}

/// Every test the change since the earlier gate run changes runs 20 times
/// too, even one the pull request as a whole leaves as its base had it:
/// `steady.rs` changed in the earlier run's head and changed back since.
#[test]
fn a_test_the_change_since_the_earlier_run_changes_runs_20_times() {
    let (workspace, base) = base();
    change(&workspace);
    workspace.append("src/steady.rs", "// Changed.\n");
    let earlier = workspace.commit_all();
    workspace.edit("src/steady.rs", "// Changed.\n", "");
    workspace.commit_all();

    let selection = select(
        &workspace,
        &["--base", &base, "--within", "stressed", "--since", &earlier],
    );

    let mut expected = change_selection();
    expected.retain(|(_, runner, _)| runner == "nextest");
    expected.push(selected(
        CHANGED,
        "nextest",
        "stressed steady::tests::unchanged",
    ));
    expected.sort();
    assert_eq!(selection, expected);
}

/// A branch behind its base, as one that has not merged its target lately:
/// the base's Vitest config imports an entry of a workspace package that the
/// head does not have. The base is listed with its own packages, so its
/// config loads, and the test the change edits is chosen.
#[test]
fn changed_vitest_test_is_selected_when_the_base_config_imports_what_the_head_lacks() {
    let (workspace, base) = base();
    workspace.edit(
        "web-support/package.json",
        r#""./include": "./include.mjs""#,
        r#""./tests": "./include.mjs""#,
    );
    workspace.edit(
        "vitest.config.mjs",
        r#""web-support/include""#,
        r#""web-support/tests""#,
    );
    workspace.edit(
        "web/changed.test.ts",
        r#"test("kept""#,
        "test(\"added\", () => {\n    expect(1).toBe(1);\n  });\n\n  test(\"kept\"",
    );
    workspace.commit_all();

    let selection = select(&workspace, &["--base", &base]);

    assert!(
        selection.contains(&selected(CHANGED, "vitest", "changed > added")),
        "{selection:?}"
    );
}

/// A parameterized test is one test case per parameter, each run under its
/// own name: Vitest's `test.for` formats its title, pytest's `parametrize`
/// adds the parameter's ID to the node ID, and Playwright's loop titles each
/// test. The change adds one in each runner's `changed` file, and each case
/// is chosen by the name it runs under.
#[test]
fn each_case_of_a_parameterized_test_is_selected_by_the_name_it_runs_under() {
    let (workspace, base) = base();
    workspace.edit(
        "web/changed.test.ts",
        r#"test("kept""#,
        "test.for([\"a.log\", \"b\"])(\"reads %s\", (name) => {\n    expect(name).toBeTruthy();\n  });\n\n  test(\"kept\"",
    );
    workspace.edit(
        "e2e/changed.spec.ts",
        r#"test("kept""#,
        "for (const name of [\"a.log\", \"b\"]) {\n    test(`reads ${name}`, () => {\n      expect(name).toBeTruthy();\n    });\n  }\n\n  test(\"kept\"",
    );
    workspace.edit(
        "tests/test_changed.py",
        "def test_kept()",
        "import pytest\n\n\n@pytest.mark.parametrize(\"name\", [\"a.log\", \"b\"])\ndef test_reads(name: str) -> None:\n    \"\"\"Passes.\"\"\"\n    assert name\n\n\ndef test_kept()",
    );
    workspace.commit_all();

    let selection = select(&workspace, &["--base", &base]);

    for (runner, test) in [
        ("vitest", "changed > reads a.log"),
        ("vitest", "changed > reads b"),
        ("playwright", "e2e/changed.spec.ts › changed › reads a.log"),
        ("playwright", "e2e/changed.spec.ts › changed › reads b"),
        ("pytest", "tests/test_changed.py::test_reads[a.log]"),
        ("pytest", "tests/test_changed.py::test_reads[b]"),
    ] {
        assert!(
            selection.contains(&selected(CHANGED, runner, test)),
            "{runner} {test} not in {selection:?}"
        );
    }
    assert!(
        !selection.iter().any(|(_, _, test)| test.contains("%s")),
        "{selection:?}"
    );
}

/// A branch behind its base, as above: the base's `conftest.py` imports a
/// name of a workspace package that the head does not have. The base is
/// listed with its own Python environment, so it collects, and the test the
/// change adds is chosen.
#[test]
fn changed_pytest_test_is_selected_when_the_base_conftest_imports_what_the_head_lacks() {
    let (workspace, base) = base();
    workspace.edit(
        "py-support/src/py_support/__init__.py",
        "included = True",
        "supported = True",
    );
    workspace.edit(
        "tests/conftest.py",
        "from py_support import included\n\nassert included",
        "from py_support import supported\n\nassert supported",
    );
    workspace.edit(
        "tests/test_changed.py",
        "def test_kept()",
        "def test_added() -> None:\n    \"\"\"Passes.\"\"\"\n\n\ndef test_kept()",
    );
    workspace.commit_all();

    let selection = select(&workspace, &["--base", &base]);

    assert!(
        selection.contains(&selected(
            CHANGED,
            "pytest",
            "tests/test_changed.py::test_added"
        )),
        "{selection:?}"
    );
}
