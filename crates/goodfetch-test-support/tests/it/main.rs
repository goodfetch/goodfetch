//! goodfetch-test-support's integration tests (Seam B, Pre-merge): the
//! crate's interface with real child processes (seeded kills and a failed
//! test's Observation log among them), `goodfetch-test-env`'s command
//! line and JSON description, `goodfetch-scripted-engine` playing
//! scripts to a tiny Client, and the CLI helper's piped and
//! pseudo-terminal runs of fixture commands.
#![expect(
    clippy::disallowed_methods,
    reason = "these tests start processes, read files and wait outside any async worker"
)]
#![expect(
    clippy::unwrap_used,
    reason = "test code, where STYLE.md allows unwrap; Clippy's test exemption does not reach integration-test helpers"
)]

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
mod browser_cache;
mod canaries;
mod cli;
mod cli_json;
#[cfg(target_os = "linux")]
mod dns_namespace;
mod endpoints;
mod exercise_record;
mod hostile_tables;
mod journey_run;
#[cfg(target_os = "linux")]
mod lazyfs;
#[cfg(all(
    target_os = "linux",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]
mod no_exec;
mod observation_log;
mod probe;
mod proving_ground;
mod pty;
mod reported;
mod scripted_daemon;
mod scripted_engine;
mod seed;
mod seeded_kill;
mod ssrf_table;
mod strict_stdout;
mod teardown;
mod test_env;
mod test_env_daemon;
mod tools;

use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};

use goodfetch_test_support::{Process, TestEnv};
use serde_json::{Value, json};

/// This commit's `goodfetch-test-env`.
const TEST_ENV: &str = env!("CARGO_BIN_EXE_goodfetch-test-env");

/// The committed schema of `goodfetch-test-env`'s description, which `xtask
/// codegen` writes.
const DESCRIPTION_SCHEMA: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../test-support/fixtures/schemas/test-env-description.schema.json"
);

/// A description's errors against the committed schema; none when it
/// matches.
fn schema_errors(description: &Value) -> Vec<String> {
    let schema: Value =
        serde_json::from_str(&std::fs::read_to_string(DESCRIPTION_SCHEMA).unwrap()).unwrap();
    jsonschema::validator_for(&schema)
        .unwrap()
        .iter_errors(description)
        .map(|error| error.to_string())
        .collect()
}

/// Writes a test tool manifest listing `goodfetch-test-env` into the test's
/// folder, and returns its path.
fn manifest(env: &TestEnv) -> PathBuf {
    manifest_with(env, &[])
}

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

/// An HTTP/1.1 answer: its status and body.
struct Answer {
    status: u16,
    body: String,
}

impl Answer {
    fn json(&self) -> Value {
        serde_json::from_str(&self.body).unwrap()
    }
}

/// Sends `GET <target>` with `Host: <host>` to `address`, and reads the whole
/// answer, which `Connection: close` ends.
#[expect(
    clippy::disallowed_types,
    reason = "the tests reach a Proving ground over a plain socket, as any client of it does"
)]
fn get(address: &str, host: &str, target: &str) -> Answer {
    let mut stream = std::net::TcpStream::connect(address).unwrap();
    write!(
        stream,
        "GET {target} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n"
    )
    .unwrap();
    let mut response = Vec::new();
    stream.read_to_end(&mut response).unwrap();
    let response = String::from_utf8(response).unwrap();
    let (head, body) = response.split_once("\r\n\r\n").unwrap();
    let status = head.split(' ').nth(1).unwrap().parse().unwrap();
    Answer {
        status,
        body: body.to_owned(),
    }
}

/// How long [`still_answers`] waits for whatever accepted its connection to
/// answer before it judges that it is no Proving ground.
const IDENTIFY_DEADLINE: std::time::Duration = std::time::Duration::from_secs(10);

/// Sends `GET /tasks` to the Proving ground at `address`, and returns the
/// address the request left from, which its connection record lists from
/// then on.
#[expect(
    clippy::disallowed_types,
    reason = "the tests reach a Proving ground over a plain socket, as any client of it does"
)]
fn requested_from(address: &str) -> std::net::SocketAddr {
    let mut stream = std::net::TcpStream::connect(address).unwrap();
    let source = stream.local_addr().unwrap();
    write!(
        stream,
        "GET /tasks HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\n\r\n"
    )
    .unwrap();
    stream.read_to_end(&mut Vec::new()).unwrap();
    source
}

/// Whether the Proving ground that recorded a connection from `source`
/// still answers at `address`, as its own connection record shows: nothing
/// accepting there is none of it, and neither is another program that bound
/// the port it freed, which lists no such connection (#2038).
#[expect(
    clippy::disallowed_types,
    reason = "the tests reach a Proving ground over a plain socket, as any client of it does"
)]
fn still_answers(address: &str, source: std::net::SocketAddr) -> bool {
    let Ok(mut stream) = std::net::TcpStream::connect(address) else {
        return false;
    };
    let mut response = Vec::new();
    let read = stream
        .set_read_timeout(Some(IDENTIFY_DEADLINE))
        .and_then(|()| {
            write!(
                stream,
                "GET /observations HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\n\r\n"
            )
        })
        .and_then(|()| stream.read_to_end(&mut response));
    if read.is_err() {
        return false;
    }
    let response = String::from_utf8_lossy(&response);
    let observations: Value = response
        .split_once("\r\n\r\n")
        .and_then(|(_, body)| serde_json::from_str(body).ok())
        .unwrap_or_default();
    observations["connections"]
        .as_array()
        .is_some_and(|connections| {
            connections
                .iter()
                .any(|connection| connection["source"] == json!(source.to_string()))
        })
}

/// Lets its owner run the file at `path`, a script a test wrote.
#[cfg(unix)]
fn make_executable(path: &Path) {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

#[cfg(not(unix))]
fn make_executable(_path: &Path) {}

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
