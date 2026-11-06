//! `xtask perf build`, before the deny (ADR 0071): it checks the base out
//! and builds the bench targets to measure, writing the plan `xtask perf run`
//! measures under the deny. The base measures every bench target of each
//! selected crate, so a ceiling whose bench target the change renamed or
//! removed still finds its figure missing in the head (ADR 0134).

use serde_json::Value;

use crate::workspace::Workspace;

const MANIFEST: &str = "crates/foxhound/goodfetch-frontier/Cargo.toml";

/// A workspace of one crate whose one bench target is `bench`.
fn crate_with_bench(workspace: &Workspace, bench: &str) {
    workspace.write(
        "Cargo.toml",
        b"[workspace]\nresolver = \"3\"\nmembers = [\"crates/foxhound/goodfetch-frontier\"]\n",
    );
    workspace.write(
        "Cargo.lock",
        b"version = 4\n\n[[package]]\nname = \"goodfetch-frontier\"\nversion = \"0.0.0\"\n",
    );
    workspace.write(
        MANIFEST,
        format!(
            "[package]\nname = \"goodfetch-frontier\"\nversion = \"0.0.0\"\nedition = \"2024\"\npublish = false\n\n[[bench]]\nname = \"{bench}\"\nharness = false\n"
        )
        .as_bytes(),
    );
    workspace.write(
        "crates/foxhound/goodfetch-frontier/src/lib.rs",
        b"//! The Frontier.\n",
    );
    workspace.write(
        &format!("crates/foxhound/goodfetch-frontier/benches/{bench}.rs"),
        b"//! A benchmark.\n\nfn main() {}\n",
    );
    workspace.write("ci/perf-reference.toml", b"");
}

#[test]
fn base_measures_a_bench_target_the_head_renamed_in_a_selected_crate() {
    let workspace = Workspace::from_fixture("empty");
    crate_with_bench(&workspace, "instructions");
    workspace.commit_all();
    workspace.remove("crates/foxhound/goodfetch-frontier/benches/instructions.rs");
    crate_with_bench(&workspace, "counts");
    workspace.commit_all();

    let outcome = workspace.xtask(&[
        "perf",
        "build",
        "--base",
        "HEAD^1",
        "--bench-targets",
        "goodfetch-frontier/counts",
    ]);

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    let plan: Value =
        serde_json::from_slice(&workspace.read("target/perf/plan.json").unwrap()).unwrap();
    assert_eq!(
        plan["base_targets"],
        serde_json::json!(["goodfetch-frontier/instructions"])
    );
    assert_eq!(
        plan["head_targets"],
        serde_json::json!(["goodfetch-frontier/counts"])
    );
}

/// Another worktree of the repository whose folder `xtask perf` cannot see
/// (in its container, only the measured worktree is mounted) keeps its
/// record: perf moves or removes only its own trees and prunes nothing else.
#[test]
fn a_worktree_out_of_sight_keeps_its_record() {
    let workspace = Workspace::from_fixture("empty");
    crate_with_bench(&workspace, "instructions");
    workspace.commit_all();
    let other = tempfile::tempdir().unwrap();
    let other_tree = other.path().join("other");
    let hidden = other.path().join("hidden");
    workspace.git(&[
        "worktree",
        "add",
        "--quiet",
        "--detach",
        other_tree.to_str().unwrap(),
    ]);
    std::fs::rename(&other_tree, &hidden).unwrap();

    let outcome = workspace.xtask(&["perf", "build", "--base", "HEAD"]);

    std::fs::rename(&hidden, &other_tree).unwrap();
    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    let worktrees = workspace.git(&["worktree", "list", "--porcelain"]);
    let other_tree = other_tree.canonicalize().unwrap();
    assert!(
        worktrees.lines().any(
            |line| line.strip_prefix("worktree ").map(std::path::Path::new) == Some(&other_tree)
        ),
        "{worktrees}"
    );
}

/// A base tree whose folder was deleted without Git (as clearing `target/`
/// does) leaves a record behind; the next build checks the base out there
/// again.
#[test]
fn a_base_tree_deleted_without_git_is_checked_out_again() {
    let workspace = Workspace::from_fixture("empty");
    crate_with_bench(&workspace, "instructions");
    workspace.commit_all();
    let first = workspace.xtask(&["perf", "build", "--base", "HEAD"]);
    assert_eq!(first.code, 0, "{}", first.stderr);
    workspace.remove_folder("target/perf-base-tree");

    let outcome = workspace.xtask(&["perf", "build", "--base", "HEAD"]);

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
}

/// The base's tree stays between builds, at a path outside the folder each
/// build clears, and moves to the next base: Git rewrites only the files the
/// bases differ in, so Cargo, which judges a build by its files' times,
/// rebuilds only what the base changed (the test-gate audit's F2).
#[test]
fn the_bases_tree_is_kept_and_moved_to_the_next_base() {
    let workspace = Workspace::from_fixture("empty");
    crate_with_bench(&workspace, "instructions");
    let first = workspace.commit_all();
    workspace.write(
        "crates/foxhound/goodfetch-frontier/src/lib.rs",
        b"//! The Frontier, as the next base has it.\n",
    );
    let second = workspace.commit_all();
    let tree = workspace.path().join("target/perf-base-tree");
    let bench = tree.join("crates/foxhound/goodfetch-frontier/benches/instructions.rs");
    let built = workspace.xtask(&["perf", "build", "--base", &first]);
    assert_eq!(built.code, 0, "{}", built.stderr);
    let kept = std::fs::metadata(&bench).unwrap().modified().unwrap();
    // A build that checked the base out again would give the file a later time.
    std::thread::sleep(std::time::Duration::from_millis(50));

    let moved = workspace.xtask(&["perf", "build", "--base", &second]);

    assert_eq!(moved.code, 0, "{}", moved.stderr);
    assert_eq!(std::fs::metadata(&bench).unwrap().modified().unwrap(), kept);
    assert_eq!(
        std::fs::read_to_string(tree.join("crates/foxhound/goodfetch-frontier/src/lib.rs"))
            .unwrap(),
        "//! The Frontier, as the next base has it.\n"
    );
    let head = workspace.git(&["-C", tree.to_str().unwrap(), "rev-parse", "HEAD"]);
    assert_eq!(head.trim(), second);
}

/// A file a build of the kept tree left in it, such as a lent Console build
/// or installed packages, is gone once the tree moves to the next base, so
/// the base is built from its own files alone.
#[test]
fn moving_the_bases_tree_removes_what_an_earlier_build_left_in_it() {
    let workspace = Workspace::from_fixture("empty");
    crate_with_bench(&workspace, "instructions");
    let base = workspace.commit_all();
    let built = workspace.xtask(&["perf", "build", "--base", &base]);
    assert_eq!(built.code, 0, "{}", built.stderr);
    let left = workspace
        .path()
        .join("target/perf-base-tree/left-by-a-build.txt");
    std::fs::write(&left, b"").unwrap();

    let again = workspace.xtask(&["perf", "build", "--base", &base]);

    assert_eq!(again.code, 0, "{}", again.stderr);
    assert!(!left.exists());
}
