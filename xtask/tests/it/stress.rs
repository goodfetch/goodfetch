//! The stress run before merge (ADR 0064): each new or changed test of a
//! pull request runs 20 times, and a test it takes off the quarantine list
//! 200 times, in an order seeded from the tree under test. `xtask stress
//! select` compares each runner's listing of the fixture workspace at its
//! head with the listing at the base; the runners list the tests for real.
//! This spec's stories stay outside the Feature inventory (#325, Out of
//! Scope), so the module declares no Feature.

use crate::workspace::Workspace;

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
