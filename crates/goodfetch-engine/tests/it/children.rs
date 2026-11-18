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
