//! `goodfetch-test-env --daemon` (ADRs 0062, 0069 and 0094, Seam B): a
//! Daemon started in the test's own Goodfetch home, from the tool
//! manifest's Collie executable, or Foxhound's with `--daemon=foxhound`,
//! whose socket and token the description names, and which stops, leaving
//! no runtime file, when `goodfetch-test-env`'s stdin closes; and the
//! `Daemon` type behind it, whose stop checks the home it leaves.
#![expect(
    clippy::disallowed_types,
    reason = "the tiny Client connects to the described Daemon from outside it, never inside the Simulation (ADRs 0060 and 0069)"
)]

use std::io::{BufRead as _, BufReader, Write as _};
use std::path::{Path, PathBuf};
use std::time::Duration;

use goodfetch_test_support::{
    Daemon, Process, Product, RuntimeFile, TOOLS_VARIABLE, TestEnv, Tool, command, tools,
};
use serde_json::{Value, json};

use crate::{TEST_ENV, make_executable, manifest_with, schema_errors};

/// How long the tiny Client waits for each message.
const DEADLINE: Duration = Duration::from_secs(10);

/// The commit's executable of `tool`, from the manifest `just test` writes.
fn commit_tool(tool: Tool) -> PathBuf {
    tools().unwrap().path(tool).unwrap().to_owned()
}

/// Starts `goodfetch-test-env` with `arguments` and that manifest.
fn start(env: &TestEnv, arguments: &[&str], manifest: &Path) -> Process {
    let mut command = env.process_command(TEST_ENV);
    command.args(arguments).env(TOOLS_VARIABLE, manifest);
    env.spawn(&mut command).unwrap()
}

/// The description it prints on its first line.
fn description(process: &Process) -> Value {
    let stdout = process.child().stdout.take().unwrap();
    let mut line = String::new();
    BufReader::new(stdout).read_line(&mut line).unwrap();
    serde_json::from_str(&line).unwrap_or_else(|error| panic!("{error}: {line:?}"))
}

/// The described Daemon's socket and token.
fn socket_and_token(description: &Value) -> (String, String) {
    let daemon = &description["daemon"];
    let field = |name: &str| {
        daemon[name]
            .as_str()
            .unwrap_or_else(|| panic!("no daemon.{name}: {description}"))
            .to_owned()
    };
    (field("socket"), field("token"))
}

/// The described Daemon's answer to a `hello` with `token` that needs
/// `product`, or `None` when it closed the Connection without one.
fn hello(socket: &str, token: &str, product: &str) -> Option<Value> {
    let (mut writer, reader) = connect(socket).unwrap();
    let request = json!({
        "jsonrpc": "2.0", "id": 0, "method": "hello",
        "params": {
            "protocol_version": "0.1",
            "surface": "sdk_typescript",
            "token": token,
            "needs": product,
            "product_version": env!("CARGO_PKG_VERSION"),
        },
    });
    writeln!(writer, "{request}").unwrap();
    writer.flush().unwrap();
    let mut line = String::new();
    match BufReader::new(reader).read_line(&mut line) {
        Ok(0) => None,
        Ok(_) => Some(serde_json::from_str(&line).unwrap()),
        Err(error) if error.kind() == std::io::ErrorKind::ConnectionReset => None,
        Err(error) => panic!("the Daemon's Connection failed: {error}"),
    }
}

type Halves = (Box<dyn std::io::Write>, Box<dyn std::io::Read>);

/// Connects to the described socket: a Unix socket's path, or loopback
/// TCP's address on Windows (ADR 0024).
fn connect(socket: &str) -> std::io::Result<Halves> {
    #[cfg(unix)]
    {
        let stream = std::os::unix::net::UnixStream::connect(socket)?;
        stream.set_read_timeout(Some(DEADLINE))?;
        Ok((Box::new(stream.try_clone()?), Box::new(stream)))
    }
    #[cfg(not(unix))]
    {
        let stream = std::net::TcpStream::connect(socket)?;
        stream.set_read_timeout(Some(DEADLINE))?;
        Ok((Box::new(stream.try_clone()?), Box::new(stream)))
    }
}

#[test]
fn daemon_describes_a_collie_daemon_a_client_reaches_with_the_described_token() {
    let env = TestEnv::new();
    let manifest = manifest_with(&env, &[("collie", &commit_tool(Tool::Collie))]);
    let process = start(&env, &["--daemon"], &manifest);
    let description = description(&process);
    let (socket, token) = socket_and_token(&description);

    let answer = hello(&socket, &token, "collie");

    process.close_stdin_and_wait().unwrap();
    assert_eq!(schema_errors(&description), Vec::<String>::new());
    assert_eq!(
        answer.as_ref().map(|answer| &answer["result"]["product"]),
        Some(&json!("collie")),
        "{answer:?}"
    );
}

#[test]
fn two_test_envs_get_two_daemons_in_two_homes_each_with_its_own_token() {
    let env = TestEnv::new();
    let manifest = manifest_with(&env, &[("collie", &commit_tool(Tool::Collie))]);
    let first = start(&env, &["--daemon"], &manifest);
    let second = start(&env, &["--daemon"], &manifest);
    let (one, two) = (description(&first), description(&second));
    let ((socket_one, token_one), (socket_two, token_two)) =
        (socket_and_token(&one), socket_and_token(&two));

    let answered = |socket: &str, token: &str| {
        hello(socket, token, "collie").is_some_and(|answer| answer.get("result").is_some())
    };
    let answers = [
        answered(&socket_one, &token_one),
        answered(&socket_two, &token_two),
        answered(&socket_one, &token_two),
    ];

    first.close_stdin_and_wait().unwrap();
    second.close_stdin_and_wait().unwrap();
    assert_ne!(one["home"], two["home"]);
    assert_ne!((&socket_one, &token_one), (&socket_two, &token_two));
    assert_eq!(answers, [true, true, false]);
}

#[test]
fn daemon_foxhound_describes_a_daemon_foxhounds_executable_runs() {
    let env = TestEnv::new();
    let manifest = manifest_with(&env, &[("foxhound", &commit_tool(Tool::Foxhound))]);
    let process = start(&env, &["--daemon=foxhound"], &manifest);
    let description = description(&process);
    let (socket, token) = socket_and_token(&description);

    let answer = hello(&socket, &token, "foxhound");

    process.close_stdin_and_wait().unwrap();
    assert_eq!(
        answer.as_ref().map(|answer| &answer["result"]["product"]),
        Some(&json!("foxhound")),
        "{answer:?}"
    );
}

#[test]
fn closing_stdin_stops_the_daemon() {
    let env = TestEnv::new();
    let manifest = manifest_with(&env, &[("collie", &commit_tool(Tool::Collie))]);
    let process = start(&env, &["--daemon"], &manifest);
    let (socket, _) = socket_and_token(&description(&process));

    let status = process.close_stdin_and_wait().unwrap();

    assert!(status.success());
    assert!(connect(&socket).is_err(), "{socket} still accepts");
}

#[test]
fn a_description_without_daemon_has_no_such_field() {
    let env = TestEnv::new();
    let process = start(&env, &[], &manifest_with(&env, &[]));

    let description = description(&process);

    process.close_stdin_and_wait().unwrap();
    assert_eq!(description.get("daemon"), None);
}

#[test]
fn a_daemon_without_its_token_fails_validation() {
    let description = json!({
        "folder": "/tmp/x é 1", "home": "/tmp/x é 1/home", "env": {}, "seed": 1,
        "start_time": "2026-01-05T09:00:00Z", "tools": {},
        "daemon": { "socket": "/tmp/x é 1/home/0.0/daemon.sock" }
    });
    let mut complete = description.clone();
    complete["daemon"]["token"] = json!("0f1e");

    let valid = (
        schema_errors(&description).is_empty(),
        schema_errors(&complete).is_empty(),
    );

    assert_eq!(valid, (false, true));
}

#[test]
fn a_daemon_that_fails_to_start_fails_the_start_with_its_message_on_stderr() {
    let env = TestEnv::new();
    let program = stand_in(
        &env,
        "failing-daemon",
        "#!/bin/sh\necho 'the Daemon cannot listen: stand-in refusal' >&2\nexit 1\n",
        "@echo the Daemon cannot listen: stand-in refusal 1>&2\r\n@exit /b 1\r\n",
    );

    let output = command(&env, TEST_ENV)
        .arg("--daemon")
        .env(TOOLS_VARIABLE, manifest_with(&env, &[("collie", &program)]))
        .output()
        .unwrap();

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(1), "{stderr}");
    assert_eq!(output.stdout, b"");
    assert!(
        stderr.contains("the Daemon cannot listen: stand-in refusal"),
        "{stderr}"
    );
}

/// A stand-in Daemon that publishes and prints a runtime file, and whose
/// `daemon stop` kills it, leaving that file in the home, as a crashed
/// Daemon does.
#[cfg(unix)]
pub(crate) const LEAVES_ITS_RUNTIME_FILE: &str = r#"#!/bin/sh
case "$2" in
launched)
  /bin/mkdir -p "$GOODFETCH_HOME/0.0"
  line="{\"endpoint\":{\"unix\":\"$GOODFETCH_HOME/0.0/daemon.sock\"},\"token\":\"stand-in\",\"version\":\"0.0.0\",\"pid\":$$}"
  printf '%s' "$line" > "$GOODFETCH_HOME/0.0/daemon.json"
  printf '%s\n' "$line"
  read -r _ ;;
stop)
  read -r line < "$GOODFETCH_HOME/0.0/daemon.json"
  pid=${line##*\"pid\":}
  kill "${pid%\}}" ;;
esac
"#;

#[cfg(unix)]
#[test]
fn a_daemon_that_leaves_its_runtime_file_fails_the_teardown() {
    let env = TestEnv::new();
    let program = stand_in(&env, "leaving-daemon", LEAVES_ITS_RUNTIME_FILE, "");
    let process = start(
        &env,
        &["--daemon"],
        &manifest_with(&env, &[("collie", &program)]),
    );
    socket_and_token(&description(&process));

    let status = process.close_stdin_and_wait().unwrap();

    assert_eq!(status.code(), Some(1));
}

#[test]
fn a_stopped_daemon_leaves_no_runtime_file_in_the_home() {
    let env = TestEnv::new();
    let daemon = Daemon::start(&env, Product::Collie).unwrap();
    // The workspace's version, which every executable of the commit reports.
    let runtime_file = RuntimeFile::path(env.home(), env!("CARGO_PKG_VERSION"));
    let published = runtime_file.is_file();

    daemon.stop(&env).unwrap();

    assert_eq!((published, runtime_file.exists()), (true, false));
}

/// A Daemon the test drops without stopping it is stopped with its own
/// `daemon stop` all the same, so it ends its Runs and its children before
/// it exits, and leaves no runtime file, where a kill would leave one.
#[test]
fn a_dropped_daemon_is_stopped_with_its_own_stop_and_leaves_no_runtime_file() {
    let env = TestEnv::new();
    let daemon = Daemon::start(&env, Product::Collie).unwrap();
    let runtime_file = RuntimeFile::path(env.home(), env!("CARGO_PKG_VERSION"));
    let published = runtime_file.is_file();

    drop(daemon);

    assert_eq!((published, runtime_file.exists()), (true, false));
}

#[cfg(unix)]
#[test]
fn stopping_a_daemon_that_leaves_its_runtime_file_fails() {
    let env = TestEnv::new();
    let program = stand_in(&env, "leaving-daemon", LEAVES_ITS_RUNTIME_FILE, "");
    let daemon =
        Daemon::start_program(&env, &program, goodfetch_test_support::DAEMON_DEADLINE).unwrap();
    let runtime_file = RuntimeFile::path(env.home(), "0.0.0");

    let stopped = daemon.stop(&env);

    assert!(
        matches!(&stopped, Err(goodfetch_test_support::DaemonError::RuntimeFileLeft(path)) if *path == runtime_file),
        "{stopped:?}"
    );
}

/// A stand-in executable in the test's folder: a shell script on Unix, a
/// batch file on Windows.
pub(crate) fn stand_in(env: &TestEnv, name: &str, unix: &str, windows: &str) -> PathBuf {
    if cfg!(windows) {
        let path = env.folder().join(format!("{name}.cmd"));
        std::fs::write(&path, windows).unwrap();
        path
    } else {
        let path = env.folder().join(name);
        std::fs::write(&path, unix).unwrap();
        make_executable(&path);
        path
    }
}
