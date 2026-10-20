//! `goodfetch-test-env --daemon` (ADRs 0062, 0069 and 0094, Seam B): a
//! Daemon started in the test's own Goodfetch home, from the tool
//! manifest's Collie executable, or Foxhound's with `--daemon=foxhound`,
//! whose socket and token the description names, and which stops, leaving
//! no runtime file, when `goodfetch-test-env`'s stdin closes.
#![expect(
    clippy::disallowed_types,
    reason = "the tiny Client connects to the described Daemon from outside it, never inside the Simulation (ADRs 0060 and 0069)"
)]

use std::io::{BufRead as _, BufReader, Write as _};
use std::path::{Path, PathBuf};
use std::time::Duration;

use goodfetch_test_support::{Process, TOOLS_VARIABLE, TestEnv, Tool, tools};
use serde_json::{Value, json};

use crate::TEST_ENV;

/// The committed schema of the description, which `xtask codegen` writes.
const SCHEMA: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../test-support/fixtures/schemas/test-env-description.schema.json"
);

/// How long the tiny Client waits for each message.
const DEADLINE: Duration = Duration::from_secs(10);

/// Writes a test tool manifest listing `goodfetch-test-env` and `entries`
/// into the test's folder, and returns its path.
fn manifest_with(env: &TestEnv, entries: &[(&str, &Path)]) -> PathBuf {
    let mut manifest = json!({ "test_env": TEST_ENV });
    for (name, path) in entries {
        manifest[*name] = json!(path);
    }
    let path = env
        .folder()
        .join(format!("tools-{}.json", env.unique_name("manifest")));
    std::fs::write(&path, manifest.to_string()).unwrap();
    path
}

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
            "product_version": "0.0.0",
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

/// The description's errors against the committed schema.
fn schema_errors(description: &Value) -> Vec<String> {
    let schema: Value = serde_json::from_str(&std::fs::read_to_string(SCHEMA).unwrap()).unwrap();
    jsonschema::validator_for(&schema)
        .unwrap()
        .iter_errors(description)
        .map(|error| error.to_string())
        .collect()
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
