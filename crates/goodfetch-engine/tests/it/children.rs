//! The engine's child processes (ADR 0024; spec #213's Seam 3, the
//! engine's public interface): each starts in a process group of its own on
//! macOS and Linux, and in the engine's kill-on-close Job Object on
//! Windows, and the engine's stop kills it with everything it started.

use std::process::Stdio;
use std::time::Duration;

use goodfetch_engine::children::Children;

/// How long a child would live on its own: far past the test.
const LIFE: &str = "30";

/// How long the test waits for a killed child to end.
const DEADLINE: Duration = Duration::from_secs(10);

fn runtime() -> tokio::runtime::Runtime {
    goodfetch_runtime::current_thread()
        .enable_all()
        .build()
        .unwrap()
}

#[cfg(unix)]
fn sleeper() -> tokio::process::Command {
    let mut command = tokio::process::Command::new("/bin/sh");
    command
        .args(["-c", &format!("sleep {LIFE} & sleep {LIFE}; wait")])
        .stdin(Stdio::null())
        .stdout(Stdio::null());
    command
}

#[cfg(windows)]
fn sleeper() -> tokio::process::Command {
    let mut command = tokio::process::Command::new("powershell.exe");
    command
        .args([
            "-NoProfile",
            "-Command",
            &format!("Start-Sleep -Seconds {LIFE}"),
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null());
    command
}

/// A child that starts nothing of its own, so that only the engine's death
/// can end it before its [`LIFE`]: `exec` leaves no shell behind it on
/// Linux.
#[cfg(target_os = "linux")]
fn lone_sleeper() -> tokio::process::Command {
    let mut command = tokio::process::Command::new("/bin/sh");
    command
        .args(["-c", &format!("exec sleep {LIFE}")])
        .stdin(Stdio::null())
        .stdout(Stdio::null());
    command
}

#[cfg(windows)]
fn lone_sleeper() -> tokio::process::Command {
    sleeper()
}

/// The stand-in engine that [`a_killed_engines_children_end_with_it`] starts.
#[cfg(any(target_os = "linux", windows))]
const STAND_IN: &str = "children::a_stand_in_engine_holds_a_child_until_it_is_killed";

/// The file the stand-in engine writes its child's process ID to.
#[cfg(any(target_os = "linux", windows))]
const ID_FILE: &str = "CHILDREN_CHILD_ID_FILE";

/// Whether the process `id` ended: it is gone, or a zombie no one reaped
/// yet, which runs nothing.
#[cfg(target_os = "linux")]
#[expect(
    clippy::disallowed_methods,
    reason = "the test reads the process's state from /proc, outside the engine"
)]
fn ended(id: u32) -> bool {
    std::fs::read_to_string(format!("/proc/{id}/stat")).map_or(true, |stat| {
        stat.rsplit_once(") ")
            .is_some_and(|(_, rest)| rest.starts_with(['Z', 'X']))
    })
}

/// Whether the process `id` ended: `tasklist` lists no process of that ID.
#[cfg(windows)]
#[expect(
    clippy::disallowed_methods,
    reason = "the test asks `tasklist`, outside the engine, whether the child still runs"
)]
fn ended(id: u32) -> bool {
    let output = std::process::Command::new("tasklist")
        .args(["/FI", &format!("PID eq {id}"), "/NH"])
        .output()
        .unwrap();
    let id = id.to_string();
    !String::from_utf8_lossy(&output.stdout)
        .split_whitespace()
        .any(|word| word == id)
}

/// The process group of the process `id`.
#[cfg(unix)]
#[expect(
    clippy::disallowed_methods,
    reason = "the test asks `ps`, outside the engine, which group the child leads"
)]
fn group_of(id: u32) -> i64 {
    let output = std::process::Command::new("ps")
        .args(["-o", "pgid=", "-p", &id.to_string()])
        .output()
        .unwrap();
    String::from_utf8(output.stdout)
        .unwrap()
        .trim()
        .parse()
        .unwrap()
}

/// Covers: story:3-15
#[cfg(unix)]
#[test]
fn a_child_of_the_engine_leads_a_process_group_of_its_own() {
    runtime().block_on(async {
        let mut children = Children::new().unwrap();
        let child = children.spawn(sleeper()).unwrap();
        let id = child.id().unwrap();

        let group = group_of(id);

        children.kill_all();
        assert_eq!(group, i64::from(id));
    });
}

/// Covers: story:3-15
#[test]
fn the_engines_stop_kills_its_children() {
    runtime().block_on(async {
        let mut children = Children::new().unwrap();
        let mut child = children.spawn(sleeper()).unwrap();

        drop(children);
        let ended = tokio::time::timeout(DEADLINE, child.wait()).await;

        let status = ended
            .expect("the child outlived the engine's stop")
            .unwrap();
        assert!(!status.success(), "{status}");
    });
}

/// The engine's death, with no stop of its own, kills its children through
/// the kernel: their parent-death signal on Linux, and on Windows the
/// engine's kill-on-close Job Object, whose only handle closes with it. A
/// stand-in engine holding a child is killed outright (`kill -9`,
/// `TerminateProcess`), and the child ends within the deadline, far before
/// its own life would.
///
/// Covers: story:3-15
#[cfg(any(target_os = "linux", windows))]
#[test]
#[expect(
    clippy::disallowed_methods,
    reason = "the test reads the file the stand-in engine writes, outside the engine"
)]
fn a_killed_engines_children_end_with_it() {
    let env = goodfetch_test_support::TestEnv::new();
    let file = env.folder().join("child-id");
    let engine = env
        .spawn(
            env.process_command(std::env::current_exe().unwrap())
                .args([STAND_IN, "--exact", "--include-ignored"])
                .env(ID_FILE, &file),
        )
        .unwrap();

    runtime().block_on(async {
        let id = tokio::time::timeout(DEADLINE, async {
            loop {
                if let Ok(id) = tokio::fs::read_to_string(&file).await {
                    break id.parse::<u32>().unwrap();
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
        .await
        .expect("the stand-in engine started no child");
        engine.child().kill().unwrap();
        engine.child().wait().unwrap();

        let gone = tokio::time::timeout(DEADLINE, async {
            while !ended(id) {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
        .await;

        assert!(gone.is_ok(), "the child {id} outlived its killed engine");
    });
}

/// The stand-in engine of [`a_killed_engines_children_end_with_it`]: it
/// starts a child through [`Children`], writes the child's process ID to
/// the file [`ID_FILE`] names, and waits past the test's deadline.
#[cfg(any(target_os = "linux", windows))]
#[test]
#[ignore = "run as a child by children::a_killed_engines_children_end_with_it"]
#[expect(
    clippy::disallowed_methods,
    reason = "the stand-in's main is its test: it reads the file its parent names"
)]
fn a_stand_in_engine_holds_a_child_until_it_is_killed() {
    let file = std::path::PathBuf::from(std::env::var_os(ID_FILE).unwrap());
    runtime().block_on(async {
        let mut children = Children::new().unwrap();
        let child = children.spawn(lone_sleeper()).unwrap();
        let written = file.with_extension("partial");
        tokio::fs::write(&written, child.id().unwrap().to_string())
            .await
            .unwrap();
        tokio::fs::rename(&written, &file).await.unwrap();

        tokio::time::sleep(Duration::from_secs(LIFE.parse().unwrap())).await;
        drop(children);
    });
}
