//! `just merge` (`ci/merge-pr.sh`, AGENTS.md's Merging; TESTING.md's
//! Commands) around its gate: it refuses to start one while the file system
//! of the gate's `target/` is short of space (#1822), keeps a failing gate's
//! log and test results in the repository's common folder (#2087), and
//! sweeps the gate's `target/` after each gate, still holding its lock. `gh`,
//! `cargo` and `just` are stand-ins that record each call. The local gate is
//! repository tooling, so the module declares no Feature.
#![cfg(unix)]

use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};

use crate::affected::{commit_all, repository};
use crate::workspace::Workspace;

const NUMBER: &str = "7";

/// Stand-ins for `gh`, `cargo` and `just` that append each call to `calls`.
/// `gh pr view` shows an open pull request at the head of `change`; `cargo
/// run … affected` prints `gate=quick`; `just pre-merge` writes a JUnit
/// file and, while `failing` exists, fails with status 3.
fn stand_ins(workspace: &Workspace, head: &str) -> PathBuf {
    let folder = workspace.path().join("stand-in");
    std::fs::create_dir_all(&folder).unwrap();
    let calls = folder.join("calls");
    let scripts = [
        (
            "gh",
            format!(
                "#!/bin/sh\necho \"gh $*\" >>\"{calls}\"\n\
                 if [ \"$1 $2\" = \"pr view\" ]; then printf 'OPEN\\tfalse\\t{head}\\tmain\\tA change\\n'; fi\n",
                calls = calls.display(),
            ),
        ),
        (
            "cargo",
            format!(
                "#!/bin/sh\necho \"cargo $* in $PWD\" >>\"{calls}\"\n\
                 case \" $* \" in *\" affected \"*) echo gate=quick ;; esac\n",
                calls = calls.display(),
            ),
        ),
        (
            "just",
            format!(
                "#!/bin/sh\necho \"just $*\" >>\"{calls}\"\n\
                 if [ \"$1\" = pre-merge ]; then\n\
                   mkdir -p target/nextest/quick && echo '<testsuites/>' >target/nextest/quick/junit.xml\n\
                   if [ -f \"{failing}\" ]; then echo 'a test failed'; exit 3; fi\n\
                 fi\n",
                calls = calls.display(),
                failing = folder.join("failing").display(),
            ),
        ),
    ];
    for (name, script) in scripts {
        let path = folder.join(name);
        std::fs::write(&path, script).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    folder
}

struct Run {
    code: Option<i32>,
    stdout: String,
    stderr: String,
    calls: Vec<String>,
}

/// A repository whose `origin` is itself, with pull request #7 at a commit
/// on `change` that contains `main`; returns it with that head.
fn pull_request() -> (Workspace, String) {
    let workspace = repository();
    let path = workspace.path().display().to_string();
    workspace.git(&["remote", "add", "origin", &path]);
    workspace.write("change.txt", b"a change\n");
    commit_all(&workspace, "A change");
    let head = workspace.git(&["rev-parse", "HEAD"]).trim().to_owned();
    workspace.git(&["update-ref", &format!("refs/pull/{NUMBER}/head"), &head]);
    (workspace, head)
}

/// Runs the repository's `ci/merge-pr.sh` on #7, the stand-ins first on
/// `PATH`, with these variables added.
fn merge(workspace: &Workspace, stand_in: &Path, variables: &[(&str, &str)]) -> Run {
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../ci/merge-pr.sh");
    let output = workspace
        .command(script)
        .arg(NUMBER)
        .env("PATH", format!("{}:/usr/bin:/bin", stand_in.display()))
        .env("TMPDIR", workspace.path())
        .env(
            "GOODFETCH_GATE_BACKSTOP_LOG",
            workspace.path().join("no-backstop"),
        )
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .envs(variables.iter().copied())
        .output()
        .unwrap();
    Run {
        code: output.status.code(),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        calls: std::fs::read_to_string(stand_in.join("calls"))
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect(),
    }
}

fn gate_worktree(workspace: &Workspace) -> PathBuf {
    workspace.path().join(".claude/worktrees/merge-gate")
}

/// The index of the first call that starts with `prefix`.
fn call(run: &Run, prefix: &str) -> usize {
    run.calls
        .iter()
        .position(|call| call.starts_with(prefix))
        .unwrap_or_else(|| panic!("no `{prefix}` call in {:?}", run.calls))
}

#[test]
fn a_gate_refuses_to_start_while_its_target_file_system_is_short_of_space() {
    let (workspace, head) = pull_request();
    let stand_in = stand_ins(&workspace, &head);

    let run = merge(
        &workspace,
        &stand_in,
        &[("GOODFETCH_GATE_MIN_FREE_GB", "1000000000")],
    );

    assert_eq!(run.code, Some(1), "{}{}", run.stdout, run.stderr);
    assert!(
        run.stderr
            .contains("GB free, under the 1000000000 GB a gate's builds need"),
        "{}",
        run.stderr
    );
    assert!(
        !run.calls.iter().any(|call| call.starts_with("just ")),
        "{:?}",
        run.calls
    );
    assert!(
        !run.calls.iter().any(|call| call.contains("sweep-target")),
        "{:?}",
        run.calls
    );
    assert!(
        !workspace
            .path()
            .join(".claude/worktrees/merge-gate.lock")
            .exists()
    );
}

/// A green gate merges, then sweeps the gate's `target/` before it lets
/// the lock go: units unread over the gate's hours, rounded up (none for
/// the stand-ins' instant gate), and the 25 hours `relatime` may lag.
#[test]
fn after_a_green_gate_the_target_is_swept_under_the_gates_lock() {
    let (workspace, head) = pull_request();
    let stand_in = stand_ins(&workspace, &head);

    let run = merge(
        &workspace,
        &stand_in,
        &[("GOODFETCH_GATE_MIN_FREE_GB", "0")],
    );

    assert_eq!(run.code, Some(0), "{}{}", run.stdout, run.stderr);
    let sweep = call(
        &run,
        "cargo run --quiet --locked --bin xtask -- sweep-target",
    );
    assert!(
        run.calls[sweep].ends_with(&format!(
            "sweep-target --unused-hours 25 in {}",
            gate_worktree(&workspace).display()
        )),
        "{}",
        run.calls[sweep]
    );
    assert!(call(&run, "just pre-merge quick") < sweep);
    assert!(call(&run, "gh pr merge") < sweep);
    assert!(
        !workspace
            .path()
            .join(".claude/worktrees/merge-gate.lock")
            .exists()
    );
}

#[test]
fn a_failing_gate_keeps_its_log_and_test_results_and_is_swept_too() {
    let (workspace, head) = pull_request();
    let stand_in = stand_ins(&workspace, &head);
    std::fs::write(stand_in.join("failing"), "").unwrap();

    let run = merge(
        &workspace,
        &stand_in,
        &[("GOODFETCH_GATE_MIN_FREE_GB", "0")],
    );

    assert_eq!(run.code, Some(3), "{}{}", run.stdout, run.stderr);
    assert!(
        !run.calls.iter().any(|call| call.starts_with("gh pr merge")),
        "{:?}",
        run.calls
    );
    call(
        &run,
        "cargo run --quiet --locked --bin xtask -- sweep-target",
    );
    let failures = workspace.path().join(".git/goodfetch-gate-failures");
    let kept: Vec<PathBuf> = std::fs::read_dir(&failures)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    assert_eq!(kept.len(), 1, "{kept:?}");
    let name = kept[0].file_name().unwrap().to_str().unwrap();
    assert!(
        name.ends_with(&format!("-pr{NUMBER}-{}", &head[..12])),
        "{name}"
    );
    assert!(
        std::fs::read_to_string(kept[0].join("gate.log"))
            .unwrap()
            .contains("a test failed"),
        "{name}"
    );
    assert!(kept[0].join("nextest/quick/junit.xml").is_file());
    assert!(
        run.stderr
            .contains(&format!("kept in {}", kept[0].display())),
        "{}",
        run.stderr
    );
}
