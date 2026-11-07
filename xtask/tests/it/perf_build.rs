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
    crate_with_benches(workspace, &[bench]);
}

/// A workspace of one crate with these bench targets.
fn crate_with_benches(workspace: &Workspace, benches: &[&str]) {
    workspace.write(
        "Cargo.toml",
        b"[workspace]\nresolver = \"3\"\nmembers = [\"crates/foxhound/goodfetch-frontier\"]\n",
    );
    workspace.write(
        "Cargo.lock",
        b"version = 4\n\n[[package]]\nname = \"goodfetch-frontier\"\nversion = \"0.0.0\"\n",
    );
    let targets = benches
        .iter()
        .map(|bench| format!("\n[[bench]]\nname = \"{bench}\"\nharness = false\n"))
        .collect::<Vec<_>>()
        .concat();
    workspace.write(
        MANIFEST,
        format!(
            "[package]\nname = \"goodfetch-frontier\"\nversion = \"0.0.0\"\nedition = \"2024\"\npublish = false\n{targets}"
        )
        .as_bytes(),
    );
    workspace.write(
        "crates/foxhound/goodfetch-frontier/src/lib.rs",
        b"//! The Frontier.\n",
    );
    for bench in benches {
        workspace.write(
            &format!("crates/foxhound/goodfetch-frontier/benches/{bench}.rs"),
            b"//! A benchmark.\n\nfn main() {}\n",
        );
    }
    workspace.write("ci/perf-reference.toml", b"");
}

/// A stand-in for Cargo that hands every command but `fetch` and `bench` to
/// the real one. It logs each `fetch` and `bench` as it starts and ends, with
/// its folder, `CARGO_TARGET_DIR` and arguments, and hands `fetch` on too.
/// Each `bench` waits, up to 10 s, until the other side's has started, and
/// fails when the sides build one after the other; then it writes a line to
/// stderr and exits with `status`.
#[cfg(unix)]
fn building_cargo(workspace: &Workspace, status: u8) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt as _;

    let folder = workspace.path().join("stand-in");
    let runs = folder.join("runs");
    std::fs::create_dir_all(&runs).unwrap();
    let real = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_owned());
    let script = format!(
        r#"#!/bin/sh
case "$1" in
  fetch | bench) ;;
  *) exec "{real}" "$@" ;;
esac
case "$CARGO_TARGET_DIR" in
  */perf-base) side=base ;;
  *) side=head ;;
esac
log() {{ printf '%s\t%s\t%s\t%s\n' "$1" "$(pwd -P)" "$CARGO_TARGET_DIR" "$2" >> "{log}"; }}
log start "$*"
if [ "$1" = fetch ]; then
  "{real}" "$@" || exit $?
  log end "$*"
  exit 0
fi
touch "{runs}/started.$side"
waited=0
until [ "$(ls "{runs}" | grep -c '^started\.')" -gt 1 ]; do
  if [ "$waited" -ge 200 ]; then
    echo "the $side's bench targets built alone" >&2
    exit 1
  fi
  sleep 0.05
  waited=$((waited + 1))
done
echo "built $*" >&2
log end "$*"
exit {status}
"#,
        runs = runs.display(),
        log = folder.join("log").display(),
    );
    let cargo = folder.join("cargo");
    std::fs::write(&cargo, script).unwrap();
    std::fs::set_permissions(&cargo, std::fs::Permissions::from_mode(0o755)).unwrap();
    cargo
}

/// One `fetch` or `bench` the stand-in Cargo logged.
#[cfg(unix)]
#[derive(Debug, PartialEq)]
struct Logged {
    event: String,
    folder: std::path::PathBuf,
    target_directory: String,
    args: String,
}

#[cfg(unix)]
fn logged(workspace: &Workspace) -> Vec<Logged> {
    String::from_utf8(workspace.read("stand-in/log").unwrap())
        .unwrap()
        .lines()
        .map(|line| {
            let [event, folder, target_directory, args] =
                line.split('\t').collect::<Vec<_>>().try_into().unwrap();
            Logged {
                event: event.to_owned(),
                folder: folder.into(),
                target_directory: target_directory.to_owned(),
                args: args.to_owned(),
            }
        })
        .collect()
}

/// The base's bench targets build at the same time as the head's, so the
/// two builds of about 50 minutes each overlap, and each side runs the very
/// commands it ran when they built one after the other: the same arguments,
/// folder and `CARGO_TARGET_DIR`, in the same order, so the binaries and
/// what they measure stay the same. Each line a side's command writes
/// carries the side's name.
#[cfg(unix)]
#[test]
fn the_bases_and_the_heads_bench_targets_build_at_the_same_time() {
    let workspace = Workspace::from_fixture("empty");
    crate_with_benches(&workspace, &["instructions", "heap"]);
    let base = workspace.commit_all();
    let cargo = building_cargo(&workspace, 0);

    let outcome = workspace.xtask_with_env(
        &[
            "perf",
            "build",
            "--base",
            &base,
            "--bench-targets",
            "goodfetch-frontier/instructions goodfetch-frontier/heap",
        ],
        &[("CARGO", cargo.as_os_str())],
    );

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    let plan: Value =
        serde_json::from_slice(&workspace.read("target/perf/plan.json").unwrap()).unwrap();
    let target_directory = plan["target_directory"].as_str().unwrap().to_owned();
    let root = workspace.path().canonicalize().unwrap();
    let tree = root.join("target/perf-base-tree");
    let logged = logged(&workspace);
    let first_end = logged
        .iter()
        .position(|logged| logged.event == "end" && logged.args.starts_with("bench"))
        .unwrap();
    for folder in [&root, &tree] {
        assert!(
            logged[..first_end]
                .iter()
                .any(|logged| &logged.folder == folder && logged.args.starts_with("bench")),
            "no bench of {} started before the first ended: {logged:#?}",
            folder.display()
        );
    }
    let started = |folder: &std::path::Path| -> Vec<(String, String)> {
        logged
            .iter()
            .filter(|logged| logged.event == "start" && logged.folder == folder)
            .map(|logged| (logged.target_directory.clone(), logged.args.clone()))
            .collect()
    };
    let base_directory = format!("{target_directory}/perf-base");
    let bench =
        |name: &str| format!("bench --locked --package goodfetch-frontier --bench {name} --no-run");
    assert_eq!(
        started(&tree),
        vec![
            (base_directory.clone(), "fetch --locked".to_owned()),
            (base_directory.clone(), bench("heap")),
            (base_directory, bench("instructions")),
        ]
    );
    assert_eq!(
        started(&root),
        vec![
            (target_directory.clone(), bench("instructions")),
            (target_directory, bench("heap")),
        ]
    );
    for (side, name) in [
        ("base", "heap"),
        ("base", "instructions"),
        ("head", "instructions"),
        ("head", "heap"),
    ] {
        let line = format!("[{side}] built {}", bench(name));
        assert!(
            outcome.stderr.lines().any(|written| written == line),
            "no `{line}` in:\n{}",
            outcome.stderr
        );
    }
}

/// When both sides fail, the build fails naming each failure, not only the
/// first one to end.
#[cfg(unix)]
#[test]
fn when_both_sides_fail_each_failure_is_reported() {
    let workspace = Workspace::from_fixture("empty");
    crate_with_benches(&workspace, &["instructions"]);
    let base = workspace.commit_all();
    let cargo = building_cargo(&workspace, 1);

    let outcome = workspace.xtask_with_env(
        &[
            "perf",
            "build",
            "--base",
            &base,
            "--bench-targets",
            "goodfetch-frontier/instructions",
        ],
        &[("CARGO", cargo.as_os_str())],
    );

    assert_eq!(outcome.code, 2, "{}", outcome.stderr);
    for side in ["base", "head"] {
        let failure = format!("building the {side}'s goodfetch-frontier/instructions failed");
        assert!(outcome.stderr.contains(&failure), "{}", outcome.stderr);
    }
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
