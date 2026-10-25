//! The Daemon through Collie's executable (spec #213's Seam 1): `daemon
//! start|stop|restart|status`, the runtime file and socket of the major
//! under the test's Goodfetch home, whose path holds a space and a
//! non-ASCII letter, two starts at once, the idle stop under a short
//! `daemon.idle_timeout`, a killed Daemon's runs and runtime file, the
//! endpoint table's Daemon socket column, and the CLI against the Scripted
//! engine run as a Daemon: a refused version, a dead PID and a Connection
//! dropped mid-command (ADRs 0024, 0067, 0069, 0073 and 0098).
#![expect(
    clippy::disallowed_methods,
    reason = "the tests start processes, read the Daemon's files and logs and write its configuration themselves"
)]
#![expect(
    clippy::disallowed_types,
    reason = "a test's own Client reaches the Daemon's socket from outside the engine (ADR 0060)"
)]
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use std::io::{BufRead as _, BufReader, Write as _};
use std::path::{Path, PathBuf};
use std::process::Stdio;

use goodfetch_protocol::catalog::PROTOCOL_VERSION;
use goodfetch_test_support::{
    Canaries, DaemonEndpoint, Endpoint, EndpointRunner, Hello, Listener, ListenerAddress,
    ListenerKind, Process, RuntimeFile, SnapshotValues, TestEnv, cli, engine_script,
    record_command, reported_address, snapshot_settings, tools,
};
use serde_json::{Value, json};

const COLLIE: &str = env!("CARGO_BIN_EXE_goodfetch-collie");

/// The committed JSON Schema bundle, which holds the `--json` outputs.
const BUNDLE: &str = include_str!("../../../goodfetch-protocol/generated/bundle.json");

/// This commit's Product version, whose major folder is `0.0`.
const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Lays out a user configuration with a short `daemon.idle_timeout`, so a
/// Daemon a test forgets never outlives it for long.
pub(crate) fn short_idle(env: &TestEnv, milliseconds: u64) {
    env.user_configuration(&format!("daemon.idle_timeout = {milliseconds}\n"));
}

/// Stops the test home's Daemon when the test ends, passing or failing.
pub(crate) struct StopsTheDaemon<'env> {
    env: &'env TestEnv,
    executable: PathBuf,
}

impl<'env> StopsTheDaemon<'env> {
    pub(crate) fn of(env: &'env TestEnv, executable: impl Into<PathBuf>) -> Self {
        Self {
            env,
            executable: executable.into(),
        }
    }
}

impl Drop for StopsTheDaemon<'_> {
    fn drop(&mut self) {
        let _ = cli(self.env, &self.executable)
            .args(["daemon", "stop"])
            .run();
    }
}

pub(crate) fn json_of(executable: &Path, env: &TestEnv, arguments: &[&str]) -> Value {
    let output = cli(env, executable).args(arguments).run().unwrap();
    assert_eq!(output.exit, Some(0), "{}", output.stderr);
    serde_json::from_str(&output.stdout).unwrap()
}

pub(crate) fn validate(output: &Value, definition: &str) {
    let bundle: Value = serde_json::from_str(BUNDLE).unwrap();
    let schema = json!({ "$ref": format!("#/$defs/{definition}"), "$defs": bundle["$defs"] });
    let validator = jsonschema::validator_for(&schema).unwrap();
    let errors: Vec<String> = validator
        .iter_errors(output)
        .map(|error| error.to_string())
        .collect();
    assert_eq!(errors, Vec::<String>::new());
}

/// The versioned copy of Collie's executable the Daemon runs from, in the
/// Browser cache of the test's home (ADR 0098).
fn installed_copy(env: &TestEnv) -> PathBuf {
    let list: Value = serde_json::from_slice(
        &std::fs::read(env.home().join("installed-executables.json")).unwrap(),
    )
    .unwrap();
    let copy = &list["executables"][0]["copy"];
    env.home()
        .join("browsers")
        .join("executable_copy")
        .join(copy["name"].as_str().unwrap())
        .join(copy["build"].as_str().unwrap())
        .join("files")
        .join(Path::new(COLLIE).file_name().unwrap())
}

pub(crate) fn runtime_file(env: &TestEnv) -> PathBuf {
    RuntimeFile::path(env.home(), VERSION)
}

/// Waits, without polling, until the Daemon of the test's home has
/// stopped: it holds its serving lock until then.
pub(crate) fn wait_until_stopped(env: &TestEnv) {
    let lock = std::fs::File::options()
        .create(true)
        .truncate(false)
        .write(true)
        .open(env.home().join("0.0").join("daemon.lock"))
        .unwrap();
    lock.lock().unwrap();
}

#[cfg(unix)]
fn mode(path: &Path) -> u32 {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::metadata(path).unwrap().permissions().mode() & 0o777
}

/// A test's own Client of the Daemon, over its socket or loopback TCP.
pub(crate) struct RawClient {
    reader: BufReader<Box<dyn std::io::Read>>,
    writer: Box<dyn std::io::Write>,
}

impl RawClient {
    /// Connects to the Daemon `found` names, and says `hello` needing
    /// `product`.
    pub(crate) fn opened(found: &RuntimeFile, product: &str) -> Self {
        Self::opened_at(&found.endpoint, &found.token, product)
    }

    /// Connects to `endpoint`, a Daemon's or an Attached engine's private
    /// endpoint, and says `hello` with `token`, needing `product`.
    pub(crate) fn opened_at(endpoint: &DaemonEndpoint, token: &str, product: &str) -> Self {
        let mut client = Self::connected(endpoint);
        let answer = client.call(
            0,
            "hello",
            &json!({
                "protocol_version": PROTOCOL_VERSION.to_string(),
                "surface": "sdk_typescript",
                "token": token,
                "needs": product,
                "product_version": VERSION,
            }),
        );
        assert!(answer.get("result").is_some(), "{answer}");
        client
    }

    /// Whether the Daemon `found` names closes, unanswered, a Connection
    /// whose `hello` carries `found`'s token.
    #[cfg(unix)]
    pub(crate) fn refused(found: &RuntimeFile) -> bool {
        let mut client = Self::connected(&found.endpoint);
        client.send(&json!({
            "jsonrpc": "2.0",
            "id": 0,
            "method": "hello",
            "params": {
                "protocol_version": PROTOCOL_VERSION.to_string(),
                "surface": "sdk_typescript",
                "token": found.token,
                "needs": "collie",
                "product_version": VERSION,
            },
        }));
        client.next().is_none()
    }

    /// Connects to `endpoint`, without a word.
    fn connected(endpoint: &DaemonEndpoint) -> Self {
        let (reader, writer): (Box<dyn std::io::Read>, Box<dyn std::io::Write>) = match endpoint {
            #[cfg(unix)]
            DaemonEndpoint::Unix(path) => {
                let stream = std::os::unix::net::UnixStream::connect(path).unwrap();
                (Box::new(stream.try_clone().unwrap()), Box::new(stream))
            }
            #[cfg(not(unix))]
            DaemonEndpoint::Unix(_) => unreachable!("a Daemon listens on TCP here"),
            DaemonEndpoint::Tcp(address) => {
                let stream = std::net::TcpStream::connect(address).unwrap();
                (Box::new(stream.try_clone().unwrap()), Box::new(stream))
            }
        };
        Self {
            reader: BufReader::new(reader),
            writer,
        }
    }

    fn send(&mut self, message: &Value) {
        writeln!(self.writer, "{message}").unwrap();
        self.writer.flush().unwrap();
    }

    /// The next message, or `None` once the Daemon closed the Connection.
    pub(crate) fn next(&mut self) -> Option<Value> {
        let mut line = String::new();
        let read = self.reader.read_line(&mut line).unwrap_or(0);
        (read > 0).then(|| serde_json::from_str(&line).unwrap())
    }

    /// Sends `message` unless the Daemon closed the Connection already.
    pub(crate) fn try_send(&mut self, message: &Value) {
        let _ = writeln!(self.writer, "{message}").and_then(|()| self.writer.flush());
    }

    pub(crate) fn call(&mut self, id: i64, method: &str, params: &Value) -> Value {
        let request = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        writeln!(self.writer, "{request}").unwrap();
        self.writer.flush().unwrap();
        loop {
            let mut line = String::new();
            assert!(
                self.reader.read_line(&mut line).unwrap() > 0,
                "the Daemon closed the Connection"
            );
            let message: Value = serde_json::from_str(&line).unwrap();
            if message["id"] == json!(id) {
                return message;
            }
        }
    }
}

/// Covers: story:3-11, story:3-12, story:3-20, story:3-22, story:3-139, command:daemon.start, command:daemon.status, command:daemon.stop, operation:daemon.status, operation:daemon.stop
#[test]
fn daemon_start_status_and_stop_run_one_daemon_of_the_major_under_goodfetch_home() {
    let env = TestEnv::new();
    short_idle(&env, 60_000);
    let _stops = StopsTheDaemon::of(&env, COLLIE);

    let started = json_of(COLLIE.as_ref(), &env, &["daemon", "start", "--json"]);
    let status = json_of(COLLIE.as_ref(), &env, &["daemon", "status", "--json"]);
    let found = RuntimeFile::read(&runtime_file(&env)).unwrap();
    let stopped = json_of(COLLIE.as_ref(), &env, &["daemon", "stop", "--json"]);
    let after = json_of(COLLIE.as_ref(), &env, &["daemon", "status", "--json"]);

    validate(&started, "DaemonStartOutput");
    validate(&status, "DaemonStatusOutput");
    validate(&stopped, "DaemonStopOutput");
    let copy = &installed_copy(&env);
    let installed: Value = serde_json::from_slice(
        &std::fs::read(env.home().join("installed-executables.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        (
            started["started"].clone(),
            status["daemon"]["product"].clone(),
            status["daemon"]["pid"].clone(),
            found.version.as_str(),
            installed["executables"][0]["copy"]["kind"].clone(),
            copy.is_file(),
            stopped,
            after["daemon"].clone(),
        ),
        (
            json!(true),
            json!("collie"),
            json!(found.pid),
            VERSION,
            json!("executable_copy"),
            true,
            json!({ "stopped": { "live_runs": 0 } }),
            Value::Null,
        )
    );
}

/// With no service installed, `daemon uninstall` has nothing to remove and
/// `daemon rotate-token` refuses, since a Daemon's token is new at each
/// launch; neither touches a service manager.
///
/// Covers: story:3-21, command:daemon.uninstall, command:daemon.rotate-token
#[test]
fn with_no_service_installed_uninstall_removes_nothing_and_no_token_rotates() {
    let env = TestEnv::new();

    let uninstalled = json_of(COLLIE.as_ref(), &env, &["daemon", "uninstall", "--json"]);
    let rotated = cli(&env, COLLIE)
        .args(["daemon", "rotate-token", "--json"])
        .run()
        .unwrap();
    let status = json_of(COLLIE.as_ref(), &env, &["daemon", "status", "--json"]);

    validate(&uninstalled, "DaemonUninstallOutput");
    assert_eq!(
        (
            uninstalled,
            rotated.exit,
            rotated.stdout.as_str(),
            rotated
                .stderr
                .contains("no service with socket activation is installed"),
            status["service"].clone(),
            env.home().join("0.0").join("daemon.token").exists(),
        ),
        (
            json!({ "removed": null }),
            Some(1),
            "",
            true,
            Value::Null,
            false,
        )
    );
}

/// Covers: story:3-11, story:3-12
#[cfg(unix)]
#[test]
fn the_runtime_file_and_the_socket_are_their_owners_alone() {
    let env = TestEnv::new();
    short_idle(&env, 60_000);
    let _stops = StopsTheDaemon::of(&env, COLLIE);

    json_of(COLLIE.as_ref(), &env, &["daemon", "start", "--json"]);
    let found = RuntimeFile::read(&runtime_file(&env)).unwrap();

    let DaemonEndpoint::Unix(socket) = &found.endpoint else {
        panic!(
            "a Daemon listens on a Unix socket here: {:?}",
            found.endpoint
        );
    };
    assert_eq!((mode(&runtime_file(&env)), mode(socket)), (0o600, 0o600));
}

/// Covers: story:3-11
#[test]
fn two_starts_at_once_start_one_daemon() {
    let env = TestEnv::new();
    short_idle(&env, 60_000);
    let _stops = StopsTheDaemon::of(&env, COLLIE);
    let start = || {
        let mut command = env.process_command(COLLIE);
        command
            .args(["daemon", "start", "--json"])
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        command.spawn().unwrap()
    };

    let (first, second) = (start(), start());
    let outputs: Vec<Value> = [first, second]
        .into_iter()
        .map(|child| serde_json::from_slice(&child.wait_with_output().unwrap().stdout).unwrap())
        .collect();

    let mut started: Vec<bool> = outputs
        .iter()
        .map(|output| output["started"].as_bool().unwrap())
        .collect();
    started.sort_unstable();
    assert_eq!(
        (started, outputs[0]["daemon"]["pid"].clone()),
        (vec![false, true], outputs[1]["daemon"]["pid"].clone())
    );
}

/// Covers: story:3-18, setting:daemon.idle_timeout
#[test]
fn the_daemon_stops_once_no_client_was_connected_for_its_idle_timeout() {
    let env = TestEnv::new();
    short_idle(&env, 300);
    let _stops = StopsTheDaemon::of(&env, COLLIE);

    json_of(COLLIE.as_ref(), &env, &["daemon", "start", "--json"]);
    wait_until_stopped(&env);
    let after = json_of(COLLIE.as_ref(), &env, &["daemon", "status", "--json"]);

    assert_eq!(
        (after["daemon"].clone(), runtime_file(&env).exists()),
        (Value::Null, false)
    );
}

/// `daemon launched`, which a start and the major's service run, serves as
/// the Daemon: once it listens it prints the runtime file that `daemon
/// status` reads, and it exits `0` when `daemon stop` stops it.
///
/// Covers: command:daemon.launched
#[test]
fn daemon_launched_prints_its_runtime_file_once_it_serves_and_exits_0_when_stopped() {
    let env = TestEnv::new();
    short_idle(&env, 60_000);
    let _stops = StopsTheDaemon::of(&env, COLLIE);
    let mut command = env.process_command(COLLIE);
    command
        .args(["daemon", "launched"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let daemon = env.spawn(&mut command).unwrap();
    let mut ready = String::new();
    BufReader::new(daemon.child().stdout.take().unwrap())
        .read_line(&mut ready)
        .unwrap();
    let printed: RuntimeFile = serde_json::from_str(&ready).unwrap();

    let status = json_of(COLLIE.as_ref(), &env, &["daemon", "status", "--json"]);
    json_of(COLLIE.as_ref(), &env, &["daemon", "stop", "--json"]);
    let exit = daemon.child().wait().unwrap().code();
    record_command(&env, &["daemon", "launched"], exit).unwrap();

    assert_eq!(
        (status["daemon"]["pid"].clone(), exit),
        (json!(printed.pid), Some(0))
    );
}

/// A Daemon killed while a Client's Run is live leaves that Run's state
/// whole and running, for a resume to take up (#218), and a runtime file
/// the next start replaces.
///
/// Covers: story:3-23, story:3-11
#[test]
fn a_killed_daemon_leaves_its_run_resumable_and_a_runtime_file_the_next_start_replaces() {
    let env = TestEnv::new();
    short_idle(&env, 60_000);
    let _stops = StopsTheDaemon::of(&env, COLLIE);
    let mut command = env.process_command(COLLIE);
    command
        .args(["daemon", "launched"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let daemon = env.spawn(&mut command).unwrap();
    let mut ready = String::new();
    BufReader::new(daemon.child().stdout.take().unwrap())
        .read_line(&mut ready)
        .unwrap();
    let found: RuntimeFile = serde_json::from_str(&ready).unwrap();
    let mut client = RawClient::opened(&found, "collie");
    client.call(1, "run.start", &json!({ "run_id": "cut" }));

    daemon.child().kill().unwrap();
    daemon.child().wait().unwrap();
    let restarted = json_of(COLLIE.as_ref(), &env, &["daemon", "start", "--json"]);

    let run = rusqlite::Connection::open(
        env.home()
            .join("0.0")
            .join("runs")
            .join("cut")
            .join("run.sqlite"),
    )
    .unwrap();
    let lifecycle: String = run
        .query_row("SELECT lifecycle FROM run", [], |row| row.get(0))
        .unwrap();
    let integrity: String = run
        .query_row("PRAGMA integrity_check", [], |row| row.get(0))
        .unwrap();
    assert_eq!(
        (
            lifecycle.as_str(),
            integrity.as_str(),
            restarted["started"].clone(),
            restarted["daemon"]["pid"] == json!(found.pid)
        ),
        ("running", "ok", json!(true), false)
    );
}

/// The Daemon socket of the Daemon the runtime file of `env`'s home names,
/// as the endpoint table's listener, with its token.
pub(crate) fn socket_listener(env: &TestEnv) -> Listener {
    let found = RuntimeFile::read(&runtime_file(env)).unwrap();
    let address = match &found.endpoint {
        #[cfg(unix)]
        DaemonEndpoint::Unix(path) => ListenerAddress::Unix(path.clone()),
        #[cfg(not(unix))]
        DaemonEndpoint::Unix(_) => unreachable!("a Daemon listens on TCP here"),
        DaemonEndpoint::Tcp(address) => ListenerAddress::Tcp(*address),
    };
    let hello = json!({
        "protocol_version": PROTOCOL_VERSION.to_string(),
        "surface": "sdk_typescript",
        "needs": "collie",
        "product_version": VERSION,
    });
    Listener::new(
        ListenerKind::DaemonSocket,
        Endpoint::JsonRpc {
            address,
            hello: Hello::new(hello.as_object().unwrap().clone(), "token"),
        },
        found.token,
    )
}

/// The token of the Daemon the runtime file of `env`'s home names.
fn daemon_token(env: &TestEnv) -> String {
    RuntimeFile::read(&runtime_file(env)).unwrap().token
}

/// The endpoint table's Daemon socket column, each refusal found in the
/// Audit log: the token of a launch `daemon stop` ended is expired, that of
/// the launch `daemon restart` replaced is revoked, and the operator's ops
/// token, read-only on `/metrics`, is scoped elsewhere.
///
/// Covers: story:3-12, story:4-21, story:4-24, story:4-90
#[test]
fn the_daemon_socket_passes_its_endpoint_table_rows() {
    let env = TestEnv::new();
    short_idle(&env, 60_000);
    let _stops = StopsTheDaemon::of(&env, COLLIE);
    let mut canaries = Canaries::new(env.seed());
    let ops_token = crate::endpoint::provision_ops_token(&env, &mut canaries);
    json_of(COLLIE.as_ref(), &env, &["daemon", "start", "--json"]);
    let expired = daemon_token(&env);
    json_of(COLLIE.as_ref(), &env, &["daemon", "stop", "--json"]);
    json_of(COLLIE.as_ref(), &env, &["daemon", "start", "--json"]);
    let revoked = daemon_token(&env);
    json_of(COLLIE.as_ref(), &env, &["daemon", "restart", "--json"]);
    let listener = socket_listener(&env)
        .with_other_listener_token("a-private-endpoints-token")
        .with_expired_token(expired)
        .with_revoked_token(revoked)
        .with_scoped_elsewhere_token(ops_token);

    let ran =
        EndpointRunner::new(crate::endpoint::audit_verifier(&env, "daemon_socket")).run(&listener);

    assert_eq!(ran, Ok(()));
}

/// Starts the Scripted engine as the Daemon of the test's home, on a script
/// of the corpus's `daemon/` group.
pub(crate) fn scripted_daemon(env: &TestEnv, script: &str) -> Process {
    let scripted = tools()
        .unwrap()
        .path(goodfetch_test_support::Tool::ScriptedEngine)
        .unwrap()
        .to_owned();
    let mut command = env.process_command(scripted);
    command
        .arg("--script")
        .arg(engine_script(script))
        .args(["--mode", "socket"])
        .stderr(Stdio::null());
    let process = env.spawn(&mut command).unwrap();
    reported_address(&mut process.child()).unwrap();
    process
}

fn stderr_snapshot(env: &TestEnv, name: &str, stderr: &str) {
    let values = SnapshotValues::of(env).with_path(COLLIE, "[COLLIE]");
    snapshot_settings(&values).bind(|| insta::assert_snapshot!(name, stderr.trim()));
}

/// A Daemon that refuses this Client's version, while it cannot hand over,
/// fails `--daemon` with a `fix` naming this executable's `daemon restart`
/// by its path (ADR 0098).
///
/// Covers: story:3-5, story:3-4, error:protocol_version_unsupported
#[test]
fn a_daemon_refusing_the_clis_version_fails_it_with_a_fix_naming_its_daemon_restart() {
    let env = TestEnv::new();
    let scripted = scripted_daemon(&env, "daemon/cli-hello-refused.json");

    let output = cli(&env, COLLIE)
        .args(["config", "explain", "daemon", "--daemon"])
        .run()
        .unwrap();
    scripted.child().wait().unwrap();

    assert_eq!(output.exit, Some(11), "{}", output.stderr);
    stderr_snapshot(&env, "daemon_refused_version", &output.stderr);
}

/// A runtime file left by a Daemon that died is replaced by the next start.
///
/// Covers: story:3-11
#[test]
fn a_runtime_file_whose_daemon_died_is_replaced() {
    let env = TestEnv::new();
    short_idle(&env, 60_000);
    let _stops = StopsTheDaemon::of(&env, COLLIE);
    let scripted = scripted_daemon(&env, "daemon/dead-pid.json");
    scripted.child().wait().unwrap();
    let dead = RuntimeFile::read(&runtime_file(&env)).unwrap();

    let started = json_of(COLLIE.as_ref(), &env, &["daemon", "start", "--json"]);

    assert_eq!(
        (
            started["started"].clone(),
            started["daemon"]["pid"] == json!(dead.pid)
        ),
        (json!(true), false)
    );
}

/// A Daemon that drops the Connection before it answers fails the command
/// rather than hanging it.
///
/// Covers: story:3-11
#[test]
fn a_connection_dropped_mid_command_fails_the_command() {
    let env = TestEnv::new();
    let scripted = scripted_daemon(&env, "daemon/cli-connection-dropped.json");

    let output = cli(&env, COLLIE)
        .args(["config", "explain", "daemon", "--daemon"])
        .run()
        .unwrap();
    scripted.child().wait().unwrap();

    assert_eq!(
        (output.exit, output.stdout.as_str()),
        (Some(1), ""),
        "{}",
        output.stderr
    );
}

/// The protocol's Compatibility corpus, recorded against an Attached
/// engine.
const CLIENT_TRANSCRIPT: &str = include_str!("../../../../compat/protocol/head/client.jsonl");
const ENGINE_TRANSCRIPT: &str = include_str!("../../../../compat/protocol/head/engine.jsonl");

/// Takes out each reply's engine instance, new in each engine process so
/// never the one recorded, answering whether each it took is a string.
fn without_engine_instances(messages: &mut [Value]) -> Vec<bool> {
    messages
        .iter_mut()
        .filter_map(|message| message["result"].as_object_mut()?.remove("engine_instance"))
        .map(|instance| instance.is_string())
        .collect()
}

/// A Client's transcript of the protocol's `head/` set, its `hello` given
/// the Daemon's token, replays against the current Daemon, each response
/// the one recorded but for the engine instance its `hello` names
/// (ADR 0066).
///
/// Covers: format:protocol, story:3-4
#[test]
fn the_protocols_client_transcript_replays_against_the_daemon() {
    let env = TestEnv::new();
    short_idle(&env, 60_000);
    let _stops = StopsTheDaemon::of(&env, COLLIE);
    json_of(COLLIE.as_ref(), &env, &["daemon", "start", "--json"]);
    let found = RuntimeFile::read(&runtime_file(&env)).unwrap();
    let mut client = RawClient::connected(&found.endpoint);
    let expected: Vec<Value> = ENGINE_TRANSCRIPT
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();

    for line in CLIENT_TRANSCRIPT.lines() {
        let mut message: Value = serde_json::from_str(line).unwrap();
        if message["method"] == json!("hello") {
            message["params"]["token"] = json!(found.token);
        }
        client.send(&message);
    }
    let mut received: Vec<Value> = (0..expected.len())
        .map(|_| client.next().unwrap())
        .collect();

    let by_id = |message: &Value| message["id"].to_string();
    let mut expected = expected;
    expected.sort_by_key(by_id);
    received.sort_by_key(by_id);
    let named = without_engine_instances(&mut received);
    let recorded = without_engine_instances(&mut expected);
    assert_eq!((received, named), (expected, recorded));
}
