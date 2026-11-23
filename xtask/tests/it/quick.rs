//! The pre-push tier (T1), `just quick` (`ci/quick.sh`), which the pre-push
//! hook runs: the compiler cache's coverage, then the checks of the change's
//! commits (sign-offs, subjects, the floor guard and the scan for secrets),
//! then the checks the change's own gate would run at merge. A change of
//! documents alone (`xtask affected`'s `gate=docs`) takes the docs gate's
//! checks, as `just merge` does (ADR 0135); any other change, and `all`,
//! takes the notices, every lint and the Quick suite. What changed is read
//! against the base the hook gives, else the branch's upstream off `main`
//! when it is another branch, never the branch's own remote copy, else
//! `origin/main`. The hook gives the commit the push's remote ref holds only
//! when the push adds the branch's own commits on top of it, not after a
//! rebase or a merge of the branch's base. A run on a clean checkout is
//! recorded with its result, by the layer that ran it, and a run a green one
//! already holds runs only the checks of the change's commits, while the
//! hook runs no `just quick` on a push of heads a green run holds, so it
//! never runs twice on one tree (`TESTING.md`, Checks by role); the record
//! itself is `xtask gate quick`'s, tested with the gate's. The hook first
//! judges the sign-offs and subjects of the pushed commits, on every push,
//! since a tree says nothing of them. Every step runs even after one
//! failed, through `ci/keep-going.sh`, and the run then names each failed step
//! and fails. The steps run through a stand-in
//! `just` that records each call, and `xtask affected` through a stand-in
//! `cargo`. The Quick suite's Cargo targets come from `ci/quick-targets.sh`,
//! run here on a fixture workspace, and it runs its selected members' tests
//! through `ci/quick-nextest.sh`, run here with a stand-in `cargo`. The
//! pre-push tier is repository tooling, so the module declares no Feature.
#![cfg(unix)]

use std::collections::BTreeSet;
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};

use rstest::rstest;

use crate::workspace::Workspace;

/// A stand-in `just` that appends its arguments to `calls` and fails with
/// status 3 when they are `failing`'s content, and a stand-in `cargo` that
/// appends its own to `cargo-calls` and prints `selection`'s content, a
/// selection the repository's own `xtask affected` printed. Each appends the
/// `GIT_DIR` it inherits to `git-dir` ([`inherited_git_dirs`]). The `just`
/// appends the record's recipes, `_quick-held` and `_quick-record`, to
/// `quick-runs` instead ([`quick_runs`]), and `_quick-held` holds the run
/// only when the file `held` exists.
fn stand_ins(workspace: &Workspace) -> PathBuf {
    let folder = workspace.path().join("stand-in");
    std::fs::create_dir_all(&folder).unwrap();
    let just = format!(
        r#"#!/bin/sh
echo "${{GIT_DIR-unset}}" >>"{git_dir}"
case "$1" in
  _quick-*)
    echo "$*" >>"{quick_runs}"
    [ "$1" != _quick-held ] || [ -f "{held}" ] || exit 1
    exit 0
    ;;
esac
echo "$*" >>"{calls}"
if [ -f "{failing}" ] && [ "$*" = "$(cat "{failing}")" ]; then
  exit 3
fi
"#,
        calls = folder.join("calls").display(),
        git_dir = folder.join("git-dir").display(),
        quick_runs = folder.join("quick-runs").display(),
        held = folder.join("held").display(),
        failing = folder.join("failing").display(),
    );
    let cargo = format!(
        r#"#!/bin/sh
echo "$*" >>"{calls}"
echo "${{GIT_DIR-unset}}" >>"{git_dir}"
cat "{selection}"
"#,
        calls = folder.join("cargo-calls").display(),
        git_dir = folder.join("git-dir").display(),
        selection = folder.join("selection").display(),
    );
    for (name, script) in [("just", just), ("cargo", cargo)] {
        let path = folder.join(name);
        std::fs::write(&path, script).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    folder
}

/// The `GIT_DIR` each stand-in call inherited, in call order, `unset` where
/// it had none.
fn inherited_git_dirs(stand_in: &Path) -> Vec<String> {
    std::fs::read_to_string(stand_in.join("git-dir"))
        .unwrap_or_default()
        .lines()
        .map(str::to_owned)
        .collect()
}

/// The calls of the record's recipes, `_quick-held` and `_quick-record`, in
/// call order.
fn quick_runs(stand_in: &Path) -> Vec<String> {
    std::fs::read_to_string(stand_in.join("quick-runs"))
        .unwrap_or_default()
        .lines()
        .map(str::to_owned)
        .collect()
}

/// `xtask affected`'s own selection, in the `affected` fixture, for a
/// change whose gate is `gate`: a document alone (`docs`), a file no test
/// reads (`quick`) or a crate's source (`full`).
fn selection(stand_in: &Path, gate: &str) {
    let (path, text) = match gate {
        "docs" => ("docs/notes.md", "# Notes\n"),
        "quick" => ("docs/diagram.svg", "<svg/>\n"),
        "full" => (
            "crates/foxhound/goodfetch-frontier/src/crawl.rs",
            "//! Crawls.\n",
        ),
        other => panic!("no change takes the {other} gate here"),
    };
    let workspace = crate::affected::repository();
    workspace.write(path, text.as_bytes());
    let outcome = workspace.xtask(&["affected", "--local"]);
    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    assert_eq!(crate::affected::value(&outcome, "gate"), gate);
    std::fs::write(stand_in.join("selection"), outcome.stdout).unwrap();
}

struct Run {
    code: Option<i32>,
    calls: Vec<String>,
    cargo_calls: Vec<String>,
    stdout: String,
    stderr: String,
}

/// Runs the repository's `ci/quick.sh` in the workspace, with the stand-ins
/// first on `PATH`.
fn quick(workspace: &Workspace, stand_in: &Path, args: &[&str]) -> Run {
    quick_with_env(workspace, stand_in, args, &[])
}

/// Runs `ci/quick.sh` as `quick` does, with these variables added.
fn quick_with_env(
    workspace: &Workspace,
    stand_in: &Path,
    args: &[&str],
    variables: &[(&str, &str)],
) -> Run {
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../ci/quick.sh");
    let output = workspace
        .command(script)
        .args(args)
        .env("PATH", format!("{}:/usr/bin:/bin", stand_in.display()))
        .envs(variables.iter().copied())
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

/// What `xtask affected` is asked, before the change's base: the working
/// tree's change since the merge base with it, as the Quick suite's own
/// selection reads it.
const AFFECTED: &str = "run --quiet --locked --bin xtask -- affected --local --since";

#[test]
fn a_change_of_documents_alone_runs_the_docs_gates_checks_and_no_code_lint_or_suite() {
    let workspace = Workspace::from_fixture("empty");
    let stand_in = stand_ins(&workspace);
    selection(&stand_in, "docs");

    let run = quick(&workspace, &stand_in, &[]);

    assert_eq!(run.code, Some(0), "{}", run.stderr);
    assert_eq!(
        run.calls,
        [
            "compiler-cache",
            "dco origin/main",
            "commit-messages origin/main",
            "floor-guard origin/main",
            "commit-leaks origin/main",
            "lint-documents",
            "_skips-report"
        ]
    );
    assert_eq!(run.cargo_calls, [format!("{AFFECTED} origin/main")]);
    assert!(run.stdout.contains("gate=docs"), "{}", run.stdout);
}

#[rstest]
#[case::no_test_reads_it("quick")]
#[case::a_test_reads_it("full")]
fn any_other_change_runs_every_lint_and_the_quick_suite_over_what_changed(#[case] gate: &str) {
    let workspace = Workspace::from_fixture("empty");
    let stand_in = stand_ins(&workspace);
    selection(&stand_in, gate);

    let run = quick(&workspace, &stand_in, &[]);

    assert_eq!(run.code, Some(0), "{}", run.stderr);
    assert_eq!(
        run.calls,
        [
            "compiler-cache",
            "dco origin/main",
            "commit-messages origin/main",
            "floor-guard origin/main",
            "commit-leaks origin/main",
            "notices",
            "lint",
            "test-affected quick --local --since origin/main",
            "_skips-report"
        ]
    );
}

/// A change to what only xtask's codegen tests check (`xtask affected`'s
/// `codegen-tests=true`, here the emitter) runs them after the Quick suite,
/// which runs no `tests/` binary (issue 2892); any other change does not, as
/// the cases above show.
#[test]
fn a_change_the_codegen_tests_alone_check_runs_them_after_the_quick_suite() {
    let workspace = Workspace::from_fixture("empty");
    let stand_in = stand_ins(&workspace);
    let repository = crate::affected::repository();
    repository.write("xtask/src/codegen/emit.rs", b"//! Emits.\n");
    let outcome = repository.xtask(&["affected", "--local"]);
    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    assert_eq!(crate::affected::value(&outcome, "codegen-tests"), "true");
    std::fs::write(stand_in.join("selection"), outcome.stdout).unwrap();

    let run = quick(&workspace, &stand_in, &[]);

    assert_eq!(run.code, Some(0), "{}", run.stderr);
    assert_eq!(
        run.calls,
        [
            "compiler-cache",
            "dco origin/main",
            "commit-messages origin/main",
            "floor-guard origin/main",
            "commit-leaks origin/main",
            "notices",
            "lint",
            "test-affected quick --local --since origin/main",
            "codegen-tests --local --since origin/main",
            "_skips-report"
        ]
    );
}

#[test]
fn the_change_is_read_against_the_base_the_pre_push_hook_gives() {
    let workspace = Workspace::from_fixture("empty");
    let stand_in = stand_ins(&workspace);
    selection(&stand_in, "quick");

    let run = quick_with_env(
        &workspace,
        &stand_in,
        &[],
        &[("GOODFETCH_QUICK_BASE", "4b825dc6")],
    );

    assert_eq!(run.code, Some(0), "{}", run.stderr);
    assert_eq!(run.cargo_calls, [format!("{AFFECTED} 4b825dc6")]);
    assert_eq!(
        run.calls,
        [
            "compiler-cache",
            "dco 4b825dc6",
            "commit-messages 4b825dc6",
            "floor-guard 4b825dc6",
            "commit-leaks 4b825dc6",
            "notices",
            "lint",
            "test-affected quick --local --since 4b825dc6",
            "_skips-report"
        ]
    );
}

/// A repository on `branch`, which tracks `origin/spec`.
fn repository_on(workspace: &Workspace, branch: &str) {
    workspace.git(&["init", "--quiet", &format!("--initial-branch={branch}")]);
    workspace.commit_all();
    workspace.git(&["remote", "add", "origin", "/nonexistent"]);
    workspace.git(&["update-ref", "refs/remotes/origin/spec", "HEAD"]);
    workspace.git(&["branch", "--quiet", "--set-upstream-to=origin/spec"]);
}

/// The floor guard, the scan of the change's commits and `xtask affected`
/// judge the commits of the change `just quick` reads, against its base, `all`
/// too, which asks `xtask affected` nothing. Without the hook's base, off
/// `main`, a branch's upstream that is another branch is the integration
/// branch its change goes into, so on it the ticket's own commits count,
/// never those of the tickets merged into it before; it stays the base while
/// the ticket holds no commit of its own, as here. `main`'s change is read
/// against `origin/main`.
#[rstest]
#[case::the_hooks_base("ticket", Some("4b825dc6"), &[], "4b825dc6", Some("4b825dc6"))]
#[case::the_upstream_off_main("ticket", None, &[], "origin/spec", Some("origin/spec"))]
#[case::main("main", None, &[], "origin/main", Some("origin/main"))]
#[case::all("ticket", None, &["all"], "origin/spec", None)]
fn the_floor_guard_the_commit_scan_and_the_selection_judge_the_change_against_its_base(
    #[case] branch: &str,
    #[case] hook_base: Option<&str>,
    #[case] args: &[&str],
    #[case] base: &str,
    #[case] selection_since: Option<&str>,
) {
    let workspace = Workspace::from_fixture("empty");
    repository_on(&workspace, branch);
    let stand_in = stand_ins(&workspace);
    selection(&stand_in, "docs");
    let variables: Vec<(&str, &str)> = hook_base
        .map(|hook_base| ("GOODFETCH_QUICK_BASE", hook_base))
        .into_iter()
        .collect();

    let run = quick_with_env(&workspace, &stand_in, args, &variables);

    assert_eq!(run.code, Some(0), "{}", run.stderr);
    assert_eq!(
        run.calls[..5],
        [
            "compiler-cache".to_owned(),
            format!("dco {base}"),
            format!("commit-messages {base}"),
            format!("floor-guard {base}"),
            format!("commit-leaks {base}"),
        ]
    );
    let selection_calls: Vec<String> = selection_since
        .map(|since| format!("{AFFECTED} {since}"))
        .into_iter()
        .collect();
    assert_eq!(run.cargo_calls, selection_calls);
}

/// A repository whose `ticket` branch holds one code change since `main` and
/// was pushed with `git push -u` to `origin`, a bare repository in the
/// workspace: `ticket` tracks its own remote copy, which holds its head.
fn a_ticket_pushed_with_its_upstream_set(workspace: &Workspace) {
    std::fs::write(
        workspace.path().join(".gitignore"),
        "/stand-in/\n/target/\n/origin.git/\n",
    )
    .unwrap();
    workspace.git(&["init", "--quiet", "--initial-branch=main"]);
    workspace.commit_all();
    workspace.git(&["init", "--quiet", "--bare", "origin.git"]);
    let origin = workspace.path().join("origin.git");
    workspace.git(&["remote", "add", "origin", origin.to_str().unwrap()]);
    workspace.git(&["push", "--quiet", "origin", "main"]);
    workspace.git(&["switch", "--quiet", "--create", "ticket"]);
    workspace.write("src/lib.rs", b"//! A code change.\n");
    workspace.commit_all();
    workspace.git(&["push", "--quiet", "--set-upstream", "origin", "ticket"]);
}

/// Against its own remote copy a pushed branch changed nothing: no step
/// would judge its commits, no test would run, and the run would still record
/// the tree. So such a branch's change is read against `origin/main`, as when
/// it has no upstream, and its run is held and recorded against it.
#[test]
fn a_pushed_branch_is_judged_against_origin_main_never_against_its_own_remote_copy() {
    let workspace = Workspace::from_fixture("empty");
    let stand_in = stand_ins(&workspace);
    a_ticket_pushed_with_its_upstream_set(&workspace);
    selection(&stand_in, "full");

    let run = quick(&workspace, &stand_in, &[]);

    assert_eq!(run.code, Some(0), "{}", run.stderr);
    assert_eq!(run.cargo_calls, [format!("{AFFECTED} origin/main")]);
    assert_eq!(
        run.calls,
        [
            "compiler-cache",
            "dco origin/main",
            "commit-messages origin/main",
            "floor-guard origin/main",
            "commit-leaks origin/main",
            "notices",
            "lint",
            "test-affected quick --local --since origin/main",
            "_skips-report"
        ]
    );
    let head = workspace.git(&["rev-parse", "HEAD"]);
    let head = head.trim();
    assert_eq!(
        quick_runs(&stand_in),
        [
            format!("_quick-held {head} origin/main changed"),
            format!("_quick-record quick {head} origin/main changed pass"),
        ]
    );
}

/// The hook's base is the commit the push's remote ref holds, so on a pushed
/// branch too the steps judge what the push adds.
#[test]
fn the_base_the_pre_push_hook_gives_wins_on_a_pushed_branch() {
    let workspace = Workspace::from_fixture("empty");
    let stand_in = stand_ins(&workspace);
    a_ticket_pushed_with_its_upstream_set(&workspace);
    selection(&stand_in, "full");

    let run = quick_with_env(
        &workspace,
        &stand_in,
        &[],
        &[("GOODFETCH_QUICK_BASE", "4b825dc6")],
    );

    assert_eq!(run.code, Some(0), "{}", run.stderr);
    assert_eq!(run.cargo_calls, [format!("{AFFECTED} 4b825dc6")]);
    assert_eq!(
        run.calls,
        [
            "compiler-cache",
            "dco 4b825dc6",
            "commit-messages 4b825dc6",
            "floor-guard 4b825dc6",
            "commit-leaks 4b825dc6",
            "notices",
            "lint",
            "test-affected quick --local --since 4b825dc6",
            "_skips-report"
        ]
    );
}

/// Any hook may start `just quick` with git's `GIT_DIR` set; no step, and no
/// build under it, inherits it.
#[test]
fn no_step_inherits_the_git_dir_quick_was_started_with() {
    let workspace = Workspace::from_fixture("empty");
    repository_on(&workspace, "ticket");
    let stand_in = stand_ins(&workspace);
    selection(&stand_in, "docs");
    let git_dir = format!("{}/.git", workspace.path().display());

    let run = quick_with_env(&workspace, &stand_in, &[], &[("GIT_DIR", &git_dir)]);

    assert_eq!(run.code, Some(0), "{}", run.stderr);
    assert_eq!(inherited_git_dirs(&stand_in), ["unset"; 8]);
}

/// A clean checkout on `ticket`, which tracks `origin/spec`, with the
/// stand-ins ignored. Returns its head.
fn a_clean_ticket(workspace: &Workspace) -> String {
    std::fs::write(
        workspace.path().join(".gitignore"),
        "/stand-in/\n/target/\n",
    )
    .unwrap();
    repository_on(workspace, "ticket");
    workspace.git(&["rev-parse", "HEAD"]).trim().to_owned()
}

/// A run on a clean checkout is recorded with the result of the steps after
/// the commit checks, by the layer that ran it, `pre-push` under the hook:
/// `all` too, in its own form. A red commit check alone leaves the tree's
/// result green, since its commit is mended by a rewrite that keeps the tree.
#[rstest]
#[case::green_by_the_docs_gate("docs", "changed", None, "quick", "pass", 0)]
#[case::green_by_the_quick_suite("quick", "changed", None, "quick", "pass", 0)]
#[case::green_under_the_hook("quick", "changed", None, "pre-push", "pass", 0)]
#[case::green_by_all("quick", "all", None, "quick", "pass", 0)]
#[case::red_by_a_lint("quick", "changed", Some("lint"), "quick", "fail", 1)]
#[case::red_by_a_commit_check_alone(
    "quick",
    "changed",
    Some("dco origin/spec"),
    "quick",
    "pass",
    1
)]
fn a_run_on_a_clean_checkout_is_recorded_with_its_trees_result(
    #[case] gate: &str,
    #[case] form: &str,
    #[case] failing: Option<&str>,
    #[case] layer: &str,
    #[case] status: &str,
    #[case] code: i32,
) {
    let args: &[&str] = if form == "all" { &["all"] } else { &[] };
    let workspace = Workspace::from_fixture("empty");
    let stand_in = stand_ins(&workspace);
    let head = a_clean_ticket(&workspace);
    selection(&stand_in, gate);
    if let Some(failing) = failing {
        std::fs::write(stand_in.join("failing"), failing).unwrap();
    }
    // The hook names its layer; a run started by hand names none.
    let variables: Vec<(&str, &str)> = if layer == "pre-push" {
        vec![("GOODFETCH_GATE_LAYER", layer)]
    } else {
        Vec::new()
    };

    let run = quick_with_env(&workspace, &stand_in, args, &variables);

    assert_eq!(run.code, Some(code), "{}", run.stderr);
    assert_eq!(
        quick_runs(&stand_in),
        [
            format!("_quick-held {head} origin/spec {form}"),
            format!("_quick-record {layer} {head} origin/spec {form} {status}"),
        ]
    );
}

/// The tree of uncommitted changes is no commit's: the run is neither held
/// nor recorded, and says so.
#[test]
fn a_run_on_uncommitted_changes_is_neither_held_nor_recorded() {
    let workspace = Workspace::from_fixture("empty");
    let stand_in = stand_ins(&workspace);
    a_clean_ticket(&workspace);
    selection(&stand_in, "quick");
    workspace.write("new.txt", b"not committed\n");

    let run = quick(&workspace, &stand_in, &[]);

    assert_eq!(run.code, Some(0), "{}", run.stderr);
    assert_eq!(quick_runs(&stand_in), Vec::<String>::new());
    assert!(
        run.stdout.contains("neither held nor recorded"),
        "{}",
        run.stdout
    );
}

/// A run a green one already holds runs the compiler cache's check and the
/// checks of the change's commits, which read more than the tree, and no
/// lint or suite, and records nothing again.
#[test]
fn a_held_run_runs_the_checks_of_the_changes_commits_alone() {
    let workspace = Workspace::from_fixture("empty");
    let stand_in = stand_ins(&workspace);
    let head = a_clean_ticket(&workspace);
    selection(&stand_in, "quick");
    std::fs::write(stand_in.join("held"), "").unwrap();

    let run = quick(&workspace, &stand_in, &[]);

    assert_eq!(run.code, Some(0), "{}", run.stderr);
    assert_eq!(
        run.calls,
        [
            "compiler-cache",
            "dco origin/spec",
            "commit-messages origin/spec",
            "floor-guard origin/spec",
            "commit-leaks origin/spec",
            "_skips-report"
        ]
    );
    assert_eq!(run.cargo_calls, Vec::<String>::new());
    assert_eq!(
        quick_runs(&stand_in),
        [format!("_quick-held {head} origin/spec changed")]
    );
}

/// A stand-in `just` for the pre-push hook: records its arguments and the
/// base the hook gives `just quick`, and the `GIT_DIR` it inherits
/// ([`inherited_git_dirs`]).
fn hook_stand_in(workspace: &Workspace) -> PathBuf {
    hook_stand_in_failing(workspace, None)
}

/// [`hook_stand_in`], whose `just` fails when it runs the recipe `failing`.
fn hook_stand_in_failing(workspace: &Workspace, failing: Option<&str>) -> PathBuf {
    let folder = workspace.path().join("stand-in");
    std::fs::create_dir_all(&folder).unwrap();
    let just = format!(
        r#"#!/bin/sh
echo "${{GIT_DIR-unset}}" >>"{git_dir}"
if [ "$1" = _quick-held ]; then
  echo "$*" >>"{quick_runs}"
  [ -f "{held}" ] || exit 1
  exit 0
fi
echo "$* base=${{GOODFETCH_QUICK_BASE-unset}}" >>"{calls}"
[ "$1" != quick ] || echo "${{GOODFETCH_GATE_LAYER-unset}}" >"{layer}"
[ "$1" != "{failing}" ] || exit 1
"#,
        calls = folder.join("calls").display(),
        git_dir = folder.join("git-dir").display(),
        quick_runs = folder.join("quick-runs").display(),
        held = folder.join("held").display(),
        layer = folder.join("layer").display(),
        failing = failing.unwrap_or("no recipe"),
    );
    let path = folder.join("just");
    std::fs::write(&path, just).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    folder
}

/// The hook's calls for the commits a push of `head` adds: their sign-offs
/// and their subjects, each with no base of the hook's, so the recipe takes
/// its own, the merge base with `origin/main`, as the gate does at merge.
fn commit_checks(head: &str) -> [String; 2] {
    [
        format!("dco  {head} base=unset"),
        format!("commit-messages  {head} base=unset"),
    ]
}

/// Runs `.githooks/pre-push` in the workspace with git's lines for the
/// refs a push updates on its standard input.
fn pre_push(workspace: &Workspace, stand_in: &Path, refs: &str) -> Run {
    pre_push_with_env(workspace, stand_in, refs, &[])
}

/// Runs `.githooks/pre-push` as `pre_push` does, with these variables added.
fn pre_push_with_env(
    workspace: &Workspace,
    stand_in: &Path,
    refs: &str,
    variables: &[(&str, &str)],
) -> Run {
    use std::io::Write as _;
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../.githooks/pre-push");
    let mut child = workspace
        .command(script)
        .args(["origin", "/nonexistent"])
        .env("PATH", format!("{}:/usr/bin:/bin", stand_in.display()))
        .envs(variables.iter().copied())
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(refs.as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    Run {
        code: output.status.code(),
        calls: std::fs::read_to_string(stand_in.join("calls"))
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect(),
        cargo_calls: Vec::new(),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

const NO_COMMIT: &str = "0000000000000000000000000000000000000000";

/// A repository with two commits on `ticket`: the first, the commit the
/// remote ref holds, and the head. Returns both.
fn two_commits(workspace: &Workspace) -> (String, String) {
    std::fs::write(
        workspace.path().join(".gitignore"),
        "/stand-in/\n/target/\n",
    )
    .unwrap();
    repository_on(workspace, "ticket");
    let first = workspace.git(&["rev-parse", "HEAD"]).trim().to_owned();
    workspace.write("change.txt", b"changed\n");
    let head = workspace.commit_all();
    (first, head)
}

/// A tree `just quick` passed on says nothing of the messages of the commits
/// that hold it: a reworded commit keeps its tree. So the hook judges the
/// pushed commits' sign-offs and subjects on every push, and a head a green
/// run holds, against the base `just quick` would read it against, spares
/// the push `just quick` alone.
#[test]
fn a_push_of_a_head_a_green_run_holds_checks_its_commits_and_runs_no_quick() {
    let workspace = Workspace::from_fixture("empty");
    let stand_in = hook_stand_in(&workspace);
    let (first, head) = two_commits(&workspace);
    std::fs::write(stand_in.join("held"), "").unwrap();

    let run = pre_push(
        &workspace,
        &stand_in,
        &format!("refs/heads/ticket {head} refs/heads/ticket {first}\n"),
    );

    assert_eq!(run.code, Some(0), "{}", run.stderr);
    assert_eq!(run.calls, commit_checks(&head));
    assert_eq!(
        quick_runs(&stand_in),
        [format!("_quick-held {head} {first} changed")]
    );
}

/// A commit that is not signed off, or whose subject is no Conventional
/// Commit, is only mended by rewriting it, after which a gate that ran on it
/// reuses nothing: the push is refused before `just quick` runs, and both
/// checks have run, so one push names every such commit.
#[rstest]
#[case::a_sign_off("dco")]
#[case::a_subject("commit-messages")]
fn a_push_whose_commits_fail_a_check_is_refused_before_quick_runs(#[case] failing: &str) {
    let workspace = Workspace::from_fixture("empty");
    let stand_in = hook_stand_in_failing(&workspace, Some(failing));
    let (first, head) = two_commits(&workspace);

    let run = pre_push(
        &workspace,
        &stand_in,
        &format!("refs/heads/ticket {head} refs/heads/ticket {first}\n"),
    );

    assert_eq!(run.code, Some(1), "{}", run.stderr);
    assert_eq!(run.calls, commit_checks(&head));
    assert!(run.stderr.contains("error: "), "{}", run.stderr);
}

#[test]
fn a_push_of_two_branches_checks_the_commits_of_each_and_runs_quick_once() {
    let workspace = Workspace::from_fixture("empty");
    let stand_in = hook_stand_in(&workspace);
    let (first, head) = two_commits(&workspace);

    let run = pre_push(
        &workspace,
        &stand_in,
        &format!(
            "refs/heads/other {first} refs/heads/other {NO_COMMIT}\nrefs/heads/ticket {head} refs/heads/ticket {first}\n"
        ),
    );

    assert_eq!(run.code, Some(0), "{}", run.stderr);
    let mut calls = commit_checks(&first).to_vec();
    calls.extend(commit_checks(&head));
    calls.push(format!("quick base={first}"));
    assert_eq!(run.calls, calls);
}

#[test]
fn a_push_of_another_tree_runs_quick_against_the_commit_the_remote_ref_holds() {
    let workspace = Workspace::from_fixture("empty");
    let stand_in = hook_stand_in(&workspace);
    let (first, head) = two_commits(&workspace);

    let run = pre_push(
        &workspace,
        &stand_in,
        &format!("refs/heads/ticket {head} refs/heads/ticket {first}\n"),
    );

    assert_eq!(run.code, Some(0), "{}", run.stderr);
    let mut calls = commit_checks(&head).to_vec();
    calls.push(format!("quick base={first}"));
    assert_eq!(run.calls, calls);
    assert_eq!(
        quick_runs(&stand_in),
        [format!("_quick-held {head} {first} changed")]
    );
    let layer = std::fs::read_to_string(stand_in.join("layer")).unwrap();
    assert_eq!(layer.trim(), "pre-push");
}

/// A push of a batch's branch with an open pull request to `main`, while the
/// pull-request workflow is active, judges its commits and runs no `just
/// quick`: the hosted run the push starts runs the Quick suite (ADR 0135's
/// amendment of 2026-11-22). Without the pull request, with the workflow off,
/// or when GitHub cannot say, `just quick` runs as before.
#[rstest]
#[case::served("1", "active", false)]
#[case::without_a_pull_request("0", "active", true)]
#[case::with_the_workflow_off("1", "disabled_manually", true)]
#[case::when_github_cannot_say("", "", true)]
fn a_push_of_a_branch_the_hosted_tier_serves_judges_its_commits_alone(
    #[case] open: &str,
    #[case] state: &str,
    #[case] quick: bool,
) {
    let workspace = Workspace::from_fixture("empty");
    let stand_in = hook_stand_in(&workspace);
    let gh = format!(
        "#!/bin/sh\necho \"$*\" >>\"{asked}\"\n\
         case \"$1 $2\" in\n\
           'pr list') [ -n '{open}' ] || exit 1; echo '{open}' ;;\n\
           'api repos/{{owner}}/{{repo}}/actions/workflows/pull-request.yml') [ -n '{state}' ] || exit 1; echo '{state}' ;;\n\
         esac\n",
        asked = stand_in.join("gh-calls").display(),
    );
    let path = stand_in.join("gh");
    std::fs::write(&path, gh).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    let (first, head) = two_commits(&workspace);
    let branch = "batch/2026-11-23a";

    let run = pre_push(
        &workspace,
        &stand_in,
        &format!("refs/heads/{branch} {head} refs/heads/{branch} {first}\n"),
    );

    assert_eq!(run.code, Some(0), "{}", run.stderr);
    let mut calls = commit_checks(&head).to_vec();
    if quick {
        calls.push(format!("quick base={first}"));
    }
    assert_eq!(run.calls, calls);
    let asked = std::fs::read_to_string(stand_in.join("gh-calls")).unwrap();
    assert!(
        asked.starts_with(&format!(
            "pr list --head {branch} --base main --state open --json number --jq length\n"
        )),
        "{asked}"
    );
    assert_eq!(
        run.stdout
            .contains("the hosted tier serves, so its hosted run runs the Quick suite"),
        !quick,
        "{}",
        run.stdout
    );
}

/// A run's ticket ends with no `just quick` of its own (Checks by role, the
/// owner's decision of 2026-11-23): its push to `spec/<n>` judges the pushed
/// commits' sign-offs and subjects alone, and the batch's `just batch-check`
/// runs `just quick` once on the combined tree that merges.
#[test]
fn a_push_to_a_spec_branch_checks_its_commits_and_runs_no_quick() {
    let workspace = Workspace::from_fixture("empty");
    let stand_in = hook_stand_in(&workspace);
    let (first, head) = two_commits(&workspace);

    let run = pre_push(
        &workspace,
        &stand_in,
        &format!("refs/heads/ticket {head} refs/heads/spec/607 {first}\n"),
    );

    assert_eq!(run.code, Some(0), "{}", run.stderr);
    assert_eq!(run.calls, commit_checks(&head));
    assert!(run.stdout.contains("spec/607"), "{}", run.stdout);
}

/// The push's other refs still take `just quick`: only `spec/<n>` leaves it
/// to the batch.
#[test]
fn a_push_of_a_spec_branch_and_another_runs_quick_for_the_other() {
    let workspace = Workspace::from_fixture("empty");
    let stand_in = hook_stand_in(&workspace);
    let (first, head) = two_commits(&workspace);

    let run = pre_push(
        &workspace,
        &stand_in,
        &format!(
            "refs/heads/ticket {head} refs/heads/spec/607 {first}\nrefs/heads/ticket {head} refs/heads/ticket {first}\n"
        ),
    );

    assert_eq!(run.code, Some(0), "{}", run.stderr);
    let mut calls = commit_checks(&head).to_vec();
    calls.extend(commit_checks(&head));
    calls.push(format!("quick base={first}"));
    assert_eq!(run.calls, calls);
}

/// A repository whose `ticket` branch, off `main` with no upstream, added a
/// document and was pushed; `origin/main` then gained a code change, which
/// `main` holds too. Returns the pushed commit.
fn a_pushed_ticket_behind_origin_main(workspace: &Workspace) -> String {
    std::fs::write(
        workspace.path().join(".gitignore"),
        "/stand-in/\n/target/\n",
    )
    .unwrap();
    workspace.git(&["init", "--quiet", "--initial-branch=main"]);
    workspace.commit_all();
    workspace.git(&["switch", "--quiet", "--create", "ticket"]);
    workspace.write("notes.md", b"# Notes\n");
    let pushed = workspace.commit_all();
    workspace.git(&["switch", "--quiet", "main"]);
    workspace.write("crates/x/lib.rs", b"//! A code change.\n");
    workspace.commit_all();
    workspace.git(&["update-ref", "refs/remotes/origin/main", "main"]);
    workspace.git(&["switch", "--quiet", "ticket"]);
    pushed
}

/// After a rebase onto a newer base, or a merge of it into the branch, the
/// commit the remote ref holds is not the branch's base: read against it,
/// the change would hold every file the base gained. The hook gives no base,
/// so `just quick` takes the branch's own, and says why in one line.
#[rstest]
#[case::a_rebase(&["rebase", "--quiet", "origin/main"])]
#[case::a_merge_of_the_base(&["merge", "--quiet", "--no-edit", "origin/main"])]
fn a_push_after_the_branch_took_its_bases_commits_runs_quick_against_its_own_base(
    #[case] taking: &[&str],
) {
    let workspace = Workspace::from_fixture("empty");
    let stand_in = hook_stand_in(&workspace);
    let pushed = a_pushed_ticket_behind_origin_main(&workspace);
    workspace.git(taking);
    let head = workspace.git(&["rev-parse", "HEAD"]).trim().to_owned();

    let run = pre_push(
        &workspace,
        &stand_in,
        &format!("refs/heads/ticket {head} refs/heads/ticket {pushed}\n"),
    );

    assert_eq!(run.code, Some(0), "{}", run.stderr);
    let mut calls = commit_checks(&head).to_vec();
    calls.push("quick base=".to_owned());
    assert_eq!(run.calls, calls);
    let said: Vec<&str> = run
        .stdout
        .lines()
        .filter(|line| line.contains(&pushed))
        .collect();
    assert_eq!(said.len(), 1, "{}", run.stdout);
    assert!(said[0].contains("origin/main"), "{}", run.stdout);
}

/// A branch whose upstream is an integration branch takes it as its own
/// base: a merge of that branch's newer tip sets the remote ref's commit
/// aside, though no `origin/main` exists to compare with.
#[test]
fn a_push_after_a_merge_of_the_integration_branch_runs_quick_against_it() {
    let workspace = Workspace::from_fixture("empty");
    let stand_in = hook_stand_in(&workspace);
    let (_, pushed) = two_commits(&workspace);
    workspace.git(&["switch", "--quiet", "--create", "side", "origin/spec"]);
    workspace.write("spec.txt", b"the integration branch moved\n");
    let moved = workspace.commit_all();
    workspace.git(&["update-ref", "refs/remotes/origin/spec", &moved]);
    workspace.git(&["switch", "--quiet", "ticket"]);
    workspace.git(&["merge", "--quiet", "--no-edit", "origin/spec"]);
    let merged = workspace.git(&["rev-parse", "HEAD"]).trim().to_owned();

    let run = pre_push(
        &workspace,
        &stand_in,
        &format!("refs/heads/ticket {merged} refs/heads/ticket {pushed}\n"),
    );

    assert_eq!(run.code, Some(0), "{}", run.stderr);
    let mut calls = commit_checks(&merged).to_vec();
    calls.push("quick base=".to_owned());
    assert_eq!(run.calls, calls);
    assert!(run.stdout.contains("origin/spec"), "{}", run.stdout);
}

/// A base the pusher names reaches `just quick` when the hook gives none of
/// its own, and the remote ref's commit wins over it when the push only adds
/// the branch's own commits.
#[rstest]
#[case::the_hook_gives_none(true)]
#[case::the_remote_refs_commit(false)]
fn a_base_the_pusher_names_reaches_quick_unless_the_remote_refs_commit_is_the_base(
    #[case] rebased: bool,
) {
    let workspace = Workspace::from_fixture("empty");
    let stand_in = hook_stand_in(&workspace);
    let pushed = a_pushed_ticket_behind_origin_main(&workspace);
    if rebased {
        workspace.git(&["rebase", "--quiet", "origin/main"]);
    } else {
        workspace.write("more.md", b"# More\n");
        workspace.commit_all();
    }
    let head = workspace.git(&["rev-parse", "HEAD"]).trim().to_owned();

    let run = pre_push_with_env(
        &workspace,
        &stand_in,
        &format!("refs/heads/ticket {head} refs/heads/ticket {pushed}\n"),
        &[("GOODFETCH_QUICK_BASE", "origin/main")],
    );

    assert_eq!(run.code, Some(0), "{}", run.stderr);
    let expected = if rebased {
        "origin/main"
    } else {
        pushed.as_str()
    };
    assert_eq!(
        run.calls.last().map(String::as_str),
        Some(format!("quick base={expected}").as_str())
    );
}

/// git gives the hook of a push from a linked worktree that worktree's
/// `GIT_DIR`. `just quick` must not inherit it: a build script's `git init`
/// (btls-sys's) would then reinitialize the repository as bare. Neither do
/// the held check and the two checks of the commits, which build xtask.
#[test]
fn a_push_runs_its_checks_without_the_git_dir_git_gives_the_hook() {
    let workspace = Workspace::from_fixture("empty");
    let stand_in = hook_stand_in(&workspace);
    let (first, head) = two_commits(&workspace);
    let git_dir = format!("{}/.git", workspace.path().display());

    let run = pre_push_with_env(
        &workspace,
        &stand_in,
        &format!("refs/heads/ticket {head} refs/heads/ticket {first}\n"),
        &[("GIT_DIR", &git_dir)],
    );

    assert_eq!(run.code, Some(0), "{}", run.stderr);
    assert_eq!(inherited_git_dirs(&stand_in), ["unset"; 4]);
}

/// A new remote branch holds no commit, so `just quick` takes the branch's
/// own base: its upstream off `main` when it is another branch, else
/// `origin/main`; the held check reads the head against that base too.
#[test]
fn a_push_of_a_new_branch_runs_quick_against_its_own_base() {
    let workspace = Workspace::from_fixture("empty");
    let stand_in = hook_stand_in(&workspace);
    let (_, head) = two_commits(&workspace);

    let run = pre_push(
        &workspace,
        &stand_in,
        &format!("refs/heads/ticket {head} refs/heads/ticket {NO_COMMIT}\n"),
    );

    assert_eq!(run.code, Some(0), "{}", run.stderr);
    let mut calls = commit_checks(&head).to_vec();
    calls.push("quick base=".to_owned());
    assert_eq!(run.calls, calls);
    assert_eq!(
        quick_runs(&stand_in),
        [format!("_quick-held {head} origin/spec changed")]
    );
}

#[test]
fn a_push_that_only_deletes_a_branch_runs_no_check() {
    let workspace = Workspace::from_fixture("empty");
    let stand_in = hook_stand_in(&workspace);
    let (first, _) = two_commits(&workspace);

    let run = pre_push(
        &workspace,
        &stand_in,
        &format!("(delete) {NO_COMMIT} refs/heads/old {first}\n"),
    );

    assert_eq!(run.code, Some(0), "{}", run.stderr);
    assert!(run.calls.is_empty(), "{:?}", run.calls);
}

#[test]
fn all_runs_every_lint_and_the_whole_quick_suite_without_asking_what_changed() {
    let workspace = Workspace::from_fixture("empty");
    let stand_in = stand_ins(&workspace);
    selection(&stand_in, "docs");

    let run = quick(&workspace, &stand_in, &["all"]);

    assert_eq!(run.code, Some(0), "{}", run.stderr);
    assert_eq!(
        run.calls,
        [
            "compiler-cache",
            "dco origin/main",
            "commit-messages origin/main",
            "floor-guard origin/main",
            "commit-leaks origin/main",
            "notices",
            "lint",
            "_suite quick all",
            "_skips-report"
        ]
    );
    assert!(run.cargo_calls.is_empty(), "{:?}", run.cargo_calls);
}

/// A red step does not end the tier: every later step still runs, the Quick
/// suite after a red lint included, so one run reports every failure, and the
/// summary names each failed step.
#[rstest]
#[case::compiler_cache("compiler-cache", "compiler-cache")]
#[case::sign_offs("dco", "dco origin/main")]
#[case::commit_messages("commit-messages", "commit-messages origin/main")]
#[case::floor_guard("floor-guard", "floor-guard origin/main")]
#[case::commit_leaks("commit-leaks", "commit-leaks origin/main")]
#[case::notices("notices", "notices")]
#[case::lint("lint", "lint")]
fn a_red_step_still_runs_every_later_step_and_the_summary_names_it(
    #[case] failing: &str,
    #[case] call: &str,
) {
    let workspace = Workspace::from_fixture("empty");
    let stand_in = stand_ins(&workspace);
    selection(&stand_in, "quick");
    std::fs::write(stand_in.join("failing"), call).unwrap();

    let run = quick(&workspace, &stand_in, &[]);

    assert_eq!(run.code, Some(1), "{}", run.stderr);
    assert_eq!(
        run.calls,
        [
            "compiler-cache",
            "dco origin/main",
            "commit-messages origin/main",
            "floor-guard origin/main",
            "commit-leaks origin/main",
            "notices",
            "lint",
            "test-affected quick --local --since origin/main",
            "_skips-report"
        ]
    );
    assert!(
        run.stdout.contains(&format!("failed   {failing}: ")),
        "{}",
        run.stdout
    );
}

/// Without `xtask affected`'s `gate=` line the tier cannot tell which checks
/// the change takes, so it runs no lint and no suite.
#[test]
fn a_red_affected_ends_the_tier_before_any_lint() {
    let workspace = Workspace::from_fixture("empty");
    let stand_in = stand_ins(&workspace);
    std::fs::write(
        stand_in.join("cargo"),
        format!(
            "#!/bin/sh\necho \"$*\" >>\"{}\"\nexit 101\n",
            stand_in.join("cargo-calls").display()
        ),
    )
    .unwrap();

    let run = quick(&workspace, &stand_in, &[]);

    assert_eq!(run.code, Some(1), "{}", run.stderr);
    assert_eq!(
        run.calls,
        [
            "compiler-cache",
            "dco origin/main",
            "commit-messages origin/main",
            "floor-guard origin/main",
            "commit-leaks origin/main",
            "_skips-report"
        ]
    );
    assert!(run.stderr.contains("xtask affected"), "{}", run.stderr);
}

/// A selection without the `gate=` line, such as one from an `xtask
/// affected` whose keys changed, ends the tier before any lint, as a red
/// `xtask affected` does, instead of taking it for some gate. Two `gate=`
/// lines say neither gate: the reader refuses them as it refuses none, rather
/// than take the last.
#[rstest]
#[case::no_gate("crates=\n")]
#[case::gate_twice("gate=full\ngate=docs\ncrates=\n")]
fn a_selection_without_its_gate_or_with_it_twice_ends_the_tier_before_any_lint(
    #[case] printed: &str,
) {
    let workspace = Workspace::from_fixture("empty");
    let stand_in = stand_ins(&workspace);
    std::fs::write(stand_in.join("selection"), printed).unwrap();

    let run = quick(&workspace, &stand_in, &[]);

    assert_eq!(run.code, Some(1), "{}", run.stderr);
    assert_eq!(
        run.calls,
        [
            "compiler-cache",
            "dco origin/main",
            "commit-messages origin/main",
            "floor-guard origin/main",
            "commit-leaks origin/main",
            "_skips-report"
        ]
    );
    assert!(run.stderr.contains("`gate=`"), "{}", run.stderr);
}

/// Runs `script`, a bash script that sources the repository's
/// `ci/keep-going.sh`, in a new workspace, with `variables` added.
fn keep_going(script: &str, variables: &[(&str, &str)]) -> (Workspace, Run) {
    let workspace = Workspace::from_fixture("empty");
    let library = Path::new(env!("CARGO_MANIFEST_DIR")).join("../ci/keep-going.sh");
    let output = workspace
        .command("bash")
        .arg("-c")
        .arg(format!(
            "set -euo pipefail\nsource \"{}\"\n{script}",
            library.display()
        ))
        .env("PATH", "/usr/bin:/bin")
        .envs(variables.iter().copied())
        .output()
        .unwrap();
    let run = Run {
        code: output.status.code(),
        calls: Vec::new(),
        cargo_calls: Vec::new(),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    };
    (workspace, run)
}

#[test]
fn a_failed_step_fails_the_run_and_the_steps_after_it_still_run() {
    let (workspace, run) = keep_going(
        "steps_begin fake\n\
         step red 'echo the red step said this; exit 4'\n\
         step green 'echo ran > green.txt'\n\
         steps_end\n",
        &[],
    );

    assert_eq!(run.code, Some(1), "{}", run.stderr);
    assert!(
        workspace.path().join("green.txt").is_file(),
        "{}",
        run.stdout
    );
    let log = workspace
        .path()
        .canonicalize()
        .unwrap()
        .join("target/steps/fake/red.log");
    assert!(
        run.stdout
            .contains(&format!("failed   red: {}", log.display())),
        "{}",
        run.stdout
    );
    assert!(!run.stdout.contains("failed   green"), "{}", run.stdout);
    assert_eq!(
        std::fs::read_to_string(log).unwrap(),
        "the red step said this\n"
    );
}

#[test]
fn a_step_after_a_failed_one_it_needs_is_skipped_and_named() {
    let (workspace, run) = keep_going(
        "steps_begin fake\n\
         step build 'exit 1'\n\
         step --after build check 'echo ran > check.txt'\n\
         step --after check later 'echo ran > later.txt'\n\
         steps_end\n",
        &[],
    );

    assert_eq!(run.code, Some(1), "{}", run.stderr);
    assert!(!workspace.path().join("check.txt").exists());
    assert!(!workspace.path().join("later.txt").exists());
    assert!(
        run.stdout.contains("skipped  check: build did not pass"),
        "{}",
        run.stdout
    );
    assert!(
        run.stdout.contains("skipped  later: check did not pass"),
        "{}",
        run.stdout
    );
}

#[test]
fn a_run_whose_steps_all_pass_passes() {
    let (_, run) = keep_going(
        "steps_begin fake\nstep one 'true'\nstep two 'true'\nsteps_end\n",
        &[],
    );

    assert_eq!(run.code, Some(0), "{}", run.stderr);
    assert!(
        run.stdout.contains("fake: 2 passed, 0 failed, 0 skipped."),
        "{}",
        run.stdout
    );
}

/// `just quick`'s step `just lint` is a run of its own: the outer summary
/// names the inner run's failed step, not the step that started it.
#[test]
fn the_summary_names_the_failed_steps_of_a_run_inside_a_step() {
    let workspace = Workspace::from_fixture("empty");
    let library = Path::new(env!("CARGO_MANIFEST_DIR")).join("../ci/keep-going.sh");
    workspace.write(
        "inner.sh",
        format!(
            "set -euo pipefail\nsource \"{}\"\nsteps_begin inner\nstep clippy 'exit 1'\nstep modules 'exit 1'\nsteps_end\n",
            library.display()
        )
        .as_bytes(),
    );

    let (_, run) = keep_going(
        &format!(
            "steps_begin outer\nstep lint 'bash {}'\nstep suite 'true'\nsteps_end\n",
            workspace.path().join("inner.sh").display()
        ),
        &[],
    );

    assert_eq!(run.code, Some(1), "{}", run.stderr);
    assert!(run.stdout.contains("failed   clippy: "), "{}", run.stdout);
    assert!(run.stdout.contains("failed   modules: "), "{}", run.stdout);
    assert!(!run.stdout.contains("failed   lint: "), "{}", run.stdout);
    assert!(
        run.stdout.contains("outer: 1 passed, 2 failed, 0 skipped."),
        "{}",
        run.stdout
    );
}

/// `just quick-steps` lists each step's command without running it.
#[test]
fn the_list_mode_prints_each_command_without_running_it() {
    let (workspace, run) = keep_going(
        "steps_begin fake\n\
         step build 'echo ran > build.txt'\n\
         step --after build check 'cargo check'\n\
         steps_end\n",
        &[("GOODFETCH_STEPS_LIST", "1")],
    );

    assert_eq!(run.code, Some(0), "{}", run.stderr);
    assert_eq!(
        run.stdout,
        "echo ran > build.txt  # build\ncargo check  # check, after build\n"
    );
    assert!(!workspace.path().join("build.txt").exists());
    assert!(!workspace.path().join("target").exists());
}

#[test]
fn an_unknown_argument_is_refused_before_any_step() {
    let workspace = Workspace::from_fixture("empty");
    let stand_in = stand_ins(&workspace);

    let run = quick(&workspace, &stand_in, &["everything"]);

    assert_eq!(run.code, Some(2), "{}", run.stderr);
    assert!(run.calls.is_empty(), "{:?}", run.calls);
}

/// The arguments `cargo` gets, one an item, when cargo-insta runs
/// `ci/quick-nextest.sh run <args>` in place of `cargo nextest` with
/// `GOODFETCH_QUICK_PACKAGES` set to `packages`.
fn quick_nextest(packages: &str, args: &[&str]) -> Vec<String> {
    let workspace = Workspace::from_fixture("empty");
    let calls = workspace.path().join("cargo-calls");
    let cargo = workspace.path().join("cargo");
    std::fs::write(
        &cargo,
        format!(
            "#!/bin/sh\nprintf '%s\\n' \"$@\" >\"{}\"\n",
            calls.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&cargo, std::fs::Permissions::from_mode(0o755)).unwrap();
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../ci/quick-nextest.sh");
    let output = workspace
        .command("bash")
        .arg(script)
        .args(args)
        .env("PATH", "/usr/bin:/bin")
        .env("CARGO", &cargo)
        .env("GOODFETCH_QUICK_PACKAGES", packages)
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    std::fs::read_to_string(calls)
        .unwrap()
        .lines()
        .map(str::to_owned)
        .collect()
}

#[test]
fn the_quick_suite_runs_the_tests_of_the_selected_members_alone() {
    assert_eq!(
        quick_nextest(
            "goodfetch-engine goodfetch-proving-ground",
            &["run", "--all", "--lib", "--bins", "--", "--skip", "a::test"]
        ),
        [
            "nextest",
            "run",
            "-E",
            "package(=goodfetch-engine) | package(=goodfetch-proving-ground)",
            "--all",
            "--lib",
            "--bins",
            "--",
            "--skip",
            "a::test",
        ]
    );
}

#[test]
fn the_quick_suite_runs_every_test_when_no_member_is_named() {
    assert_eq!(
        quick_nextest("", &["run", "--all", "--lib", "--bins"]),
        ["nextest", "run", "--all", "--lib", "--bins"]
    );
}

/// `ci/quick-targets.sh` on the `affected` fixture, whose engine, conformance
/// and frontier crates have a library and whose Proving ground and S3 Sink
/// are executables alone.
fn quick_targets(packages: &[&str]) -> Vec<String> {
    let workspace = Workspace::from_fixture("affected");
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../ci/quick-targets.sh");
    let output = workspace
        .command("bash")
        .arg(script)
        .args(packages)
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

#[rstest]
#[case::the_whole_workspace(&["--workspace"])]
#[case::one_library(&["-p", "goodfetch-engine"])]
#[case::a_library_beside_executables(&["-p", "goodfetch-proving-ground", "-p", "goodfetch-frontier"])]
fn the_quick_suite_builds_libraries_and_executables_when_a_selected_package_has_a_library(
    #[case] packages: &[&str],
) {
    assert_eq!(quick_targets(packages), ["--lib", "--bins"]);
}

/// Cargo refuses `--lib` when no selected package has a library.
#[test]
fn the_quick_suite_builds_executables_alone_when_no_selected_package_has_a_library() {
    assert_eq!(
        quick_targets(&["-p", "goodfetch-proving-ground", "-p", "goodfetch-sink-s3"]),
        ["--bins"]
    );
}

/// The tests nextest's `profile` selects in a workspace, as `<binary id>
/// <test name>`.
fn selected_tests(workspace: &Workspace, profile: &str) -> BTreeSet<String> {
    let listed = workspace.cargo(&[
        "nextest",
        "list",
        "--workspace",
        "--profile",
        profile,
        "--message-format",
        "json",
    ]);
    assert_eq!(listed.code, 0, "{}", listed.stderr);
    let list: serde_json::Value = serde_json::from_str(&listed.stdout).unwrap();
    let mut selected = BTreeSet::new();
    for (binary, suite) in list["rust-suites"].as_object().unwrap() {
        for (name, case) in suite["testcases"].as_object().unwrap() {
            if case["filter-match"]["status"] == "matches" {
                selected.insert(format!("{binary} {name}"));
            }
        }
    }
    selected
}

/// The repository's nextest settings with each of `profiles` keeping only
/// its own `default-filter`: the overrides name the repository's binaries,
/// which a fixture has none of, and nextest refuses a filter that matches
/// no binary. The platform overrides of the Pre-merge suite's profile leave
/// out integration tests alone (`binary_id(<crate>::it)`), none of which the
/// Quick suite runs.
fn default_filters_of_the_repository(profiles: &[&str]) -> String {
    let settings = Path::new(env!("CARGO_MANIFEST_DIR")).join("../.config/nextest.toml");
    let settings: toml::Table = std::fs::read_to_string(settings).unwrap().parse().unwrap();
    let mut kept = toml_edit::DocumentMut::new();
    kept["profile"] = toml_edit::table();
    kept["profile"].as_table_mut().unwrap().set_implicit(true);
    for profile in profiles {
        let filter = settings["profile"][*profile]["default-filter"]
            .as_str()
            .unwrap_or_else(|| panic!("the {profile} profile has no default-filter"));
        kept["profile"][*profile]["default-filter"] = toml_edit::value(filter);
    }
    kept.to_string()
}

/// The full gate runs no Quick suite (ADR 0135's amendment of 2026-11-22):
/// with the repository's own nextest filters, the Pre-merge suite's profile
/// selects every test the Quick suite's selects, the in-process tests of a
/// library, an executable and a procedural macro, and the integration tests
/// besides. The Quick suite's shorter time limit is no selection: a test
/// that passes in it passes under the Pre-merge suite's longer one. Vitest's
/// `unit` project and pytest's tests outside `tests/integration/` are the
/// Pre-merge suite's too, which runs every project and every test.
#[test]
fn the_pre_merge_suite_selects_every_test_the_quick_suite_selects() {
    let workspace = Workspace::from_fixture("empty");
    workspace.write(
        ".config/nextest.toml",
        default_filters_of_the_repository(&["quick", "ci"]).as_bytes(),
    );
    workspace.write(
        "Cargo.toml",
        b"[workspace]\nmembers = [\"tool\", \"derive\"]\nresolver = \"3\"\n",
    );
    workspace.write(
        "tool/Cargo.toml",
        b"[package]\nname = \"tool\"\nversion = \"0.0.0\"\nedition = \"2024\"\npublish = false\n",
    );
    let unit_test =
        |name: &str| format!("#[cfg(test)]\nmod tests {{\n    #[test]\n    fn {name}() {{}}\n}}\n");
    workspace.write("tool/src/lib.rs", unit_test("in_the_library").as_bytes());
    workspace.write(
        "tool/src/main.rs",
        format!("fn main() {{}}\n{}", unit_test("in_the_executable")).as_bytes(),
    );
    workspace.write(
        "tool/tests/it.rs",
        b"#[test]\nfn an_integration_test() {}\n",
    );
    workspace.write(
        "derive/Cargo.toml",
        b"[package]\nname = \"derive\"\nversion = \"0.0.0\"\nedition = \"2024\"\npublish = false\n\n[lib]\nproc-macro = true\n",
    );
    workspace.write("derive/src/lib.rs", unit_test("in_the_macro").as_bytes());

    let quick = selected_tests(&workspace, "quick");
    let pre_merge = selected_tests(&workspace, "ci");

    assert_eq!(
        quick,
        BTreeSet::from(
            [
                "derive tests::in_the_macro",
                "tool tests::in_the_library",
                "tool::bin/tool tests::in_the_executable",
            ]
            .map(str::to_owned)
        )
    );
    assert!(
        quick.is_subset(&pre_merge),
        "{quick:?} against {pre_merge:?}"
    );
    assert!(
        pre_merge.contains("tool::it an_integration_test"),
        "{pre_merge:?}"
    );
}
