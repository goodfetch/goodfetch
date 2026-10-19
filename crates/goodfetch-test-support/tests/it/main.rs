//! goodfetch-test-support's integration tests (Seam B, Pre-merge): the
//! crate's interface with real child processes (seeded kills and a failed
//! test's Observation log among them), and `goodfetch-test-env`'s command
//! line and JSON description.
#![expect(
    clippy::disallowed_methods,
    reason = "these tests start processes, read files and wait outside any async worker"
)]
#![expect(
    clippy::unwrap_used,
    reason = "test code, where STYLE.md allows unwrap; Clippy's test exemption does not reach integration-test helpers"
)]

mod endpoints;
mod observation_log;
mod probe;
mod proving_ground;
mod reported;
mod seed;
mod seeded_kill;
mod ssrf_table;
mod strict_stdout;
mod teardown;
mod test_env;
mod tools;

use std::path::PathBuf;

use goodfetch_test_support::{Process, TestEnv};

/// This commit's `goodfetch-test-env`.
const TEST_ENV: &str = env!("CARGO_BIN_EXE_goodfetch-test-env");

/// Writes a test tool manifest listing `goodfetch-test-env` into the test's
/// folder, and returns its path.
fn manifest(env: &TestEnv) -> PathBuf {
    let path = env.folder().join("tools.json");
    let manifest = serde_json::json!({ "test_env": TEST_ENV });
    std::fs::write(&path, manifest.to_string()).unwrap();
    path
}

/// A command that runs one probe of `probe.rs` as a child test, whose
/// temporary folders land in this test's folder.
fn probe(env: &TestEnv, name: &str) -> std::process::Command {
    let mut command = env.process_command(std::env::current_exe().unwrap());
    command
        .args(["--exact", name, "--ignored", "--nocapture"])
        .env("TMPDIR", env.folder())
        .env("TMP", env.folder())
        .env("TEMP", env.folder());
    command
}

/// Starts the OS's own shell on a script: `sh` on Unix, `cmd` on Windows.
fn shell(env: &TestEnv, unix: &str, windows: &str) -> Process {
    let mut command = if cfg!(windows) {
        let mut command = env.process_command(&env.env()["ComSpec"]);
        command.args(["/C", windows]);
        command
    } else {
        let mut command = env.process_command("/bin/sh");
        command.args(["-c", unix]);
        command
    };
    env.spawn(&mut command).unwrap()
}
