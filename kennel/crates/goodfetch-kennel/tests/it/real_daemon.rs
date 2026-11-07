//! The Kennel's controls against a real Daemon, Collie's executable that
//! the root workspace builds (ADR 0141): a test program starts its Runs on
//! a Connection of its own, and the Kennel's Client, on another
//! Connection, pauses one, stops one and aborts one. This proves that the
//! Daemon accepts `run.pause`, `run.stop` and `run.abort` from a Connection
//! that did not start the Run.
#![expect(
    clippy::disallowed_methods,
    reason = "the test starts the Daemon's process, reads its runtime file and talks to its socket on the test thread"
)]
#![expect(
    clippy::disallowed_types,
    reason = "the test program's own Connection to the Daemon's socket"
)]

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use goodfetch_kennel::client::{Call, ConnectionChange};
use goodfetch_kennel::home::Home;
use goodfetch_protocol::catalog::PROTOCOL_VERSION;
use goodfetch_protocol::connection::{DaemonEndpoint, ProductVersion};
use goodfetch_protocol::daemon::DaemonRuntimeFile;
use goodfetch_protocol::run::{RunAbortParams, RunId, RunParams, RunWaitParams};
use serde_json::{Value, json};

use crate::client::{Following, NEVER};
use crate::support::{PATIENCE, collie};

/// A Daemon of Collie's executable, run as the Daemon of a test's own
/// Goodfetch home under a cleared environment, as the root workspace's
/// test Daemons are: `daemon launched --keep-alive`, the process `daemon
/// start` runs, so it is the test's child and never stops idle while the
/// test runs. Dropping it runs `daemon stop`, then kills it if it has not
/// ended.
struct Daemon {
    child: Child,
    executable: PathBuf,
    variables: Vec<(String, PathBuf)>,
    runtime_file: DaemonRuntimeFile,
}

impl Daemon {
    fn start(executable: &Path, folder: &Path) -> Self {
        let user = folder.join("user");
        let variables: Vec<(String, PathBuf)> = vec![
            ("GOODFETCH_HOME".to_owned(), folder.join("home")),
            ("HOME".to_owned(), user.clone()),
            ("XDG_CONFIG_HOME".to_owned(), user.join(".config")),
            ("XDG_DATA_HOME".to_owned(), user.join(".local/share")),
            ("XDG_CACHE_HOME".to_owned(), user.join(".cache")),
            ("XDG_STATE_HOME".to_owned(), user.join(".local/state")),
        ];
        for (_, path) in &variables {
            std::fs::create_dir_all(path).unwrap();
        }
        // The Console on a free port, as the root workspace's test homes
        // have it, so no fixed port is taken (TESTING.md).
        let major = Self::major_folder(executable, &variables);
        let configuration = folder.join("home").join(major);
        std::fs::create_dir_all(&configuration).unwrap();
        std::fs::write(
            configuration.join("configuration.toml"),
            "version = \"0.1\"\n\n[settings]\nconsole.address = \"127.0.0.1:0\"\n",
        )
        .unwrap();
        let mut child = Self::command(executable, &variables)
            .args(["daemon", "launched", "--keep-alive"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let mut line = String::new();
        BufReader::new(child.stdout.take().unwrap())
            .read_line(&mut line)
            .unwrap();
        let runtime_file = serde_json::from_str(&line).unwrap_or_else(|error| {
            panic!("the Daemon printed its runtime file ({error}): {line:?}")
        });
        Self {
            child,
            executable: executable.to_owned(),
            variables,
            runtime_file,
        }
    }

    /// The per-major folder of `executable`'s version: `0.y` during `0.y`
    /// previews, the major otherwise (ADR 0024).
    fn major_folder(executable: &Path, variables: &[(String, PathBuf)]) -> String {
        let output = Self::command(executable, variables)
            .arg("--version")
            .output()
            .unwrap();
        let printed = String::from_utf8(output.stdout).unwrap();
        let version = printed.split_whitespace().last().unwrap();
        ProductVersion::parse(version).unwrap().line().to_string()
    }

    fn command(executable: &Path, variables: &[(String, PathBuf)]) -> Command {
        let mut command = Command::new(executable);
        command
            .env_clear()
            .envs(variables.iter().map(|(name, value)| (name, value)))
            .env("TZ", "UTC")
            .env("LANG", "C.UTF-8");
        command
    }

    fn home(&self) -> Home {
        Home::new(&self.variables[0].1)
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = Self::command(&self.executable, &self.variables)
            .args(["daemon", "stop"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        let started = std::time::Instant::now();
        while matches!(self.child.try_wait(), Ok(None)) && started.elapsed() < PATIENCE {
            std::thread::sleep(Duration::from_millis(20));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// The test program's own Connection to the Daemon: an SDK's, which
/// starts the Runs and stays open while the Kennel acts on them.
struct Program {
    reader: BufReader<Box<dyn std::io::Read>>,
    writer: Box<dyn std::io::Write>,
    next_id: i64,
}

impl Program {
    fn connect(runtime_file: &DaemonRuntimeFile) -> Self {
        let (reader, writer): (Box<dyn std::io::Read>, Box<dyn std::io::Write>) =
            match &runtime_file.endpoint {
                #[cfg(unix)]
                DaemonEndpoint::Unix(path) => {
                    let stream = std::os::unix::net::UnixStream::connect(path).unwrap();
                    stream.set_read_timeout(Some(PATIENCE)).unwrap();
                    (Box::new(stream.try_clone().unwrap()), Box::new(stream))
                }
                #[cfg(not(unix))]
                DaemonEndpoint::Unix(_) => unreachable!("a Daemon listens on TCP here"),
                DaemonEndpoint::Tcp(address) => {
                    let stream = std::net::TcpStream::connect(address).unwrap();
                    stream.set_read_timeout(Some(PATIENCE)).unwrap();
                    (Box::new(stream.try_clone().unwrap()), Box::new(stream))
                }
            };
        let mut program = Self {
            reader: BufReader::new(reader),
            writer,
            next_id: 1,
        };
        program.call(
            "hello",
            &json!({
                "protocol_version": PROTOCOL_VERSION.to_string(),
                "surface": "sdk_typescript",
                "token": runtime_file.token.expose(),
                "needs": "collie",
                "product_version": "0.0.0",
            }),
        );
        program
    }

    /// Calls `method` and returns its result, failing the test on an
    /// error.
    fn call(&mut self, method: &str, params: &Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        let request = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        writeln!(self.writer, "{request}").unwrap();
        self.writer.flush().unwrap();
        loop {
            let mut line = String::new();
            assert!(
                self.reader.read_line(&mut line).unwrap() > 0,
                "the Daemon closed the program's Connection"
            );
            let message: Value = serde_json::from_str(&line).unwrap();
            if message["id"] == json!(id) {
                assert!(message.get("error").is_none(), "{method} failed: {message}");
                return message["result"].clone();
            }
        }
    }
}

fn run_id(run: &str) -> RunId {
    RunId::parse(run).unwrap()
}

#[test]
fn the_kennel_pauses_stops_and_aborts_runs_another_connection_started() {
    let Some(executable) = collie("the_kennel_pauses_stops_and_aborts_runs") else {
        return;
    };
    let folder = tempfile::tempdir().unwrap();
    let daemon = Daemon::start(&executable, folder.path());
    let mut program = Program::connect(&daemon.runtime_file);
    let runs = [
        "paused-by-the-kennel",
        "stopped-by-the-kennel",
        "aborted-by-the-kennel",
    ];
    for run in runs {
        program.call("run.start", &json!({"run_id": run}));
    }
    let mut kennel = Following::start(&daemon.home(), false, NEVER);
    let reached = kennel.next(1);
    assert!(
        matches!(reached[..], [ConnectionChange::Connected(_)]),
        "the Kennel reached the Daemon: {reached:?}"
    );

    let calls = [
        Call::Pause(RunParams {
            run_id: run_id(runs[0]),
        }),
        Call::Stop(RunWaitParams {
            run_id: run_id(runs[1]),
            timeout: None,
        }),
        Call::Abort(RunAbortParams {
            run_id: run_id(runs[2]),
            forwarded: false,
        }),
    ];
    let mut answers = Vec::new();
    for call in calls.clone() {
        kennel.call(call);
        answers.push(kennel.next_answer());
    }

    let expected: Vec<_> = calls.into_iter().map(|call| (call, Ok(()))).collect();
    assert_eq!(answers, expected, "the Daemon accepted each call");
    let ended: Vec<(Value, Value)> = runs
        .iter()
        .map(|run| {
            let progress = program.call("run.progress", &json!({"run_id": run}));
            (
                progress["lifecycle"].clone(),
                progress["stop_reason"]["kind"].clone(),
            )
        })
        .collect();
    assert_eq!(
        ended,
        [
            (json!("stopped"), json!("pause")),
            (json!("stopped"), json!("abort")),
            (json!("stopped"), json!("abort")),
        ],
        "each Run stopped as its control says"
    );
}
