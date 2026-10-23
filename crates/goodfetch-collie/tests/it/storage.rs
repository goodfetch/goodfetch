//! The Storage conformance cases' Collie lines (TESTING.md; spec #213's
//! Seam 1; ADR 0122), on Collie's executable over its protocol, attached
//! and through a Daemon (`daemon run`), on a tmpfs of its own holding the
//! Goodfetch home and the run folder, with `storage.low_free` and
//! `storage.critical_free` set against its size and a filler file to cross
//! them:
//! - **Low:** the Run goes on, its progress reads `storage` at `low` with
//!   its folder, freeing space brings it back to `normal`, and its report
//!   carries `storage_low`.
//! - **Critical:** the Run stops at its next safe point, `stopped` and
//!   `incomplete`, with `stopReason` `resources`, `resource: "storage"` and
//!   its folder, no Checkpoint copy written; a new Run's start there raises
//!   `resources_exhausted`, not retryable, whose `fix` names the folder;
//!   `run.checkpoint` fails with `file_write_failed`, side `engine`,
//!   writing nothing; and once space is freed a resume goes on.
//! - **A failed write:** with both levels off, a commit the full disk
//!   refuses stops the Run; the commit recording the stop fails too, so its
//!   waiting `run.finish` raises `resources_exhausted`, and once space is
//!   freed a resume goes on as after a crash.
//!
//! The `/readyz` and metrics line is #1255's, and the crawl and WARC lines
//! are area 6's.
#![expect(
    clippy::disallowed_methods,
    reason = "a test reads the Daemon's token through its mount namespace, outside any async worker"
)]
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use std::path::{Path, PathBuf};

use goodfetch_test_support::{MESSAGE_DEADLINE, TestEnv};
use rstest::rstest;
use serde_json::{Value, json};

use crate::attached::{Attached, hello};
use crate::daemon::RawClient;
use crate::daemon_run::{Foreground, free_address};
use crate::tmpfs::{fill, free, inside, mounted};

const COLLIE: &str = env!("CARGO_BIN_EXE_goodfetch-collie");

const MIB: u64 = 1 << 20;

/// The tmpfs's size, and the two levels against it.
const SIZE: u64 = 64 * MIB;
const LEVELS: &str = "storage.low_free = 41943040\nstorage.critical_free = 20971520\n\
                      governor.sample_interval = 100\n";

/// Free space between the two levels, and below `critical`.
const LOW_ROOM: u64 = 30 * MIB;
const CRITICAL_ROOM: u64 = 10 * MIB;

/// How the test reaches the engine.
#[derive(Clone, Copy, Debug)]
enum Mode {
    Attached,
    Daemon,
}

/// The engine of a test, on its tmpfs.
enum Engine<'env> {
    Attached(Attached<'env>),
    Daemon {
        daemon: Foreground,
        client: RawClient,
    },
}

impl<'env> Engine<'env> {
    /// Starts Collie's executable in `mode` with a tmpfs over the test's
    /// home, its user configuration holding `settings`, and opens the
    /// Connection subscribed to every event.
    fn start(env: &'env TestEnv, mode: Mode, settings: &str) -> Self {
        let mut engine = match mode {
            Mode::Attached => {
                env.user_configuration(settings);
                let mut command = mounted(env, env.home(), SIZE, COLLIE);
                command.arg("attached-engine");
                let mut engine =
                    Self::Attached(Attached::spawned(env, MESSAGE_DEADLINE, &mut command));
                engine.send(&hello("collie", "0.1"));
                let _ = engine.next();
                engine
            }
            Mode::Daemon => {
                env.user_configuration(&format!(
                    "{settings}daemon.address = \"{}\"\n",
                    free_address()
                ));
                let mut command = mounted(env, env.home(), SIZE, COLLIE);
                command.args(["daemon", "run"]);
                let mut daemon = Foreground::spawned(env, &mut command);
                let endpoint = daemon.serving();
                let pid = daemon.process.child().id();
                let token = std::fs::read_to_string(inside(
                    pid,
                    &env.home().join("0.0").join("daemon.token"),
                ))
                .unwrap();
                let client = RawClient::opened_at(&endpoint, &token, "collie");
                Self::Daemon { daemon, client }
            }
        };
        engine.send(
            &json!({ "jsonrpc": "2.0", "id": 0, "method": "events.subscribe", "params": {} }),
        );
        engine.send(&json!({ "jsonrpc": "2.0", "method": "$/credit", "params": { "id": 0, "credits": 10_000 } }));
        engine
    }

    fn pid(&self) -> u32 {
        match self {
            Self::Attached(engine) => engine.pid(),
            Self::Daemon { daemon, .. } => daemon.process.child().id(),
        }
    }

    fn send(&mut self, message: &Value) {
        match self {
            Self::Attached(engine) => engine.send(message),
            Self::Daemon { client, .. } => client.try_send(message),
        }
    }

    fn next(&mut self) -> Option<Value> {
        match self {
            Self::Attached(engine) => engine.next(),
            Self::Daemon { client, .. } => client.next(),
        }
    }

    /// Sends the call `id` and answers its reply, past the events on the
    /// way.
    fn call(&mut self, id: i64, method: &str, params: &Value) -> Value {
        self.send(&json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }));
        loop {
            let message = self.next().expect("the engine closed before replying");
            if message["id"] == json!(id) {
                return message;
            }
        }
    }

    /// The `storage` of the next `run.progress` at `level`.
    fn progress_at(&mut self, level: &str) -> Value {
        loop {
            let message = self.next().expect("the engine closed before its progress");
            let events = message["params"]["events"]
                .as_array()
                .cloned()
                .unwrap_or_default();
            if let Some(event) = events.iter().find(|event| {
                event["event"] == "run.progress" && event["data"]["storage"]["level"] == level
            }) {
                return event["data"]["storage"].clone();
            }
        }
    }

    /// Ends the engine and its tmpfs.
    fn stop(self) {
        match self {
            Self::Attached(engine) => {
                let _ = engine.finish();
            }
            Self::Daemon { mut daemon, client } => {
                drop(client);
                daemon.terminate();
            }
        }
    }
}

fn run_folder(env: &TestEnv, run_id: &str) -> PathBuf {
    env.home().join("0.0").join("runs").join(run_id)
}

/// The codes of `report`'s warnings.
fn warnings(report: &Value) -> Vec<Value> {
    report["warnings"]
        .as_array()
        .map(|warnings| {
            warnings
                .iter()
                .map(|warning| warning["code"].clone())
                .collect()
        })
        .unwrap_or_default()
}

fn exists(pid: u32, path: &Path) -> bool {
    inside(pid, path).exists()
}

/// Low: crossing `storage.low_free` leaves the Run working, its progress
/// reads `storage` at `low` with its folder, freeing space brings it back
/// to `normal`, and its report carries `storage_low`.
///
/// Covers: row:RT-03, setting:storage.low_free, event:run.progress
#[rstest]
#[case::attached(Mode::Attached)]
#[case::through_a_daemon(Mode::Daemon)]
fn past_low_free_a_run_goes_on_and_carries_storage_low(#[case] mode: Mode) {
    let env = TestEnv::new();
    let mut engine = Engine::start(&env, mode, LEVELS);
    let pid = engine.pid();

    let started = engine.call(1, "run.start", &json!({ "run_id": "low" }));
    let filler = fill(pid, env.home(), LOW_ROOM);
    let low = engine.progress_at("low");
    free(&filler);
    let normal = engine.progress_at("normal");
    let finished = engine.call(2, "run.finish", &json!({ "run_id": "low" }));
    engine.stop();

    let folder = json!(run_folder(&env, "low").display().to_string());
    assert_eq!(
        (
            started.get("result").is_some(),
            &low["folder"],
            &normal["level"],
            &finished["result"]["lifecycle"],
            warnings(&finished["result"]),
        ),
        (
            true,
            &folder,
            &json!("normal"),
            &json!("finished"),
            vec![json!("storage_low")],
        ),
        "{finished}"
    );
}

/// Critical: crossing `storage.critical_free` stops the Run resumably,
/// writing no Checkpoint copy there; a start there raises
/// `resources_exhausted`, and `run.checkpoint` fails with
/// `file_write_failed`, writing nothing; once space is freed the Run
/// resumes and finishes.
///
/// Covers: row:RT-03, setting:storage.critical_free, error:resources_exhausted, error:file_write_failed, operation:run.resume
#[rstest]
#[case::attached(Mode::Attached)]
#[case::through_a_daemon(Mode::Daemon)]
fn past_critical_free_a_run_stops_resumably_and_nothing_new_is_written_there(#[case] mode: Mode) {
    let env = TestEnv::new();
    let mut engine = Engine::start(&env, mode, LEVELS);
    let pid = engine.pid();

    engine.call(1, "run.start", &json!({ "run_id": "full" }));
    let filler = fill(pid, env.home(), CRITICAL_ROOM);
    engine.progress_at("critical");
    let stopped = engine.call(2, "run.finish", &json!({ "run_id": "full" }));
    let refused = engine.call(3, "run.start", &json!({ "run_id": "another" }));
    let checkpoint = run_folder(&env, "full").join("copy.sqlite");
    let copied = engine.call(
        4,
        "run.checkpoint",
        &json!({ "run_id": "full", "location": checkpoint.display().to_string() }),
    );
    let written = (
        exists(pid, &checkpoint),
        exists(pid, &run_folder(&env, "full").join("checkpoint.sqlite")),
        exists(pid, &run_folder(&env, "another")),
    );
    free(&filler);
    let resumed = engine.call(5, "run.resume", &json!({ "run_id": "full" }));
    let finished = engine.call(6, "run.finish", &json!({ "run_id": "full" }));
    engine.stop();

    let folder = run_folder(&env, "full").display().to_string();
    let runs = env.home().join("0.0").join("runs").display().to_string();
    assert_eq!(
        (
            (
                &stopped["result"]["lifecycle"],
                &stopped["result"]["completeness"],
                &stopped["result"]["stop_reason"],
                &stopped["result"]["checkpoint"],
            ),
            (
                &refused["error"]["data"]["code"],
                &refused["error"]["data"]["retryable"],
                refused["error"]["data"]["fix"]
                    .as_str()
                    .is_some_and(|fix| fix.contains(&runs)),
            ),
            (
                &copied["error"]["data"]["code"],
                &copied["error"]["data"]["side"],
                &copied["error"]["data"]["reason"],
            ),
            written,
            (
                &resumed["result"]["started_as"],
                &finished["result"]["lifecycle"]
            ),
        ),
        (
            (
                &json!("stopped"),
                &json!("incomplete"),
                &json!({ "kind": "resources", "resource": "storage", "folder": folder }),
                &Value::Null,
            ),
            (&json!("resources_exhausted"), &json!(false), true),
            (
                &json!("file_write_failed"),
                &json!("engine"),
                &json!("storage_full")
            ),
            (false, false, false),
            (&json!("resumed"), &json!("finished")),
        ),
        "{stopped}\n{refused}\n{copied}"
    );
}

/// A failed write: with both levels off, a commit the full disk refuses
/// stops the Run, the commit recording the stop fails too, so the waiting
/// `run.finish` raises `resources_exhausted`; once space is freed the Run
/// resumes as after a crash and finishes.
///
/// Covers: row:RT-03, error:resources_exhausted, operation:run.set_progress_message, operation:run.resume
#[rstest]
#[case::attached(Mode::Attached)]
#[case::through_a_daemon(Mode::Daemon)]
fn a_write_the_full_disk_refuses_stops_the_run_and_its_waiting_call_raises_resources_exhausted(
    #[case] mode: Mode,
) {
    let env = TestEnv::new();
    let mut engine = Engine::start(
        &env,
        mode,
        "storage.low_free = 0\nstorage.critical_free = 0\n",
    );
    let pid = engine.pid();

    engine.call(1, "run.start", &json!({ "run_id": "refused" }));
    let filler = fill(pid, env.home(), 0);
    let failed = engine.call(
        2,
        "run.set_progress_message",
        &json!({ "run_id": "refused", "message": "a status line the full disk refuses" }),
    );
    let stopped = engine.call(3, "run.finish", &json!({ "run_id": "refused" }));
    free(&filler);
    let resumed = engine.call(4, "run.resume", &json!({ "run_id": "refused" }));
    let finished = engine.call(5, "run.finish", &json!({ "run_id": "refused" }));
    engine.stop();

    assert_eq!(
        (
            failed.get("error").is_some(),
            &stopped["error"]["data"]["code"],
            &stopped["error"]["data"]["resource"],
            &resumed["result"]["started_as"],
            &finished["result"]["lifecycle"],
        ),
        (
            true,
            &json!("resources_exhausted"),
            &json!("storage"),
            &json!("resumed"),
            &json!("finished"),
        ),
        "{failed}\n{stopped}"
    );
}
