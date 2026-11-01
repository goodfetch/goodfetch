//! The order every suite runs its tests in (ADR 0064; TESTING.md,
//! Randomness): the order seed is the first 8 bytes of the tree under test,
//! read as an unsigned integer, which `xtask order-seed` prints and
//! `ci/order-seed.sh` gives where no Cargo runs, and `ci/order-arguments.sh`
//! turns it into each runner's arguments. This spec's stories stay outside
//! the Feature inventory (#325, Out of Scope), so the module declares no
//! Feature.

use std::path::Path;

use rstest::rstest;

use crate::workspace::Workspace;

/// The tree with no entry, whose hash every Git repository knows.
const EMPTY_TREE_SEED: &str = "5441014405922516665";

/// The order seed `xtask order-seed` prints, with its arguments.
fn seed(workspace: &Workspace, args: &[&str]) -> String {
    let outcome = workspace.xtask(&[&["order-seed"], args].concat());
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
/// host before its JUnit upload; it must agree with `xtask order-seed`.
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
