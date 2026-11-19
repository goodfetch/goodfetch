//! `just batch-check` (`ci/batch-check.sh`): before `just merge` of a batch
//! branch, what the batch's combined tree can break that each pull request's
//! own `just quick` did not read. In the batch's fresh worktree it first
//! installs the locked JavaScript and Python dependencies, then runs the
//! checks of the whole change (sign-offs, subjects, the floor guard, the scan
//! for secrets, the notices), the checks that read every document and the
//! drift checks (`just lint-documents`, `just lint-drift`), then the Quick
//! suite over the members that two or more of the batch's branches reach:
//! each branch a merge brought, and the commits made on the batch branch
//! itself as one more. A merge of the base itself, as the gate's merge of
//! `main`'s tip, brings no branch. The steps run through a stand-in `just`,
//! and the installs through stand-in `pnpm` and `uv`, that record each call,
//! and `xtask affected` through a stand-in `cargo` that prints the selection
//! written for the head it is asked about. Repository tooling: the module
//! declares no Feature.
#![cfg(unix)]

use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};

use crate::workspace::Workspace;

/// Stand-in `just`, `pnpm` and `uv` that append their call to `calls` (the
/// arguments alone for `just`, behind the program's name for the others) and
/// fail when it is `failing`'s content, and a stand-in `cargo` that appends
/// its own to `cargo-calls` and prints `selection-<head>` for
/// `--head <head>`, an empty selection when there is none, or fails when
/// `cargo-fails` exists.
fn stand_ins(workspace: &Workspace) -> PathBuf {
    let folder = workspace.path().join("stand-in");
    std::fs::create_dir_all(&folder).unwrap();
    let recorder = |prefix: &str| {
        format!(
            r#"#!/bin/sh
call="{prefix}$*"
echo "$call" >>"{folder}/calls"
if [ -f "{folder}/failing" ] && [ "$call" = "$(cat "{folder}/failing")" ]; then
  exit 3
fi
"#,
            folder = folder.display(),
        )
    };
    let cargo = format!(
        r#"#!/bin/sh
echo "$*" >>"{folder}/cargo-calls"
[ ! -f "{folder}/cargo-fails" ] || exit 101
head=
while [ $# -gt 0 ]; do
  if [ "$1" = --head ]; then head=$2; fi
  shift
done
if [ -f "{folder}/selection-$head" ]; then
  cat "{folder}/selection-$head"
else
  printf 'crates=\nvitest-packages=\npytest-packages=\ngate=docs\n'
fi
"#,
        folder = folder.display(),
    );
    for (name, script) in [
        ("just", recorder("")),
        ("pnpm", recorder("pnpm ")),
        ("uv", recorder("uv ")),
        ("cargo", cargo),
    ] {
        let path = folder.join(name);
        std::fs::write(&path, script).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    folder
}

/// Writes the selection `xtask affected` prints for `head`: these crates and
/// these pnpm packages.
fn reaches(stand_in: &Path, head: &str, crates: &str, vitest: &str) {
    std::fs::write(
        stand_in.join(format!("selection-{head}")),
        format!("crates={crates}\nvitest-packages={vitest}\npytest-packages=\ngate=full\n"),
    )
    .unwrap();
}

struct Run {
    code: Option<i32>,
    calls: Vec<String>,
    cargo_calls: Vec<String>,
    stdout: String,
    stderr: String,
}

fn batch_check(workspace: &Workspace, stand_in: &Path) -> Run {
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../ci/batch-check.sh");
    let output = workspace
        .command(script)
        .env("PATH", format!("{}:/usr/bin:/bin", stand_in.display()))
        .output()
        .unwrap();
    let read = |file: &str| -> Vec<String> {
        std::fs::read_to_string(stand_in.join(file))
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect()
    };
    Run {
        code: output.status.code(),
        calls: read("calls"),
        cargo_calls: read("cargo-calls"),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

/// The installs and checks every batch runs before its suite, in their order.
const CHECKS: [&str; 9] = [
    "pnpm install --frozen-lockfile",
    "uv sync --frozen --all-extras",
    "dco origin/main",
    "commit-messages origin/main",
    "floor-guard origin/main",
    "commit-leaks origin/main",
    "notices",
    "lint-documents",
    "lint-drift",
];

/// What the script asks `xtask affected` for a branch from `base` to `head`.
fn affected(base: &str, head: &str) -> String {
    format!("run --quiet --locked --bin xtask -- affected --base {base} --head {head}")
}

/// A repository on `main`, which `origin/main` holds.
fn repository(workspace: &Workspace) {
    std::fs::write(
        workspace.path().join(".gitignore"),
        "/stand-in/\n/target/\n",
    )
    .unwrap();
    workspace.git(&["init", "--quiet", "--initial-branch=main"]);
    workspace.commit_all();
    workspace.git(&["update-ref", "refs/remotes/origin/main", "main"]);
}

/// A branch off `main` with one commit writing `file`; returns its head.
fn branch(workspace: &Workspace, name: &str, file: &str) -> String {
    workspace.git(&["switch", "--quiet", "--create", name, "main"]);
    workspace.write(file, b"changed\n");
    workspace.commit_all()
}

/// The batch branch off `main`, with each of `branches` merged in.
fn batch(workspace: &Workspace, branches: &[&str]) {
    workspace.git(&["switch", "--quiet", "--create", "batch", "main"]);
    for name in branches {
        workspace.git(&["merge", "--quiet", "--no-ff", "--no-edit", name]);
    }
}

/// Two branches green alone can break on their combined tree only where both
/// reach (batch h, 2026-11-18: a corpus test of a crate both reached). The
/// suite runs over those members alone; a later merge of `main`'s newer tip,
/// as the gate makes, brings no branch and is asked nothing.
#[test]
fn the_suite_runs_over_the_members_two_branches_reach() {
    let workspace = Workspace::from_fixture("empty");
    let stand_in = stand_ins(&workspace);
    repository(&workspace);
    let first = branch(&workspace, "first", "a.txt");
    let second = branch(&workspace, "second", "b.txt");
    reaches(
        &stand_in,
        &first,
        "goodfetch-a goodfetch-shared",
        "sdks/typescript/collie",
    );
    reaches(
        &stand_in,
        &second,
        "goodfetch-b goodfetch-shared",
        "sdks/typescript/collie",
    );
    batch(&workspace, &["first", "second"]);
    workspace.git(&["switch", "--quiet", "main"]);
    workspace.write("main.txt", b"main moved\n");
    workspace.commit_all();
    workspace.git(&["update-ref", "refs/remotes/origin/main", "main"]);
    workspace.git(&["switch", "--quiet", "batch"]);
    workspace.git(&["merge", "--quiet", "--no-ff", "--no-edit", "main"]);

    let run = batch_check(&workspace, &stand_in);

    assert_eq!(run.code, Some(0), "{}{}", run.stdout, run.stderr);
    let mut calls: Vec<String> = CHECKS.iter().map(|call| (*call).to_owned()).collect();
    calls.push("test quick goodfetch-shared sdks/typescript/collie".to_owned());
    assert_eq!(run.calls, calls);
    let mut asked = vec![
        affected("origin/main", &second),
        affected("origin/main", &first),
    ];
    asked.sort();
    let mut cargo_calls = run.cargo_calls.clone();
    cargo_calls.sort();
    assert_eq!(cargo_calls, asked);
}

/// A batch of one branch has nothing combined: the branch's own `just quick`
/// ran its suite, so no suite runs, and the run says why.
#[test]
fn one_branch_runs_no_suite() {
    let workspace = Workspace::from_fixture("empty");
    let stand_in = stand_ins(&workspace);
    repository(&workspace);
    let only = branch(&workspace, "only", "a.txt");
    reaches(&stand_in, &only, "goodfetch-a", "");
    batch(&workspace, &["only"]);

    let run = batch_check(&workspace, &stand_in);

    assert_eq!(run.code, Some(0), "{}{}", run.stdout, run.stderr);
    assert_eq!(run.calls, CHECKS);
    assert!(run.stdout.contains("no suite"), "{}", run.stdout);
}

/// The commits made on the batch branch itself, such as a fix of the
/// combination, are one more branch together: with a merged branch they
/// share a member, among themselves they do not.
#[test]
fn commits_on_the_batch_branch_itself_count_as_one_branch() {
    let workspace = Workspace::from_fixture("empty");
    let stand_in = stand_ins(&workspace);
    repository(&workspace);
    let merged = branch(&workspace, "merged", "a.txt");
    reaches(&stand_in, &merged, "goodfetch-a", "");
    batch(&workspace, &["merged"]);
    workspace.write("fix.txt", b"fixed\n");
    let fix = workspace.commit_all();
    reaches(&stand_in, &fix, "goodfetch-a goodfetch-own", "");
    workspace.write("fix.txt", b"fixed again\n");
    let again = workspace.commit_all();
    reaches(&stand_in, &again, "goodfetch-own", "");

    let run = batch_check(&workspace, &stand_in);

    assert_eq!(run.code, Some(0), "{}{}", run.stdout, run.stderr);
    assert_eq!(
        run.calls.last().map(String::as_str),
        Some("test quick goodfetch-a")
    );
    assert!(
        run.cargo_calls
            .contains(&affected(&format!("{fix}^"), &fix)),
        "{:?}",
        run.cargo_calls
    );
}

/// A red check ends nothing: every later step and the suite still run, and
/// the end names the red step and fails.
#[test]
fn a_red_check_still_runs_every_later_step_and_the_suite() {
    let workspace = Workspace::from_fixture("empty");
    let stand_in = stand_ins(&workspace);
    repository(&workspace);
    let first = branch(&workspace, "first", "a.txt");
    let second = branch(&workspace, "second", "b.txt");
    reaches(&stand_in, &first, "goodfetch-shared", "");
    reaches(&stand_in, &second, "goodfetch-shared", "");
    batch(&workspace, &["first", "second"]);
    std::fs::write(stand_in.join("failing"), "notices").unwrap();

    let run = batch_check(&workspace, &stand_in);

    assert_eq!(run.code, Some(1), "{}{}", run.stdout, run.stderr);
    let mut calls: Vec<String> = CHECKS.iter().map(|call| (*call).to_owned()).collect();
    calls.push("test quick goodfetch-shared".to_owned());
    assert_eq!(run.calls, calls);
    assert!(run.stdout.contains("failed   notices: "), "{}", run.stdout);
}

/// Without a branch's selection the script cannot tell which members two
/// branches share: it runs no suite and fails, after the checks.
#[test]
fn a_red_affected_runs_no_suite_and_fails() {
    let workspace = Workspace::from_fixture("empty");
    let stand_in = stand_ins(&workspace);
    repository(&workspace);
    branch(&workspace, "first", "a.txt");
    batch(&workspace, &["first"]);
    std::fs::write(stand_in.join("cargo-fails"), "").unwrap();

    let run = batch_check(&workspace, &stand_in);

    assert_eq!(run.code, Some(1), "{}{}", run.stdout, run.stderr);
    assert_eq!(run.calls, CHECKS);
    assert!(run.stderr.contains("xtask affected"), "{}", run.stderr);
}

/// A batch's fresh worktree has no `node_modules` or `.venv` until the
/// installs run (batches k and l, 2026-11-19: `notices` red on a missing
/// package). A red install fails the run and skips, named so, every step
/// that reads the installed packages; the checks of the commits still run.
#[test]
fn a_red_install_skips_the_steps_that_read_the_packages() {
    let workspace = Workspace::from_fixture("empty");
    let stand_in = stand_ins(&workspace);
    repository(&workspace);
    let first = branch(&workspace, "first", "a.txt");
    let second = branch(&workspace, "second", "b.txt");
    reaches(&stand_in, &first, "goodfetch-shared", "");
    reaches(&stand_in, &second, "goodfetch-shared", "");
    batch(&workspace, &["first", "second"]);
    std::fs::write(stand_in.join("failing"), "uv sync --frozen --all-extras").unwrap();

    let run = batch_check(&workspace, &stand_in);

    assert_eq!(run.code, Some(1), "{}{}", run.stdout, run.stderr);
    assert_eq!(run.calls, &CHECKS[..6]);
    for skipped in ["notices", "lint-documents", "lint-drift", "quick-suite"] {
        assert!(
            run.stdout
                .contains(&format!("skipped  {skipped}: uv-sync did not pass")),
            "{}",
            run.stdout
        );
    }
    assert!(run.stdout.contains("failed   uv-sync: "), "{}", run.stdout);
}
