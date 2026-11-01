//! `goodfetch-scripted-engine` as a Daemon (ADR 0069, Seam B): in
//! `--mode socket` it writes the Daemon's runtime file into the test's
//! Goodfetch home, whose path holds a space and a non-ASCII letter, and a
//! tiny Client written here finds it there, as the CLI and the SDKs find a
//! Daemon; in `--mode tcp` the Client reads the loopback address it
//! prints. The `hello` token check is the script's own `expect`.
#![expect(
    clippy::disallowed_types,
    reason = "the tiny Client connects to the Scripted engine from outside it, never inside the Simulation (ADRs 0060 and 0069)"
)]

use std::io::{BufRead as _, BufReader, Read as _, Write as _};
use std::net::TcpStream;
#[cfg(unix)]
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{ExitStatus, Stdio};
use std::time::Duration;

use goodfetch_test_support::{
    DaemonEndpoint, Process, Reported, RuntimeFile, TestEnv, engine_script, reported_address,
};
use serde_json::{Value, json};

use crate::shell;

/// This commit's `goodfetch-scripted-engine`.
const SCRIPTED_ENGINE: &str = env!("CARGO_BIN_EXE_goodfetch-scripted-engine");

/// The token the corpus's Daemon scripts expect in `hello`.
const TOKEN: &str = "scripted-daemon-token";

/// How long the tiny Client waits for each message.
const DEADLINE: Duration = Duration::from_secs(10);

/// Starts the Scripted engine in `mode` on a corpus script, under the
/// test's environment map, and waits for the line it prints once a Client
/// can reach it.
fn start(env: &TestEnv, mode: &str, script: &str, extra: &[&str]) -> (Process, Reported) {
    let mut command = env.process_command(SCRIPTED_ENGINE);
    command
        .arg("--script")
        .arg(engine_script(script))
        .args(["--mode", mode])
        .args(extra)
        .stderr(Stdio::piped());
    let process = env.spawn(&mut command).unwrap();
    let reported = reported_address(&mut process.child()).unwrap();
    (process, reported)
}

/// Where a Daemon of this commit's version, `0.0.0`, keeps its runtime
/// file in `home`: the per-major folder `0.0`.
fn runtime_file(home: &Path) -> PathBuf {
    home.join("0.0").join("daemon.json")
}

/// Waits for the engine to exit, and returns its status and stderr.
fn finish(process: &Process) -> (ExitStatus, String) {
    let mut child = process.child();
    let status = child.wait().unwrap();
    let mut stderr = String::new();
    child
        .stderr
        .take()
        .unwrap()
        .read_to_string(&mut stderr)
        .unwrap();
    (status, stderr)
}

/// Plays the refused-`hello` script to its end, so the engine exits and
/// removes what it published.
fn refused_to_its_end(found: &RuntimeFile, process: &Process) {
    let mut client = TinyClient::found_through(found);
    client.hello("0.2", TOKEN);
    client.next().unwrap();
    let (status, stderr) = finish(process);
    assert!(status.success(), "{stderr}");
}

/// The failure code the Scripted engine's stderr opens with.
fn failure(stderr: &str) -> &str {
    stderr
        .strip_prefix("goodfetch-scripted-engine: ")
        .and_then(|rest| rest.split(':').next())
        .unwrap_or(stderr)
}

/// One connection to a Daemon.
enum Stream {
    Tcp(TcpStream),
    #[cfg(unix)]
    Unix(UnixStream),
}

impl Stream {
    fn try_clone(&self) -> Self {
        match self {
            Self::Tcp(stream) => Self::Tcp(stream.try_clone().unwrap()),
            #[cfg(unix)]
            Self::Unix(stream) => Self::Unix(stream.try_clone().unwrap()),
        }
    }
}

impl std::io::Read for Stream {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        match self {
            Self::Tcp(stream) => stream.read(buffer),
            #[cfg(unix)]
            Self::Unix(stream) => stream.read(buffer),
        }
    }
}

impl std::io::Write for Stream {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        match self {
            Self::Tcp(stream) => stream.write(buffer),
            #[cfg(unix)]
            Self::Unix(stream) => stream.write(buffer),
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            Self::Tcp(stream) => stream.flush(),
            #[cfg(unix)]
            Self::Unix(stream) => stream.flush(),
        }
    }
}

/// A Client as small as a test can write: it connects to an endpoint and
/// speaks newline-delimited JSON-RPC.
struct TinyClient {
    reader: BufReader<Stream>,
    writer: Stream,
    next_id: i64,
}

impl TinyClient {
    /// Connects to the endpoint a runtime file names, as a Client that
    /// found the Daemon there.
    fn found_through(runtime_file: &RuntimeFile) -> Self {
        match &runtime_file.endpoint {
            DaemonEndpoint::Tcp(address) => Self::tcp(&address.to_string()),
            #[cfg(unix)]
            DaemonEndpoint::Unix(path) => {
                let stream = UnixStream::connect(path).unwrap();
                stream.set_read_timeout(Some(DEADLINE)).unwrap();
                Self::over(Stream::Unix(stream))
            }
            #[cfg(not(unix))]
            DaemonEndpoint::Unix(path) => panic!("no Unix socket on this OS: {}", path.display()),
        }
    }

    /// Connects to a loopback TCP address.
    fn tcp(address: &str) -> Self {
        let stream = TcpStream::connect(address).unwrap();
        stream.set_read_timeout(Some(DEADLINE)).unwrap();
        Self::over(Stream::Tcp(stream))
    }

    fn over(stream: Stream) -> Self {
        Self {
            writer: stream.try_clone(),
            reader: BufReader::new(stream),
            next_id: 0,
        }
    }

    /// Sends a request and returns its ID.
    fn request(&mut self, method: &str, params: &Value) -> i64 {
        let id = self.next_id;
        self.next_id += 1;
        let message = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        let mut line = serde_json::to_vec(&message).unwrap();
        line.push(b'\n');
        self.writer.write_all(&line).unwrap();
        self.writer.flush().unwrap();
        id
    }

    /// Says `hello` as the CLI of Collie at `protocol_version`, with the
    /// Daemon's token.
    fn hello(&mut self, protocol_version: &str, token: &str) -> i64 {
        self.request(
            "hello",
            &json!({
                "protocol_version": protocol_version,
                "surface": "cli",
                "token": token,
                "needs": "collie",
                "product_version": "0.1.0",
            }),
        )
    }

    /// The next message, or `None` once the Daemon closed the connection.
    fn next(&mut self) -> Option<Value> {
        let mut line = String::new();
        match self.reader.read_line(&mut line) {
            Ok(0) => None,
            Ok(_) => Some(serde_json::from_str(&line).unwrap()),
            Err(error) if error.kind() == std::io::ErrorKind::ConnectionReset => None,
            Err(error) => panic!("the Daemon's connection failed: {error}"),
        }
    }
}

#[test]
fn a_client_finds_the_daemon_through_its_runtime_file_and_is_refused_at_hello() {
    let env = TestEnv::new();
    let home = env.home().to_str().unwrap();
    assert!(home.contains(' ') && home.contains('é'), "{home}");
    let (process, _) = start(
        &env,
        "socket",
        "daemon/hello-refused-protocol-version-unsupported.json",
        &[],
    );
    let found = RuntimeFile::read(&runtime_file(env.home())).unwrap();
    let mut client = TinyClient::found_through(&found);

    let id = client.hello("0.2", &found.token);
    let reply = client.next().unwrap();

    assert_eq!(reply["id"], id);
    assert_eq!(
        reply["error"]["data"]["code"],
        "protocol_version_unsupported"
    );
    assert_eq!(client.next(), None);
    let (status, stderr) = finish(&process);
    assert!(status.success(), "{stderr}");
}

#[test]
fn a_client_reaches_the_tcp_mode_at_the_address_it_prints() {
    let env = TestEnv::new();
    let (process, reported) = start(
        &env,
        "tcp",
        "daemon/hello-refused-protocol-version-unsupported.json",
        &[],
    );
    let mut client = TinyClient::tcp(&reported.address);

    let id = client.hello("0.2", TOKEN);
    let reply = client.next().unwrap();

    assert_eq!(reply["id"], id);
    assert_eq!(
        reply["error"]["data"]["code"],
        "protocol_version_unsupported"
    );
    assert_eq!(client.next(), None);
    let (status, stderr) = finish(&process);
    assert!(status.success(), "{stderr}");
}

#[test]
fn the_runtime_file_names_the_endpoint_token_version_and_pid() {
    let env = TestEnv::new();
    let (process, reported) = start(
        &env,
        "socket",
        "daemon/hello-refused-protocol-version-unsupported.json",
        &["--token", "launch-7", "--product-version", "0.4.2"],
    );

    let found = RuntimeFile::read(&env.home().join("0.4").join("daemon.json")).unwrap();

    assert_eq!(found.endpoint.to_string(), reported.address);
    assert_eq!(found.token, "launch-7");
    assert_eq!(found.version, "0.4.2");
    assert_eq!(found.pid, process.child().id());
    refused_to_its_end(&found, &process);
}

#[test]
fn the_runtime_file_and_its_endpoint_are_removed_when_the_play_ends() {
    let env = TestEnv::new();
    let (process, reported) = start(
        &env,
        "socket",
        "daemon/hello-refused-protocol-version-unsupported.json",
        &[],
    );
    let found = RuntimeFile::read(&runtime_file(env.home())).unwrap();
    let mut client = TinyClient::found_through(&found);
    client.hello("0.2", TOKEN);
    client.next().unwrap();

    let (status, stderr) = finish(&process);

    assert!(status.success(), "{stderr}");
    assert!(!runtime_file(env.home()).exists());
    if let DaemonEndpoint::Unix(path) = &found.endpoint {
        assert!(!path.exists(), "{}", reported.address);
    }
}

#[test]
fn a_hello_without_the_scripts_token_fails_the_play_with_unexpected_message() {
    let env = TestEnv::new();
    let (process, _) = start(
        &env,
        "socket",
        "daemon/hello-refused-protocol-version-unsupported.json",
        &[],
    );
    let found = RuntimeFile::read(&runtime_file(env.home())).unwrap();
    let mut client = TinyClient::found_through(&found);

    client.hello("0.2", "a-stranger's-token");

    assert_eq!(client.next(), None);
    let (status, stderr) = finish(&process);
    assert_eq!(status.code(), Some(1));
    assert_eq!(failure(&stderr), "unexpected_message", "{stderr}");
    assert!(!runtime_file(env.home()).exists());
}

#[cfg(unix)]
#[test]
fn a_failed_play_keeps_its_code_when_removing_the_runtime_file_also_fails() {
    use std::os::unix::fs::PermissionsExt as _;
    let env = TestEnv::new();
    let (process, _) = start(
        &env,
        "socket",
        "daemon/hello-refused-protocol-version-unsupported.json",
        &[],
    );
    let found = RuntimeFile::read(&runtime_file(env.home())).unwrap();
    let folder = env.home().join("0.0");
    let mode = |bits| std::fs::set_permissions(&folder, std::fs::Permissions::from_mode(bits));
    mode(0o500).unwrap();
    let mut client = TinyClient::found_through(&found);

    client.hello("0.2", "a-stranger's-token");

    assert_eq!(client.next(), None);
    let (status, stderr) = finish(&process);
    mode(0o700).unwrap();
    assert_eq!(status.code(), Some(1));
    assert_eq!(failure(&stderr), "unexpected_message", "{stderr}");
}

#[cfg(unix)]
#[test]
fn a_daemon_that_cannot_publish_fails_with_publish_failed() {
    use std::os::unix::fs::PermissionsExt as _;
    let env = TestEnv::new();
    let folder = env.home().join("0.0");
    std::fs::create_dir_all(&folder).unwrap();
    let mode = |bits| std::fs::set_permissions(&folder, std::fs::Permissions::from_mode(bits));
    mode(0o500).unwrap();
    let mut command = env.process_command(SCRIPTED_ENGINE);
    command
        .arg("--script")
        .arg(engine_script(
            "daemon/hello-refused-protocol-version-unsupported.json",
        ))
        .args(["--mode", "socket"])
        .stderr(Stdio::piped());
    let process = env.spawn(&mut command).unwrap();

    let (status, stderr) = finish(&process);

    mode(0o700).unwrap();
    assert_eq!(status.code(), Some(1));
    assert_eq!(failure(&stderr), "publish_failed", "{stderr}");
}

#[test]
fn the_dead_pid_script_leaves_a_runtime_file_whose_pid_no_longer_runs() {
    let env = TestEnv::new();
    let (process, _) = start(&env, "socket", "daemon/dead-pid.json", &[]);
    let pid = process.child().id();

    let (status, stderr) = finish(&process);

    assert_eq!(status.code(), Some(101), "{stderr}");
    let found = RuntimeFile::read(&runtime_file(env.home())).unwrap();
    assert_eq!(found.pid, pid);
    let probe = shell(
        &env,
        &format!("kill -0 {pid} 2>/dev/null"),
        &format!("tasklist /NH /FI \"PID eq {pid}\" | find \" {pid} \""),
    );
    assert!(!probe.close_stdin_and_wait().unwrap().success());
}

#[test]
fn a_daemon_started_after_a_dead_one_replaces_its_files() {
    let env = TestEnv::new();
    let (dead, _) = start(&env, "socket", "daemon/dead-pid.json", &[]);
    let (status, stderr) = finish(&dead);
    assert_eq!(status.code(), Some(101), "{stderr}");

    let (process, _) = start(
        &env,
        "socket",
        "daemon/hello-refused-protocol-version-unsupported.json",
        &[],
    );

    let found = RuntimeFile::read(&runtime_file(env.home())).unwrap();
    assert_eq!(found.pid, process.child().id());
    refused_to_its_end(&found, &process);
}

#[test]
fn the_dropped_connection_script_closes_mid_call_after_its_expect() {
    let env = TestEnv::new();
    let record = env.folder().join("record.jsonl");
    let (process, _) = start(
        &env,
        "socket",
        "daemon/connection-dropped-mid-call.json",
        &["--record", record.to_str().unwrap()],
    );
    let found = RuntimeFile::read(&runtime_file(env.home())).unwrap();
    let mut client = TinyClient::found_through(&found);
    client.hello("0.1", &found.token);
    assert_eq!(client.next().unwrap()["result"]["product"], "collie");

    client.request("engine.info", &json!({}));

    assert_eq!(client.next(), None);
    let (status, stderr) = finish(&process);
    assert!(status.success(), "{stderr}");
    // The engine read the call before it closed, and answered nothing after.
    let seen: Vec<Value> = std::fs::read_to_string(&record)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let last = seen.last().unwrap();
    assert_eq!(
        (&last["from"], &last["message"]["method"]),
        (&json!("client"), &json!("engine.info")),
        "{seen:?}"
    );
}

#[cfg(unix)]
#[test]
fn the_runtime_file_and_the_socket_are_for_their_owner_only() {
    use std::os::unix::fs::PermissionsExt as _;
    let env = TestEnv::new();
    let (process, reported) = start(
        &env,
        "socket",
        "daemon/hello-refused-protocol-version-unsupported.json",
        &[],
    );

    let file = std::fs::metadata(runtime_file(env.home())).unwrap();
    let socket = std::fs::metadata(&reported.address).unwrap();

    assert_eq!(file.permissions().mode() & 0o777, 0o600);
    assert_eq!(socket.permissions().mode() & 0o777, 0o600);
    // A socket the home's path is too long for sits in a folder of its own,
    // which only its owner may enter.
    let socket_folder = Path::new(&reported.address).parent().unwrap();
    if !socket_folder.starts_with(env.home()) {
        let folder = std::fs::metadata(socket_folder).unwrap();
        assert_eq!(folder.permissions().mode() & 0o777, 0o700);
    }
    refused_to_its_end(
        &RuntimeFile::read(&runtime_file(env.home())).unwrap(),
        &process,
    );
}
