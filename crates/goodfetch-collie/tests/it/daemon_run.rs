//! `daemon run`, the foreground Daemon, through Collie's executable (spec
//! #213's Seam 1; ADR 0049): it listens on the configured
//! `daemon.address`, writes no runtime file, never stops idle and logs
//! JSON lines to stdout; it refuses an address that is not loopback; and on
//! `SIGTERM` it checkpoints each run as resumable and exits, its runs
//! resuming on its next start. On Linux it runs as PID 1 of a PID
//! namespace and reaps every browser descendant.
#![expect(
    clippy::disallowed_methods,
    reason = "the tests start processes, signal them, and read the Daemon's files and logs themselves"
)]
#![expect(
    clippy::disallowed_types,
    reason = "a test finds a free port for the Daemon outside the engine (ADR 0060)"
)]
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use std::io::{BufRead as _, BufReader};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::mpsc;
use std::time::Duration;

use goodfetch_test_support::{DaemonEndpoint, Process, RuntimeFile, TestEnv};
use serde_json::{Value, json};

use crate::daemon::RawClient;

const COLLIE: &str = env!("CARGO_BIN_EXE_goodfetch-collie");

/// This commit's Product version, whose major folder is `0.0`.
const VERSION: &str = env!("CARGO_PKG_VERSION");

/// How long a test waits for a line the Daemon must log; a failure, never
/// a measure.
const LOG_DEADLINE: Duration = Duration::from_secs(60);

/// A free port on the loopback address, for the Daemon's configured
/// address.
pub(crate) fn free_address() -> SocketAddr {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
}

/// A foreground Daemon the test started, with its stdout's lines read on a
/// thread of their own.
pub(crate) struct Foreground {
    pub(crate) process: Process,
    lines: mpsc::Receiver<String>,
    /// Every line it wrote to stdout so far.
    pub(crate) logged: Vec<String>,
}

impl Foreground {
    /// Starts `executable daemon run` in `env`, with `variables` added to
    /// its environment.
    pub(crate) fn start(env: &TestEnv, executable: &str, variables: &[(&str, &str)]) -> Self {
        let mut command = env.process_command(executable);
        command
            .args(["daemon", "run"])
            .envs(variables.iter().copied())
            .stderr(Stdio::null());
        let process = env.spawn(&mut command).unwrap();
        let stdout = process.child().stdout.take().unwrap();
        let (sent, lines) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { return };
                if sent.send(line).is_err() {
                    return;
                }
            }
        });
        Self {
            process,
            lines,
            logged: Vec::new(),
        }
    }

    /// The fields of the first log record from here on whose message is
    /// `message`.
    pub(crate) fn record(&mut self, message: &str) -> Value {
        loop {
            let line = self
                .lines
                .recv_timeout(LOG_DEADLINE)
                .unwrap_or_else(|_| panic!("no `{message}` in its log: {:#?}", self.logged));
            self.logged.push(line.clone());
            if let Ok(record) = serde_json::from_str::<Value>(&line)
                && record["fields"]["message"] == json!(message)
            {
                return record["fields"].clone();
            }
        }
    }

    /// Where it serves, as it logged once it did.
    pub(crate) fn serving(&mut self) -> DaemonEndpoint {
        let serves = self.record("the foreground Daemon serves");
        DaemonEndpoint::Tcp(serves["endpoint"].as_str().unwrap().parse().unwrap())
    }

    /// Sends it `SIGTERM`, and waits for it to exit, reading the rest of its
    /// log; whether it exited with 0.
    pub(crate) fn terminate(&mut self) -> bool {
        let pid = self.process.child().id().to_string();
        let sent = std::process::Command::new("kill")
            .args(["-TERM", &pid])
            .status()
            .unwrap();
        assert!(sent.success(), "`kill -TERM {pid}` failed");
        let exited = self.process.child().wait().unwrap();
        while let Ok(line) = self.lines.recv_timeout(LOG_DEADLINE) {
            self.logged.push(line);
        }
        exited.success()
    }
}

/// The persistent token Clients of the foreground Daemon open with.
pub(crate) fn token(env: &TestEnv) -> String {
    std::fs::read_to_string(env.home().join("0.0").join("daemon.token")).unwrap()
}

/// A Client of the foreground Daemon at `endpoint`.
pub(crate) fn opened(env: &TestEnv, endpoint: &DaemonEndpoint) -> RawClient {
    RawClient::opened_at(endpoint, &token(env), "collie")
}

fn run_folder(env: &TestEnv, run_id: &str) -> PathBuf {
    env.home().join("0.0").join("runs").join(run_id)
}

/// The run row of `run_id`'s run file: its lifecycle, stop reason and run
/// epoch.
fn run_row(env: &TestEnv, run_id: &str) -> Value {
    let file = rusqlite::Connection::open(run_folder(env, run_id).join("run.sqlite")).unwrap();
    let row: String = file
        .query_row(
            "SELECT json_object('lifecycle', lifecycle, 'stop_reason', json(stop_reason), 'epoch', epoch) FROM run",
            [],
            |row| row.get(0),
        )
        .unwrap();
    serde_json::from_str(&row).unwrap()
}

/// The foreground Daemon listens on the address user configuration gives,
/// logs JSON lines to stdout, writes no runtime file and is still serving
/// long after its idle timeout with no Client connected.
///
/// Covers: story:3-98, command:daemon.run, setting:daemon.address
#[test]
fn daemon_run_listens_on_the_configured_address_logs_json_to_stdout_and_never_stops_idle() {
    let env = TestEnv::new();
    let address = free_address();
    env.user_configuration(&format!(
        "daemon.address = \"{address}\"\ndaemon.idle_timeout = 100\n"
    ));

    let mut daemon = Foreground::start(&env, COLLIE, &[]);
    let endpoint = daemon.serving();
    std::thread::sleep(Duration::from_secs(1));
    let mut client = opened(&env, &endpoint);
    let status = client.call(1, "daemon.status", &json!({}));
    drop(client);
    let exited = daemon.terminate();

    let not_json: Vec<&String> = daemon
        .logged
        .iter()
        .filter(|line| serde_json::from_str::<Value>(line).is_err())
        .collect();
    assert_eq!(
        (
            endpoint,
            status["result"]["kept_alive"].clone(),
            RuntimeFile::path(env.home(), VERSION).exists(),
            not_json,
            exited,
        ),
        (
            DaemonEndpoint::Tcp(address),
            json!(true),
            false,
            Vec::<&String>::new(),
            true,
        )
    );
}

/// Until the Daemon terminates TLS, `daemon run` refuses an address that is
/// not loopback, and starts nothing.
///
/// Covers: story:3-98, error:setting_invalid
#[test]
fn daemon_run_refuses_an_address_that_is_not_loopback() {
    let env = TestEnv::new();
    env.user_configuration("daemon.address = \"0.0.0.0:7900\"\n");

    let mut daemon = Foreground::start(&env, COLLIE, &[]);
    let refusal = daemon.record("`daemon.address` is `0.0.0.0:7900`, which is not a loopback address: `daemon run` listens only on loopback until the Daemon terminates TLS; put a TLS-terminating proxy in front of a loopback address.");

    let exited = daemon.process.child().wait().unwrap();
    assert_eq!(
        (exited.code(), refusal["code"].clone()),
        (Some(1), json!("setting_invalid"))
    );
}

/// On `SIGTERM`, under a short drain grace from the environment, the
/// foreground Daemon stops each live run with a Checkpoint, resumable, and
/// exits; its next start resumes both runs by their Run IDs, at the
/// next run epoch, and each finishes.
///
/// Covers: story:3-99, setting:daemon.drain_grace, operation:run.resume
#[test]
fn on_sigterm_daemon_run_checkpoints_each_run_and_its_next_start_resumes_them() {
    let env = TestEnv::new();
    env.user_configuration(&format!("daemon.address = \"{}\"\n", free_address()));
    let grace = [("GOODFETCH_DAEMON_DRAIN_GRACE", "5000")];

    let mut first = Foreground::start(&env, COLLIE, &grace);
    let endpoint = first.serving();
    let mut client = opened(&env, &endpoint);
    for (id, run_id) in [(1, "first"), (2, "second")] {
        let started = client.call(id, "run.start", &json!({ "run_id": run_id }));
        assert!(started.get("result").is_some(), "{started}");
    }
    let drained = first.terminate();
    drop(client);
    let stopped = ["first", "second"].map(|run_id| {
        (
            run_row(&env, run_id),
            run_folder(&env, run_id).join("checkpoint.sqlite").is_file(),
        )
    });

    let mut second = Foreground::start(&env, COLLIE, &grace);
    let endpoint = second.serving();
    let mut client = opened(&env, &endpoint);
    let resumed = [(3, "first"), (4, "second")].map(|(id, run_id)| {
        let resumed = client.call(id, "run.resume", &json!({ "run_id": run_id }));
        let finished = client.call(id + 10, "run.finish", &json!({ "run_id": run_id }));
        (
            resumed["result"]["started_as"].clone(),
            finished["result"]["lifecycle"].clone(),
            run_row(&env, run_id)["epoch"].clone(),
        )
    });
    drop(client);
    let exited = second.terminate();

    let drained_run = (
        json!({ "lifecycle": "stopped", "stop_reason": { "kind": "drain" }, "epoch": 1 }),
        true,
    );
    let resumed_run = (json!("resumed"), json!("finished"), json!(2));
    assert_eq!(
        (drained, stopped, resumed, exited),
        (
            true,
            [drained_run.clone(), drained_run],
            [resumed_run.clone(), resumed_run],
            true
        )
    );
}
