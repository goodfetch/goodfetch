//! The pre-push tier (T1), `just quick` (`ci/quick.sh`), which the pre-push
//! hook runs: the compiler cache's coverage, the floor guard and the scan of
//! the change's commits first, then the checks the change's own gate would run
//! at merge. A change of documents alone (`xtask affected`'s `gate=docs`) takes
//! the docs gate's checks, as `just merge` does (ADR 0135); any other change,
//! and `all`, takes every lint and the Quick suite. What changed is read
//! against the base the hook gives, else the branch's upstream off `main`, else
//! `origin/main`. A green run on a clean checkout records its tree, and the
//! hook skips a push of a recorded tree, so `just quick` never runs twice on
//! one tree (`TESTING.md`, Checks by role). Every step runs even after one
//! failed, through `ci/keep-going.sh`, and the run then names each failed step
//! and fails. The steps run through a stand-in
//! `just` that records each call, and `xtask affected` through a stand-in
//! `cargo`. The Quick suite's Cargo targets come from `ci/quick-targets.sh`,
//! run here on a fixture workspace, and it runs its selected members' tests
//! through `ci/quick-nextest.sh`, run here with a stand-in `cargo`. The
//! pre-push tier is repository tooling, so the module declares no Feature.
#![cfg(unix)]

use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};

use rstest::rstest;

use crate::workspace::Workspace;

/// A stand-in `just` that appends its arguments to `calls` and fails with
/// status 3 when they are `failing`'s content, and a stand-in `cargo` that
/// appends its own to `cargo-calls` and prints `selection`'s content, as
/// `xtask affected` prints its selection.
fn stand_ins(workspace: &Workspace) -> PathBuf {
    let folder = workspace.path().join("stand-in");
    std::fs::create_dir_all(&folder).unwrap();
    let just = format!(
        r#"#!/bin/sh
echo "$*" >>"{calls}"
if [ -f "{failing}" ] && [ "$*" = "$(cat "{failing}")" ]; then
  exit 3
fi
"#,
        calls = folder.join("calls").display(),
        failing = folder.join("failing").display(),
    );
    let cargo = format!(
        r#"#!/bin/sh
echo "$*" >>"{calls}"
cat "{selection}"
"#,
        calls = folder.join("cargo-calls").display(),
        selection = folder.join("selection").display(),
    );
    for (name, script) in [("just", just), ("cargo", cargo)] {
        let path = folder.join(name);
        std::fs::write(&path, script).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    folder
}

/// `xtask affected`'s selection for a change whose gate is `gate`.
fn selection(stand_in: &Path, gate: &str) {
    std::fs::write(
        stand_in.join("selection"),
        format!("products=\njobs=[\"docs\"]\ncrates=\ngate={gate}\n"),
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

/// What `xtask affected` is asked: the working tree's change since the
/// merge base with `origin/main`, as the Quick suite's own selection reads
/// it.
const AFFECTED: &str = "run --quiet --locked --bin xtask -- affected --local";

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
            "floor-guard",
            "commit-leaks",
            "lint-documents",
            "_skips-report"
        ]
    );
    assert_eq!(run.cargo_calls, [AFFECTED]);
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
            "floor-guard",
            "commit-leaks",
            "lint",
            "test-affected quick --local",
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
    assert_eq!(run.cargo_calls, [format!("{AFFECTED} --since 4b825dc6")]);
    assert_eq!(
        run.calls,
        [
            "compiler-cache",
            "floor-guard",
            "commit-leaks",
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

/// Off `main`, a branch's upstream is the integration branch its change goes
/// into; `main`'s change is read against `origin/main`, as before.
#[rstest]
#[case::a_ticket_branch("ticket", Some("origin/spec"))]
#[case::main("main", None)]
fn without_a_base_the_change_is_read_against_the_upstream_off_main(
    #[case] branch: &str,
    #[case] since: Option<&str>,
) {
    let workspace = Workspace::from_fixture("empty");
    repository_on(&workspace, branch);
    let stand_in = stand_ins(&workspace);
    selection(&stand_in, "docs");

    let run = quick(&workspace, &stand_in, &[]);

    assert_eq!(run.code, Some(0), "{}", run.stderr);
    let expected = match since {
        Some(since) => format!("{AFFECTED} --since {since}"),
        None => AFFECTED.to_owned(),
    };
    assert_eq!(run.cargo_calls, [expected]);
}

/// The file in the repository's common folder where a green run records its
/// tree, one a line.
fn recorded_trees(workspace: &Workspace) -> Option<String> {
    let common = workspace.git(&["rev-parse", "--path-format=absolute", "--git-common-dir"]);
    std::fs::read_to_string(Path::new(common.trim()).join("goodfetch-quick-green")).ok()
}

#[rstest]
#[case::the_docs_gate("docs")]
#[case::the_quick_suite("quick")]
fn a_green_run_on_a_clean_checkout_records_its_tree(#[case] gate: &str) {
    let workspace = Workspace::from_fixture("empty");
    let stand_in = stand_ins(&workspace);
    std::fs::write(
        workspace.path().join(".gitignore"),
        "/stand-in/\n/target/\n",
    )
    .unwrap();
    repository_on(&workspace, "ticket");
    selection(&stand_in, gate);

    let run = quick(&workspace, &stand_in, &[]);

    assert_eq!(run.code, Some(0), "{}", run.stderr);
    let tree = workspace.git(&["rev-parse", "HEAD^{tree}"]);
    assert_eq!(recorded_trees(&workspace).as_deref(), Some(tree.as_str()));
}

#[rstest]
#[case::a_red_step(Some("lint"), false)]
#[case::uncommitted_changes(None, true)]
fn a_red_run_or_a_run_on_uncommitted_changes_records_no_tree(
    #[case] failing: Option<&str>,
    #[case] dirty: bool,
) {
    let workspace = Workspace::from_fixture("empty");
    let stand_in = stand_ins(&workspace);
    std::fs::write(
        workspace.path().join(".gitignore"),
        "/stand-in/\n/target/\n",
    )
    .unwrap();
    repository_on(&workspace, "ticket");
    selection(&stand_in, "quick");
    if let Some(failing) = failing {
        std::fs::write(stand_in.join("failing"), failing).unwrap();
    }
    if dirty {
        workspace.write("new.txt", b"not committed\n");
    }

    let run = quick(&workspace, &stand_in, &[]);

    assert_eq!(
        run.code,
        Some(match failing {
            Some(_) => 1,
            None => 0,
        }),
        "{}",
        run.stderr
    );
    assert_eq!(recorded_trees(&workspace), None);
}

/// A stand-in `just` for the pre-push hook: records its arguments and the
/// base the hook gives `just quick`.
fn hook_stand_in(workspace: &Workspace) -> PathBuf {
    let folder = workspace.path().join("stand-in");
    std::fs::create_dir_all(&folder).unwrap();
    let just = format!(
        "#!/bin/sh\necho \"$* base=${{GOODFETCH_QUICK_BASE-unset}}\" >>\"{}\"\n",
        folder.join("calls").display()
    );
    let path = folder.join("just");
    std::fs::write(&path, just).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    folder
}

/// Runs `.githooks/pre-push` in the workspace with git's lines for the
/// refs a push updates on its standard input.
fn pre_push(workspace: &Workspace, stand_in: &Path, refs: &str) -> Run {
    use std::io::Write as _;
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../.githooks/pre-push");
    let mut child = workspace
        .command(script)
        .args(["origin", "/nonexistent"])
        .env("PATH", format!("{}:/usr/bin:/bin", stand_in.display()))
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

#[test]
fn a_push_of_a_tree_a_green_run_recorded_runs_no_check() {
    let workspace = Workspace::from_fixture("empty");
    let stand_in = hook_stand_in(&workspace);
    let (first, head) = two_commits(&workspace);
    let common = workspace.git(&["rev-parse", "--path-format=absolute", "--git-common-dir"]);
    let tree = workspace.git(&["rev-parse", "HEAD^{tree}"]);
    std::fs::write(Path::new(common.trim()).join("goodfetch-quick-green"), tree).unwrap();

    let run = pre_push(
        &workspace,
        &stand_in,
        &format!("refs/heads/ticket {head} refs/heads/ticket {first}\n"),
    );

    assert_eq!(run.code, Some(0), "{}", run.stderr);
    assert!(run.calls.is_empty(), "{:?}", run.calls);
    assert!(run.stdout.contains("already passed"), "{}", run.stdout);
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
    assert_eq!(run.calls, [format!("quick base={first}")]);
}

/// A new remote branch holds no commit, so `just quick` takes the branch's
/// own base: its upstream off `main`, else `origin/main`.
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
    assert_eq!(run.calls, ["quick base="]);
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
            "floor-guard",
            "commit-leaks",
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
#[case::compiler_cache("compiler-cache")]
#[case::floor_guard("floor-guard")]
#[case::commit_leaks("commit-leaks")]
#[case::lint("lint")]
fn a_red_step_still_runs_every_later_step_and_the_summary_names_it(#[case] failing: &str) {
    let workspace = Workspace::from_fixture("empty");
    let stand_in = stand_ins(&workspace);
    selection(&stand_in, "quick");
    std::fs::write(stand_in.join("failing"), failing).unwrap();

    let run = quick(&workspace, &stand_in, &[]);

    assert_eq!(run.code, Some(1), "{}", run.stderr);
    assert_eq!(
        run.calls,
        [
            "compiler-cache",
            "floor-guard",
            "commit-leaks",
            "lint",
            "test-affected quick --local",
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
            "floor-guard",
            "commit-leaks",
            "_skips-report"
        ]
    );
    assert!(run.stderr.contains("xtask affected"), "{}", run.stderr);
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
