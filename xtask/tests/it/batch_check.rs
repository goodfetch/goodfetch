//! `just batch-check` (`ci/batch-check.sh`): before `just merge` of a batch
//! branch, the batch's one `just quick`. A ticket of a run ends with no
//! `just quick` of its own (Checks by role, the owner's decision of
//! 2026-11-23): its push to `spec/<n>` judges its commits alone, and the
//! batch runs `just quick` once, against the base, on the combined tree of
//! every pull request it holds, so the tree that merges passes every step a
//! ticket's `just quick` ran. In the batch's fresh worktree it first installs
//! the locked JavaScript and Python dependencies, which `just quick`'s steps
//! read. The steps run through a stand-in `just`, and the installs through
//! stand-in `pnpm` and `uv`, that record each call.
//! Repository tooling: the module declares no Feature.
#![cfg(unix)]

use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};

use crate::workspace::Workspace;

/// Stand-in `just`, `pnpm` and `uv` that append their call to `calls` (the
/// arguments alone for `just`, with the base it inherits, behind the
/// program's name for the others) and fail when it is `failing`'s content.
fn stand_ins(workspace: &Workspace) -> PathBuf {
    let folder = workspace.path().join("stand-in");
    std::fs::create_dir_all(&folder).unwrap();
    let recorder = |prefix: &str, base: &str| {
        format!(
            r#"#!/bin/sh
call="{prefix}$*{base}"
echo "$call" >>"{folder}/calls"
if [ -f "{folder}/failing" ] && [ "$call" = "$(cat "{folder}/failing")" ]; then
  exit 3
fi
"#,
            folder = folder.display(),
        )
    };
    for (name, script) in [
        ("just", recorder("", " base=${GOODFETCH_QUICK_BASE-unset}")),
        ("pnpm", recorder("pnpm ", "")),
        ("uv", recorder("uv ", "")),
    ] {
        let path = folder.join(name);
        std::fs::write(&path, script).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    folder
}

struct Run {
    code: Option<i32>,
    calls: Vec<String>,
    stdout: String,
    stderr: String,
}

/// Runs `ci/batch-check.sh` in the workspace with these arguments.
fn batch_check(workspace: &Workspace, stand_in: &Path, arguments: &[&str]) -> Run {
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../ci/batch-check.sh");
    let output = workspace
        .command(script)
        .args(arguments)
        .env("PATH", format!("{}:/usr/bin:/bin", stand_in.display()))
        .env_remove("GOODFETCH_QUICK_BASE")
        .output()
        .unwrap();
    Run {
        code: output.status.code(),
        calls: std::fs::read_to_string(stand_in.join("calls"))
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect(),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

const INSTALLS: [&str; 2] = [
    "pnpm install --frozen-lockfile",
    "uv sync --frozen --all-extras",
];

/// A batch branch off `main`, which `origin/main` holds, with one merged
/// branch.
fn batch(workspace: &Workspace) {
    std::fs::write(
        workspace.path().join(".gitignore"),
        "/stand-in/\n/target/\n",
    )
    .unwrap();
    workspace.git(&["init", "--quiet", "--initial-branch=main"]);
    workspace.commit_all();
    workspace.git(&["update-ref", "refs/remotes/origin/main", "main"]);
    workspace.git(&["switch", "--quiet", "--create", "first", "main"]);
    workspace.write("a.txt", b"changed\n");
    workspace.commit_all();
    workspace.git(&["switch", "--quiet", "--create", "batch", "main"]);
    workspace.git(&["merge", "--quiet", "--no-ff", "--no-edit", "first"]);
}

/// The tickets' pushes ran no `just quick`: the batch runs it once, after the
/// installs, against `origin/main`, so every step a ticket's own run took
/// reads the whole combined change.
#[test]
fn the_batch_runs_one_quick_against_origin_main_after_the_installs() {
    let workspace = Workspace::from_fixture("empty");
    let stand_in = stand_ins(&workspace);
    batch(&workspace);

    let run = batch_check(&workspace, &stand_in, &[]);

    assert_eq!(run.code, Some(0), "{}{}", run.stdout, run.stderr);
    let mut calls: Vec<String> = INSTALLS.iter().map(|call| (*call).to_owned()).collect();
    calls.push("quick base=origin/main".to_owned());
    assert_eq!(run.calls, calls);
}

#[test]
fn a_base_given_is_the_base_quick_reads_the_change_against() {
    let workspace = Workspace::from_fixture("empty");
    let stand_in = stand_ins(&workspace);
    batch(&workspace);

    let run = batch_check(&workspace, &stand_in, &["origin/spec/607"]);

    assert_eq!(run.code, Some(0), "{}{}", run.stdout, run.stderr);
    assert_eq!(
        run.calls.last().map(String::as_str),
        Some("quick base=origin/spec/607")
    );
}

/// A red `just quick` fails the run, and the end names it with its log.
#[test]
fn a_red_quick_fails_the_run_and_is_named_with_its_log() {
    let workspace = Workspace::from_fixture("empty");
    let stand_in = stand_ins(&workspace);
    batch(&workspace);
    std::fs::write(stand_in.join("failing"), "quick base=origin/main").unwrap();

    let run = batch_check(&workspace, &stand_in, &[]);

    assert_eq!(run.code, Some(1), "{}{}", run.stdout, run.stderr);
    assert!(
        run.stdout.contains("failed   quick: ")
            && run.stdout.contains("target/steps/batch-check/quick.log"),
        "{}",
        run.stdout
    );
}

/// A batch's fresh worktree has no `node_modules` or `.venv` until the
/// installs run (batches k and l, 2026-11-19: `notices` red on a missing
/// package). A red install fails the run and skips `just quick`, named so.
#[test]
fn a_red_install_skips_quick_and_fails() {
    let workspace = Workspace::from_fixture("empty");
    let stand_in = stand_ins(&workspace);
    batch(&workspace);
    std::fs::write(stand_in.join("failing"), "uv sync --frozen --all-extras").unwrap();

    let run = batch_check(&workspace, &stand_in, &[]);

    assert_eq!(run.code, Some(1), "{}{}", run.stdout, run.stderr);
    assert_eq!(run.calls, INSTALLS);
    assert!(
        run.stdout.contains("skipped  quick: uv-sync did not pass"),
        "{}",
        run.stdout
    );
    assert!(run.stdout.contains("failed   uv-sync: "), "{}", run.stdout);
}
