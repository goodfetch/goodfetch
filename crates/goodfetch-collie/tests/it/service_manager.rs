//! `daemon install` for real, with the user's own service manager (spec
//! #213's Seam 1; ADRs 0024, 0031, 0059 and 0098): a launch agent on macOS
//! and a systemd user unit on Linux, each started by the first connection
//! to its socket, and a logon task on Windows. The service manager is
//! shared by the whole user, so these tests run in a nextest test group,
//! each service named by its test's own Goodfetch home, and each test
//! removes its service when it ends. They need the user's service manager,
//! which `ci/os-services.sh` checks: on Linux in the Pre-merge suite, on
//! macOS and Windows in the Release suite only (TESTING.md, Storage,
//! network and OS services).
#![expect(
    clippy::disallowed_methods,
    reason = "the tests read the user's session from their own process, and the service's files themselves"
)]

#[cfg(unix)]
use std::path::Path;
use std::path::PathBuf;

use goodfetch_test_support::{Cli, TestEnv, cli};
#[cfg(unix)]
use goodfetch_test_support::{ConsoleState, DaemonEndpoint, RuntimeFile};
use serde_json::{Value, json};

#[cfg(unix)]
use crate::daemon::RawClient;
use crate::daemon::{short_idle, validate};

const COLLIE: &str = env!("CARGO_BIN_EXE_goodfetch-collie");

/// This commit's Product version, whose major folder is `0.0`.
#[cfg(unix)]
const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The variables through which a command reaches the user's own service
/// manager, from the test's process, over the test's isolated ones: where
/// the user's agents and units are read from, the session's bus, and the
/// programs' `PATH`.
const USER_SESSION: [&str; 7] = [
    "HOME",
    "PATH",
    "XDG_RUNTIME_DIR",
    "DBUS_SESSION_BUS_ADDRESS",
    "USERNAME",
    "USERDOMAIN",
    "SystemRoot",
];

/// A run of `executable` in the user's own session, Goodfetch home still
/// the test's.
pub(crate) fn in_session<'env>(env: &'env TestEnv, executable: &str) -> Cli<'env> {
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

pub(crate) fn json_in_session(env: &TestEnv, executable: &str, arguments: &[&str]) -> Value {
    let output = in_session(env, executable).args(arguments).run().unwrap();
    assert_eq!(output.exit, Some(0), "{}", output.stderr);
    serde_json::from_str(&output.stdout).unwrap()
}

/// Removes the test home's service when the test ends, passing or failing.
pub(crate) struct RemovesTheService<'env>(pub(crate) &'env TestEnv);

impl Drop for RemovesTheService<'_> {
    fn drop(&mut self) {
        let _ = in_session(self.0, COLLIE)
            .args(["daemon", "uninstall"])
            .run();
    }
}

/// What a Client of the installed service's socket needs: the socket and
/// the persistent token.
#[cfg(unix)]
pub(crate) fn service_reach(env: &TestEnv, service: &Value) -> RuntimeFile {
    let token = std::fs::read_to_string(env.home().join("0.0").join("daemon.token")).unwrap();
    RuntimeFile {
        endpoint: DaemonEndpoint::Unix(PathBuf::from(service["socket"].as_str().unwrap())),
        token,
        version: VERSION.to_owned(),
        pid: 0,
        console: ConsoleState::Absent,
    }
}

#[cfg(unix)]
fn mode(path: &Path) -> u32 {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::metadata(path).unwrap().permissions().mode() & 0o777
}

/// `daemon install` registers the major's service, whose socket the
/// service manager listens on: the first connection, with the persistent
/// token from its `0600` file, starts the Daemon, kept alive past a short
/// idle timeout, from the versioned copy the service names; `daemon
/// uninstall` stops it and removes every file the service had.
///
/// Covers: story:3-21, story:3-18, command:daemon.install, command:daemon.uninstall, command:daemon.status
#[cfg(unix)]
#[test]
fn the_first_connection_to_the_installed_services_socket_starts_the_daemon() {
    let env = TestEnv::new();
    short_idle(&env, 300);
    let _removes = RemovesTheService(&env);

    let installed = json_in_session(&env, COLLIE, &["daemon", "install", "--json"]);
    let service = installed["service"].clone();
    let before = json_in_session(&env, COLLIE, &["daemon", "status", "--json"]);
    let mut client = RawClient::opened(&service_reach(&env, &service), "collie");
    let status = client.call(1, "daemon.status", &json!({}));
    let found = RuntimeFile::read(&RuntimeFile::path(env.home(), VERSION)).unwrap();
    drop(client);
    let token_mode = mode(&env.home().join("0.0").join("daemon.token"));
    let uninstalled = json_in_session(&env, COLLIE, &["daemon", "uninstall", "--json"]);
    let after = json_in_session(&env, COLLIE, &["daemon", "status", "--json"]);

    validate(&installed, "DaemonStatus");
    validate(&uninstalled, "DaemonUninstallOutput");
    let files: Vec<bool> = service["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|file| Path::new(file.as_str().unwrap()).exists())
        .collect();
    assert_eq!(
        (
            before["pid"].clone(),
            before["service"] == service,
            status["result"]["product"].clone(),
            status["result"]["kept_alive"].clone(),
            status["result"]["pid"] == json!(found.pid),
            token_mode,
            Path::new(service["executable"].as_str().unwrap())
                .starts_with(env.home().join("browsers")),
            uninstalled["removed"] == service,
            files,
            after["service"].clone(),
            after["pid"].clone(),
            env.home().join("0.0").join("daemon.token").exists(),
        ),
        (
            Value::Null,
            true,
            json!("collie"),
            json!(true),
            true,
            0o600,
            true,
            true,
            vec![false; service["files"].as_array().unwrap().len()],
            Value::Null,
            Value::Null,
            false,
        )
    );
}

/// With the service installed, the service manager also listens on the
/// Console's address, a port `daemon install` chose for the test home's
/// port 0: the first request to the Console there starts the Daemon, which
/// answers it with the Console's page, so a bookmark always works.
///
/// Covers: story:12-7, command:daemon.install, command:daemon.status
#[cfg(unix)]
#[test]
fn the_first_request_to_the_consoles_address_starts_the_daemon() {
    let env = TestEnv::new();
    let _removes = RemovesTheService(&env);
    let installed = json_in_session(&env, COLLIE, &["daemon", "install", "--json"]);
    let console = installed["service"]["console"]
        .as_str()
        .expect("the service listens on the Console's address")
        .to_owned();
    let before = json_in_session(&env, COLLIE, &["daemon", "status", "--json"]);

    let page = crate::sign_in::agent()
        .get(format!("http://{console}/"))
        .call()
        .unwrap();

    let after = json_in_session(&env, COLLIE, &["daemon", "status", "--json"]);
    assert_eq!(
        (
            before["pid"].clone(),
            page.status().as_u16(),
            after["pid"].is_u64(),
            after["console_url"].clone(),
        ),
        (Value::Null, 200, true, json!(format!("http://{console}/")),)
    );
}

/// systemd re-reads each listening socket's path at every reload of the
/// user's manager, which any program may ask for; the service's two
/// sockets still listen after one, though the test home's path holds a
/// character systemd escapes (#1781): the first connection to the
/// service's socket and the first request to the Console both reach the
/// Daemon. That socket's folder, of its own then, goes at `daemon
/// uninstall`.
///
/// Covers: story:3-21, story:12-7, command:daemon.uninstall
#[cfg(target_os = "linux")]
#[test]
fn a_reload_of_the_users_systemd_keeps_both_of_the_services_sockets() {
    let env = TestEnv::new();
    let _removes = RemovesTheService(&env);
    let installed = json_in_session(&env, COLLIE, &["daemon", "install", "--json"]);
    let console = installed["service"]["console"]
        .as_str()
        .expect("the service listens on the Console's address")
        .to_owned();

    let reloaded = std::process::Command::new("systemctl")
        .args(["--user", "daemon-reload"])
        .status()
        .unwrap();
    let mut client = RawClient::opened(&service_reach(&env, &installed["service"]), "collie");
    let status = client.call(1, "daemon.status", &json!({}));
    let page = crate::sign_in::agent()
        .get(format!("http://{console}/"))
        .call()
        .unwrap();
    drop(client);
    json_in_session(&env, COLLIE, &["daemon", "uninstall", "--json"]);
    let socket = PathBuf::from(installed["service"]["socket"].as_str().unwrap());

    assert_eq!(
        (
            reloaded.success(),
            status["result"]["product"].clone(),
            page.status().as_u16(),
            socket.parent().unwrap().exists(),
        ),
        (true, json!("collie"), 200, false)
    );
}

/// A rotation stops the Daemon that took the old token: a Client with the
/// old token is closed unanswered, and one with the new token starts the
/// next Daemon.
///
/// Covers: story:3-21, command:daemon.rotate-token
#[cfg(unix)]
#[test]
fn after_a_rotation_only_the_new_token_opens_a_connection() {
    let env = TestEnv::new();
    short_idle(&env, 60_000);
    let _removes = RemovesTheService(&env);
    let installed = json_in_session(&env, COLLIE, &["daemon", "install", "--json"]);
    let old = service_reach(&env, &installed["service"]);
    drop(RawClient::opened(&old, "collie"));

    let rotated = json_in_session(&env, COLLIE, &["daemon", "rotate-token", "--json"]);
    let refused = RawClient::refused(&old);
    let new = service_reach(&env, &installed["service"]);
    let mut client = RawClient::opened(&new, "collie");
    let status = client.call(1, "daemon.status", &json!({}));

    validate(&rotated, "TokenRotated");
    assert_eq!(
        (
            refused,
            new.token == old.token,
            status["result"]["product"].clone(),
        ),
        (true, false, json!("collie"))
    );
}

/// On Windows the logon task starts the Daemon at logon, and the
/// installed service keeps it running past its idle timeout.
///
/// Covers: story:3-21, command:daemon.install, command:daemon.uninstall
#[cfg(windows)]
#[test]
fn the_installed_logon_task_keeps_the_daemon_running() {
    let env = TestEnv::new();
    short_idle(&env, 300);
    let _removes = RemovesTheService(&env);

    let installed = json_in_session(&env, COLLIE, &["daemon", "install", "--json"]);
    let started = json_in_session(&env, COLLIE, &["daemon", "start", "--json"]);
    let uninstalled = json_in_session(&env, COLLIE, &["daemon", "uninstall", "--json"]);
    let after = json_in_session(&env, COLLIE, &["daemon", "status", "--json"]);

    validate(&installed, "DaemonStatus");
    assert_eq!(
        (
            installed["service"]["manager"].clone(),
            started["pid"].is_u64(),
            uninstalled["removed"] == installed["service"],
            after["pid"].clone(),
        ),
        (json!("task_scheduler"), true, true, Value::Null)
    );
}
