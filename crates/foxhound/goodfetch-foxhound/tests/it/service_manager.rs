//! The installed service starts its major's pick, whichever executable
//! installed it (spec #213's Seam 1; ADRs 0024 and 0098): a service Collie's
//! executable installed starts Foxhound's Daemon once a Foxhound of the
//! major counts as installed. A real install with the user's own service
//! manager, in the nextest test group of `service_manager::`, on Linux in
//! the Pre-merge suite and on macOS in the Release suite only (TESTING.md,
//! Storage, network and OS services). Collie's executable comes from the
//! test tool manifest.
#![cfg(unix)]
#![expect(
    clippy::disallowed_methods,
    reason = "the tests read the user's session from their own process, and the service's token themselves"
)]
#![expect(
    clippy::disallowed_types,
    reason = "a test's own Client reaches the service's socket from outside the engine (ADR 0060)"
)]

use std::io::{BufRead as _, BufReader, Write as _};
use std::path::{Path, PathBuf};

use goodfetch_protocol::catalog::PROTOCOL_VERSION;
use goodfetch_protocol::home::DaemonFile;
use goodfetch_test_client::HomeLayout;
use goodfetch_test_support::{Cli, TestEnv, Tool, cli, tools};
use serde_json::{Value, json};

const FOXHOUND: &str = env!("CARGO_BIN_EXE_goodfetch-foxhound");

const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The variables through which a command reaches the user's own service
/// manager, from the test's process, over the test's isolated ones.
const USER_SESSION: [&str; 4] = [
    "HOME",
    "PATH",
    "XDG_RUNTIME_DIR",
    "DBUS_SESSION_BUS_ADDRESS",
];

fn in_session<'env>(env: &'env TestEnv, executable: &Path) -> Cli<'env> {
    let mut command = cli(env, executable);
    for name in USER_SESSION {
        if let Some(value) = std::env::var_os(name) {
            command = command.env(name, value);
        }
    }
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")));
    if let Some(config) = config {
        command = command.env("XDG_CONFIG_HOME", config);
    }
    command
}

/// A `--json` run of `executable` in the user's session, its document;
/// a failure shows the run's stdout too, where `--json` prints its reason.
fn json_in_session(env: &TestEnv, executable: &Path, arguments: &[&str]) -> Value {
    let output = in_session(env, executable).args(arguments).run().unwrap();
    assert_eq!(output.exit, Some(0), "{output}");
    serde_json::from_str(&output.stdout).unwrap()
}

fn collie() -> PathBuf {
    tools().unwrap().path(Tool::Collie).unwrap().to_owned()
}

/// Removes the test home's service when the test ends.
struct RemovesTheService<'env>(&'env TestEnv);

impl Drop for RemovesTheService<'_> {
    fn drop(&mut self) {
        let _ = in_session(self.0, &collie())
            .args(["daemon", "uninstall"])
            .run();
    }
}

/// Opens a Connection to the service's socket with its persistent token,
/// which starts the Daemon, and asks it `daemon.status`.
fn status_through(env: &TestEnv, socket: &Path) -> Value {
    let token =
        std::fs::read_to_string(HomeLayout::of(env.home()).daemon_file(DaemonFile::Token)).unwrap();
    let stream = std::os::unix::net::UnixStream::connect(socket).unwrap();
    let mut writer = stream.try_clone().unwrap();
    let mut reader = BufReader::new(stream);
    let mut call = |id: i64, method: &str, params: Value| {
        let request = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        writeln!(writer, "{request}").unwrap();
        writer.flush().unwrap();
        loop {
            let mut line = String::new();
            assert!(
                reader.read_line(&mut line).unwrap() > 0,
                "the Daemon closed the Connection"
            );
            let message: Value = serde_json::from_str(&line).unwrap();
            if message["id"] == json!(id) {
                return message;
            }
        }
    };
    let hello = call(
        0,
        "hello",
        json!({
            "protocol_version": PROTOCOL_VERSION.to_string(),
            "surface": "sdk_typescript",
            "token": token,
            "needs": "collie",
            "product_version": VERSION,
        }),
    );
    assert!(hello.get("result").is_some(), "{hello}");
    call(1, "daemon.status", json!({}))["result"].clone()
}

/// Covers: story:3-21, story:3-137, command:daemon.install
#[test]
fn a_service_collie_installed_starts_foxhound_once_foxhound_counts_as_installed() {
    let env = TestEnv::new();
    let _removes = RemovesTheService(&env);
    let installed = json_in_session(&env, &collie(), &["daemon", "install", "--json"]);
    // Foxhound's executable counts as installed once a command of it ran.
    json_in_session(&env, FOXHOUND.as_ref(), &["daemon", "status", "--json"]);

    let status = status_through(
        &env,
        Path::new(installed["service"]["socket"].as_str().unwrap()),
    );

    assert_eq!(
        (
            Path::new(installed["service"]["executable"].as_str().unwrap())
                .file_name()
                .unwrap()
                .to_string_lossy()
                .into_owned(),
            status["product"].clone(),
            status["kept_alive"].clone(),
        ),
        (
            collie().file_name().unwrap().to_string_lossy().into_owned(),
            json!("foxhound"),
            json!(true),
        )
    );
}
