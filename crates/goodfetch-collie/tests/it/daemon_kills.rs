//! Seeded kills of the Daemon and of its Clients on Collie's executable
//! (spec #213's multi-process tests; ADRs 0024 and 0060): a real process,
//! the Daemon or a Client, is killed (`kill -9`, `TerminateProcess` on
//! Windows) at a moment drawn from the test's seed among the `$/events`
//! its Client observed, and every Run stays resumable from its own state.
//! A Daemon's kill stops its Runs, which the next Daemon takes up; a
//! Client's kill stops that Client's Runs alone. Fixed seeds run in the
//! Pre-merge suite and on every OS in the Release suite; the case seeded by
//! `GOODFETCH_TEST_SEED` takes new seeds in the nightly-only job.
#![expect(
    clippy::disallowed_methods,
    reason = "the tests start processes, and read a Client's output on a thread of their own"
)]

use std::io::{BufRead as _, BufReader, Write as _};
use std::path::Path;
use std::process::{ChildStdin, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

use goodfetch_test_client::DaemonClient;
use goodfetch_test_support::{
    Fault, Moment, ObservationLog, Operation, Process, Record, RuntimeFile, TestEnv, Tool, tools,
};
use rstest::rstest;
use serde_json::{Value, json};

use crate::daemon::{StopsTheDaemon, json_of, runtime_file, short_idle};
use crate::daemon_hello;

const COLLIE: &str = env!("CARGO_BIN_EXE_goodfetch-collie");

/// How many Runs the Client starts, each one `$/events` notification, one
/// at a time.
const RUNS: u32 = 4;

/// How long the test waits for each message.
const DEADLINE: Duration = Duration::from_secs(10);

/// One Client's Connection to the Daemon, as the test drives it.
trait Wire {
    fn send(&mut self, message: &Value);

    /// The next message.
    ///
    /// # Panics
    ///
    /// When the Connection closed first, or nothing came within
    /// [`DEADLINE`].
    fn next(&mut self) -> Value;
}

impl Wire for DaemonClient {
    fn send(&mut self, message: &Value) {
        Self::send(self, message);
    }

    fn next(&mut self) -> Value {
        Self::next(self).expect("the Daemon closed the Connection before the moment")
    }
}

/// The Connection of a Client process, `goodfetch-test-env daemon-client`,
/// which relays its stdin to the Daemon's socket and the socket to its
/// stdout.
struct Relay {
    stdin: ChildStdin,
    lines: Receiver<std::io::Result<String>>,
}

impl Relay {
    /// The Client process the test's home's Daemon serves, through its
    /// runtime file at `runtime`.
    fn spawned(env: &TestEnv, runtime: &Path) -> (Process, Self) {
        let program = tools().unwrap().path(Tool::TestEnv).unwrap().to_owned();
        let mut command = env.process_command(program);
        command
            .arg("daemon-client")
            .arg("--runtime-file")
            .arg(runtime)
            .stderr(Stdio::null());
        let process = env.spawn(&mut command).unwrap();
        let stdin = process.child().stdin.take().unwrap();
        let stdout = process.child().stdout.take().unwrap();
        let (sender, lines) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                if sender.send(line).is_err() {
                    break;
                }
            }
        });
        (process, Self { stdin, lines })
    }
}

impl Wire for Relay {
    fn send(&mut self, message: &Value) {
        let mut line = serde_json::to_vec(message).unwrap();
        line.push(b'\n');
        self.stdin.write_all(&line).unwrap();
        self.stdin.flush().unwrap();
    }

    fn next(&mut self) -> Value {
        let line = self
            .lines
            .recv_timeout(DEADLINE)
            .expect("the Client relayed nothing before the moment")
            .unwrap();
        serde_json::from_str(&line).unwrap()
    }
}

/// A Client of the Daemon `found` names, past its `hello`, each read
/// waiting at most [`DEADLINE`].
fn opened(found: &RuntimeFile) -> DaemonClient {
    let mut client = DaemonClient::connect_within(&found.endpoint, DEADLINE);
    let answer = client.open(&daemon_hello(&found.token));
    assert!(answer.get("result").is_some(), "{answer}");
    client
}

/// Subscribes `wire` to the engine's events and starts Runs one at a time,
/// each answered and evented, up to `moment`: the Run IDs it started.
fn started_until(wire: &mut impl Wire, moment: &Moment) -> Vec<String> {
    wire.send(
        &json!({ "jsonrpc": "2.0", "id": "events", "method": "events.subscribe", "params": {} }),
    );
    wire.send(
        &json!({ "jsonrpc": "2.0", "method": "$/credit", "params": { "id": "events", "credits": RUNS } }),
    );
    let mut observed = 0;
    let mut started = Vec::new();
    for index in 1..=RUNS {
        let run_id = format!("run-{index}");
        wire.send(
            &json!({ "jsonrpc": "2.0", "id": index, "method": "run.start", "params": { "run_id": run_id } }),
        );
        started.push(run_id);
        let (mut answered, mut evented) = (false, false);
        while !(answered && evented) {
            let message = wire.next();
            if message["id"] == json!(index) {
                assert!(message.get("result").is_some(), "{message}");
                answered = true;
            } else if message["method"] == "$/events" {
                evented = true;
                observed += 1;
                if observed == moment.occurrence {
                    return started;
                }
            }
        }
    }
    started
}

/// Kills `process` at `moment`, which the test reached, and records it.
fn killed(process: &Process, moment: &Moment, log: &mut ObservationLog<Value>) {
    {
        let mut child = process.child();
        child.kill().unwrap();
        child.wait().unwrap();
    }
    log.push(Record::new(
        process.program(),
        Operation::Fault(Fault::Kill {
            event: moment.event.clone(),
            occurrence: moment.occurrence,
        }),
    ));
}

/// Resumes and finishes each of `run_ids` through `client`: how each
/// started, and the lifecycle it finished with.
fn resumed_and_finished(client: &mut DaemonClient, run_ids: &[String]) -> Vec<(Value, Value)> {
    run_ids
        .iter()
        .map(|run_id| {
            let params = json!({ "run_id": run_id });
            let resumed = client.call(format!("resume-{run_id}"), "run.resume", &params);
            let finished = client.call(format!("finish-{run_id}"), "run.finish", &params);
            (
                resumed["result"]["started_as"].clone(),
                finished["result"]["lifecycle"].clone(),
            )
        })
        .collect()
}

/// The report of each of `run_ids` through `client`.
fn reports(client: &mut DaemonClient, run_ids: &[String]) -> Vec<Value> {
    run_ids
        .iter()
        .map(|run_id| {
            client.call(
                format!("report-{run_id}"),
                "run.report",
                &json!({ "run_id": run_id }),
            )
        })
        .collect()
}

/// A Daemon killed at a moment drawn from the test's seed among the
/// `$/events` its Client observed leaves each Run it served `running` with
/// no writer: the next start's Daemon reports each interrupted (#218), then
/// resumes and finishes it from its own state, and the runtime file names
/// that Daemon.
///
/// Covers: story:3-23, story:3-11, story:3-164
#[rstest]
#[case::seed_5(Some(5))]
#[case::seed_9(Some(9))]
#[case::the_tests_seed(None)]
fn a_daemon_killed_at_a_seeded_moment_leaves_each_run_for_the_next_daemon_to_resume(
    #[case] seed: Option<u64>,
) {
    let env = TestEnv::new();
    short_idle(&env, 60_000);
    let _stops = StopsTheDaemon::of(&env, COLLIE);
    let moment = Moment::drawn(seed.unwrap_or(env.seed()), &["$/events"], RUNS);
    let mut log = ObservationLog::<Value>::for_test(&env);
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
    let mut client = opened(&found);
    let started = started_until(&mut client, &moment);

    killed(&daemon, &moment, &mut log);
    drop(client);
    let restarted = json_of(COLLIE.as_ref(), &env, &["daemon", "start", "--json"]);
    let mut client = opened(&RuntimeFile::read(&runtime_file(&env)).unwrap());
    let interrupted: Vec<Value> = reports(&mut client, &started)
        .iter()
        .map(|report| report["error"]["data"]["code"].clone())
        .collect();
    let taken_up = resumed_and_finished(&mut client, &started);

    assert_eq!(
        (
            interrupted,
            taken_up,
            restarted["pid"].is_u64(),
            restarted["pid"] == json!(found.pid)
        ),
        (
            vec![json!("run_interrupted"); started.len()],
            vec![(json!("resumed"), json!("finished")); started.len()],
            true,
            false
        )
    );
}

/// A Client killed at a moment drawn from the test's seed among the
/// `$/events` it observed stops its own Runs alone: the Daemon stops each,
/// aborted by its Client and resumable from its own state, while another
/// Client's Run goes on and finishes.
///
/// Covers: story:3-23
#[rstest]
#[case::seed_5(Some(5))]
#[case::seed_9(Some(9))]
#[case::the_tests_seed(None)]
fn a_client_killed_at_a_seeded_moment_stops_its_own_runs_alone_each_resumable(
    #[case] seed: Option<u64>,
) {
    let env = TestEnv::new();
    short_idle(&env, 60_000);
    let _stops = StopsTheDaemon::of(&env, COLLIE);
    let moment = Moment::drawn(seed.unwrap_or(env.seed()), &["$/events"], RUNS);
    let mut log = ObservationLog::<Value>::for_test(&env);
    json_of(COLLIE.as_ref(), &env, &["daemon", "start", "--json"]);
    let runtime = runtime_file(&env);
    let found = RuntimeFile::read(&runtime).unwrap();
    let mut bystander = opened(&found);
    let going_on = bystander.call("bystander", "run.start", &json!({ "run_id": "bystander" }));
    assert!(going_on.get("result").is_some(), "{going_on}");
    let (client, mut wire) = Relay::spawned(&env, &runtime);
    wire.send(&daemon_hello(&found.token).id("hello").message());
    let started = started_until(&mut wire, &moment);

    killed(&client, &moment, &mut log);
    drop(wire);
    let mut other = opened(&found);
    // Each report waits for its Run's end, which the Daemon reaches once
    // it finds the killed Client's Connection closed.
    let stopped: Vec<Value> = reports(&mut other, &started)
        .iter()
        .map(|report| report["result"]["stop_reason"].clone())
        .collect();
    let taken_up = resumed_and_finished(&mut other, &started);
    let finished = bystander.call("finish", "run.finish", &json!({ "run_id": "bystander" }));

    assert_eq!(
        (stopped, taken_up, finished["result"]["lifecycle"].clone()),
        (
            vec![json!({ "kind": "abort", "by": "client" }); started.len()],
            vec![(json!("resumed"), json!("finished")); started.len()],
            json!("finished")
        )
    );
}
