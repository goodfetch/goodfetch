//! The stress run before merge (ADR 0064): each new or changed test of a
//! pull request runs 20 times, and a test it takes off the quarantine list
//! 200 times, in an order seeded from the tree under test. `xtask stress
//! select` compares each runner's listing of the fixture workspace at its
//! head with the listing at the base; the runners list the tests for real.
//! This spec's stories stay outside the Feature inventory (#325, Out of
//! Scope), so the module declares no Feature.

use std::path::Path;

use rstest::rstest;

use crate::workspace::Workspace;

/// How many times a new or changed test runs, and a test taken off the
/// quarantine list (ADR 0064).
const CHANGED: &str = "20";
const UNQUARANTINED: &str = "200";

/// The tree with no entry, whose hash every Git repository knows.
const EMPTY_TREE_SEED: &str = "5441014405922516665";

/// The order seed `xtask stress seed` prints, with its arguments.
fn seed(workspace: &Workspace, args: &[&str]) -> String {
    let outcome = workspace.xtask(&[&["stress", "seed"], args].concat());
    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    outcome.stdout.trim().to_owned()
}

/// The empty tree's hash starts with `4b825dc642cb6eb9`, 5441014405922516665
/// as an unsigned integer.
#[test]
fn order_seed_is_the_first_eight_bytes_of_the_head_tree() {
    let workspace = Workspace::from_fixture("empty");
    workspace.remove("README.md");
    workspace.commit_all();

    let seed = seed(&workspace, &[]);

    assert_eq!(seed, EMPTY_TREE_SEED);
}

#[test]
fn two_commits_of_one_tree_share_their_order_seed() {
    let workspace = Workspace::from_fixture("empty");
    let first = workspace.commit_all();
    let second = workspace.commit_all();

    let seeds = [
        seed(&workspace, &["--tree", &first]),
        seed(&workspace, &["--tree", &second]),
    ];

    assert_eq!(seeds[0], seeds[1]);
    assert_ne!(seeds[0], EMPTY_TREE_SEED);
}

/// `ci/order-seed.sh` gives the seed where no Cargo runs, such as a job's
/// host before its JUnit upload; it must agree with `xtask stress seed`.
#[test]
fn order_seed_script_prints_the_seed_xtask_prints() {
    let workspace = Workspace::from_fixture("empty");
    workspace.commit_all();
    let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../ci/order-seed.sh");

    let output = workspace.command(script).output().unwrap();

    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).unwrap().trim(),
        seed(&workspace, &[])
    );
}

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

/// The repository's `ci/order-arguments.sh`, which the justfile's recipes
/// pass each runner.
fn order_arguments(workspace: &Workspace, runner: &str, seed: &str) -> Vec<String> {
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../ci/order-arguments.sh");
    let output = workspace
        .command("bash")
        .arg(script)
        .arg(runner)
        .env("GOODFETCH_ORDER_SEED", seed)
        .env("PATH", std::env::var_os("PATH").unwrap())
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(str::to_owned)
        .collect()
}

/// Eight tests of `runner`, one per file, so a runner that orders files
/// orders them; each appends its number to `order.log` as it runs, since a
/// JUnit file lists tests by file, not in the order they ran.
fn write_eight_tests(workspace: &Workspace, runner: &str) {
    for n in 1..=8 {
        match runner {
            "vitest" => workspace.write(
                &format!("web/order{n}.test.ts"),
                format!(
                    "import {{ appendFileSync }} from \"node:fs\";\nimport {{ test }} from \"vitest\";\ntest(\"order {n}\", () => appendFileSync(\"order.log\", \"{n}\\n\"));\n"
                )
                .as_bytes(),
            ),
            "playwright" => workspace.write(
                &format!("e2e/order{n}.spec.ts"),
                format!(
                    "import {{ appendFileSync }} from \"node:fs\";\nimport {{ test }} from \"@playwright/test\";\ntest(\"order {n}\", () => appendFileSync(\"order.log\", \"{n}\\n\"));\n"
                )
                .as_bytes(),
            ),
            "pytest" => workspace.write(
                &format!("tests/test_order{n}.py"),
                format!(
                    "from pathlib import Path\n\n\ndef test_order_{n}() -> None:\n    with Path(\"order.log\").open(\"a\") as log:\n        log.write(\"{n}\\n\")\n"
                )
                .as_bytes(),
            ),
            _ => panic!("no runner {runner}"),
        }
    }
}

/// The order `runner` ran the fixture's eight tests in, under `seed`.
fn order(workspace: &Workspace, runner: &str, seed: &str) -> Vec<String> {
    let arguments = order_arguments(workspace, runner, seed);
    let (program, base): (&str, Vec<&str>) = match runner {
        "vitest" => (
            "node_modules/.bin/vitest",
            vec!["run", "--no-file-parallelism"],
        ),
        "playwright" => ("node_modules/.bin/playwright", vec!["test", "--workers=1"]),
        "pytest" => (".venv/bin/pytest", vec!["-p", "no:cacheprovider", "tests"]),
        _ => panic!("no runner {runner}"),
    };
    // A run before this one wrote its own order.
    let _ = std::fs::remove_file(workspace.path().join("order.log"));
    let output = workspace
        .command(workspace.path().join(program))
        .args(base)
        .args(&arguments)
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    workspace
        .read_text("order.log")
        .lines()
        .map(str::to_owned)
        .collect()
}

/// One seed gives one order in each runner, and another seed another
/// (ADR 0064; #333): the arguments the justfile passes seed each runner's
/// order, so a failure's order repeats with its tree.
#[rstest]
#[case::vitest("vitest")]
#[case::playwright("playwright")]
#[case::pytest("pytest")]
fn one_seed_gives_one_order_in_each_runner_and_another_seed_another(#[case] runner: &str) {
    let workspace = Workspace::from_fixture("quarantine");
    workspace.link_installed_runners();
    write_eight_tests(&workspace, runner);

    let first = order(&workspace, runner, "17");
    let again = order(&workspace, runner, "17");
    let other = order(&workspace, runner, "4242");

    assert_eq!(first.len(), 8, "{first:?}");
    assert_eq!(first, again);
    assert_ne!(first, other);
}
