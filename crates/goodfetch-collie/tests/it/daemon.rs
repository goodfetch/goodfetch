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

use std::collections::BTreeMap;
use std::io::{BufRead as _, BufReader, Write as _};
use std::path::{Path, PathBuf};
use std::process::Stdio;

use goodfetch_protocol::catalog::PROTOCOL_VERSION;
use goodfetch_test_support::{
    AuditUnreadable, DaemonEndpoint, Endpoint, EndpointRunner, Hello, Listener, ListenerAddress,
    ListenerKind, Process, Rule, RuntimeFile, SnapshotValues, TestEnv, cli, engine_script,
    reported_address, snapshot_settings, tools,
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
    let major = env.home().join("0.0");
    std::fs::create_dir_all(&major).unwrap();
    std::fs::write(
        major.join("configuration.toml"),
        format!("version = \"0.1\"\n\n[settings]\ndaemon.idle_timeout = {milliseconds}\n"),
    )
    .unwrap();
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

fn validate(output: &Value, definition: &str) {
    let bundle: Value = serde_json::from_str(BUNDLE).unwrap();
    let schema = json!({ "$ref": format!("#/$defs/{definition}"), "$defs": bundle["$defs"] });
    let validator = jsonschema::validator_for(&schema).unwrap();
    let errors: Vec<String> = validator
        .iter_errors(output)
        .map(|error| error.to_string())
        .collect();
    assert_eq!(errors, Vec::<String>::new());
}

fn runtime_file(env: &TestEnv) -> PathBuf {
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
        let (reader, writer): (Box<dyn std::io::Read>, Box<dyn std::io::Write>) =
            match &found.endpoint {
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
        let mut client = Self {
            reader: BufReader::new(reader),
            writer,
        };
        let answer = client.call(
            0,
            "hello",
            &json!({
                "protocol_version": PROTOCOL_VERSION.to_string(),
                "surface": "sdk_typescript",
                "token": found.token,
                "needs": product,
                "product_version": VERSION,
            }),
        );
        assert!(answer.get("result").is_some(), "{answer}");
        client
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

/// Covers: story:3-11, story:3-12, story:3-22, story:3-139, command:daemon.start, command:daemon.status, command:daemon.stop, operation:daemon.status, operation:daemon.stop
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
            stopped,
            after["daemon"].clone(),
        ),
        (
            json!(true),
            json!("collie"),
            json!(found.pid),
            VERSION,
            json!("executable_copy"),
            json!({ "stopped": { "live_runs": 0 } }),
            Value::Null,
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

/// The endpoint table's Daemon socket column, each refusal checked in the
/// Daemon's log until the Audit log records it (area 4).
///
/// Covers: story:3-12
#[test]
fn the_daemon_socket_passes_its_endpoint_table_rows() {
    let env = TestEnv::new();
    short_idle(&env, 60_000);
    let _stops = StopsTheDaemon::of(&env, COLLIE);
    json_of(COLLIE.as_ref(), &env, &["daemon", "start", "--json"]);
    let found = RuntimeFile::read(&runtime_file(&env)).unwrap();
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
    let listener = Listener::new(
        ListenerKind::DaemonSocket,
        Endpoint::JsonRpc {
            address,
            hello: Hello::new(hello.as_object().unwrap().clone(), "token"),
        },
        found.token.clone(),
    )
    .with_other_listener_token("a-private-endpoints-token");
    let log = env
        .home()
        .join("logs")
        .join(format!("goodfetch-collie-{}.jsonl", found.pid));
    let mut seen: BTreeMap<String, usize> = BTreeMap::new();
    let verifier = move |_: ListenerKind, rule: Rule| {
        let reason = match rule {
            Rule::TokenMissing => "token_missing",
            Rule::TokenWrong | Rule::TokenFromAnotherListener => "token_wrong",
            Rule::HelloNotFirst => "hello_not_first",
            _ => return Ok(false),
        };
        let text = std::fs::read_to_string(&log).map_err(AuditUnreadable::new)?;
        let refusals = text
            .lines()
            .filter_map(|line| serde_json::from_str::<Value>(line).ok())
            .filter(|record| {
                record["target"] == json!("goodfetch::daemon::refusal")
                    && record["fields"]["reason"] == json!(reason)
            })
            .count();
        let earlier = seen.insert(reason.to_owned(), refusals).unwrap_or(0);
        Ok(refusals > earlier)
    };

    let ran = EndpointRunner::new(verifier).run(&listener);

    assert_eq!(ran, Ok(()));
}

/// Starts the Scripted engine as the Daemon of the test's home, on a script
/// of the corpus's `daemon/` group.
fn scripted_daemon(env: &TestEnv, script: &str) -> Process {
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
    let mut settings = snapshot_settings(&SnapshotValues::of(env));
    settings.add_filter(&regex_escape(COLLIE), "[COLLIE]");
    settings.bind(|| insta::assert_snapshot!(name, stderr.trim()));
}

fn regex_escape(text: &str) -> String {
    text.chars()
        .flat_map(|character| {
            let escaped = "\\.+*?()|[]{}^$".contains(character);
            escaped.then_some('\\').into_iter().chain([character])
        })
        .collect()
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
