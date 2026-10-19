//! The stress run before merge (ADR 0064): each new or changed test of a
//! pull request runs 20 times, and a test it takes off the quarantine list
//! 200 times, in an order seeded from the tree under test. `xtask stress
//! select` compares each runner's listing of the fixture workspace at its
//! head with the listing at the base; the runners list the tests for real.
//! This spec's stories stay outside the Feature inventory (#325, Out of
//! Scope), so the module declares no Feature.

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
    workspace.link_installed_runners();
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
