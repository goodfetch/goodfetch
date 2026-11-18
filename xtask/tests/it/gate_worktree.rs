//! The lock of a gate's worktree (`ci/gate-worktree.sh`, ADR 0135), which
//! `just merge`'s gate, the backstop, the test-health runs and the nightly
//! fuzzing take before they build there: it has one owner at a time, and a
//! lock whose owner died is taken without a manual step (#2334). Each test
//! sources the real script in a `bash` of its own. The local gate is
//! repository tooling, so the module declares no Feature.
#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::process::Stdio;

use rstest::rstest;

use crate::affected::repository;
use crate::merge_pr::{NO_PROCESS, gate_lock, until};
use crate::workspace::{Started, Workspace};

fn script() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../ci/gate-worktree.sh")
}

/// Run `n` of the runs that take the gate's lock the moment the workspace's
/// `go` exists, which each watches without a pause, so they start within
/// microseconds of each other; `ready-<n>` appears once it watches. It
/// appends `held` to `outcomes` and lives on while it holds the lock, or
/// `ended <status>` when it ends without it, its error output in `said-<n>`.
fn taker(workspace: &Workspace, n: usize) -> Started {
    let file = |name: &str| workspace.path().join(name).display().to_string();
    let run = format!(
        "source '{script}'\n\
         trap 'echo \"ended $?\" >>\"{outcomes}\"' EXIT\n\
         : >'{ready}'\n\
         until [ -e '{go}' ]; do :; done\n\
         gate_worktree_lock\n\
         trap - EXIT\n\
         echo held >>\"{outcomes}\"\n\
         exec sleep 600\n",
        script = script().display(),
        outcomes = file("outcomes"),
        ready = file(&format!("ready-{n}")),
        go = file("go"),
    );
    Started::spawn(
        workspace
            .command("bash")
            .args(["-c", &run])
            .stdout(Stdio::null())
            .stderr(std::fs::File::create(file(&format!("said-{n}"))).unwrap()),
    )
}

/// Two gates that take the lock at the same moment never both hold it: a
/// lock folder made with `mkdir`, before its owner was recorded, showed the
/// second run no owner, which it removed as stale before both went on. One
/// holds it; the other is refused with the holder's process.
#[test]
fn two_gates_that_take_the_lock_at_once_never_both_hold_it() {
    let workspace = repository();
    let _first = taker(&workspace, 1);
    let _second = taker(&workspace, 2);
    until("both runs watching", || {
        (1..=2).all(|n| workspace.path().join(format!("ready-{n}")).exists())
    });

    std::fs::write(workspace.path().join("go"), "").unwrap();
    let outcomes = workspace.path().join("outcomes");
    until("both runs' outcomes", || {
        std::fs::read_to_string(&outcomes).is_ok_and(|text| text.lines().count() == 2)
    });

    let mut ended: Vec<String> = std::fs::read_to_string(&outcomes)
        .unwrap()
        .lines()
        .map(str::to_owned)
        .collect();
    ended.sort();
    assert_eq!(ended, ["ended 1", "held"]);
    let holder = std::fs::read_to_string(gate_lock(&workspace).join("pid")).unwrap();
    let refusal = (1..=2)
        .map(|n| std::fs::read_to_string(workspace.path().join(format!("said-{n}"))).unwrap())
        .collect::<String>();
    assert!(
        refusal.contains(&format!("another gate (process {})", holder.trim())),
        "{refusal}"
    );
}

/// A lock whose owner died is the next run's to take: one that names a
/// process that no longer runs, and one left with no owner at all, as a run
/// killed while it removed a dead owner's lock leaves it.
#[rstest]
#[case::its_owner_ended(Some(NO_PROCESS))]
#[case::it_names_no_owner(None)]
fn a_lock_whose_owner_died_is_taken_without_a_manual_step(#[case] owner: Option<&str>) {
    let workspace = repository();
    std::fs::create_dir_all(gate_lock(&workspace)).unwrap();
    if let Some(owner) = owner {
        std::fs::write(gate_lock(&workspace).join("pid"), owner).unwrap();
    }
    let run = format!(
        "source '{}'; gate_worktree_lock; echo $$",
        script().display()
    );

    let output = workspace
        .command("bash")
        .args(["-c", &run])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        std::fs::read_to_string(gate_lock(&workspace).join("pid")).unwrap(),
        String::from_utf8(output.stdout).unwrap()
    );
}
