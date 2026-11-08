//! Checkpoints, pause and resume, through the engine's public interface
//! over the in-memory network (spec #213's Seam 3; ADRs 0002, 0003, 0047,
//! 0056, 0066, 0102 and 0116): `run.checkpoint` answers a Checkpoint that
//! is complete at its location, announced as `run.checkpointing`;
//! `run.pause` and `run.migrate` checkpoint and stop a Run, resumable, and
//! `run.resume` takes its next run epoch, from its run file after a
//! restart or from a Checkpoint in another Goodfetch home, which stands for
//! another machine, redoing the work since; every Checkpoint carries the
//! run epoch, so a compare-and-set destination keeps only the current
//! copy's; a Run bound to its machine is refused elsewhere and one behind a
//! Browser host is not; the Clean-up rules digest is kept; a Checkpoint of
//! a version this engine cannot read is refused; `checkpoint.interval`
//! waits on the engine's `Clock`; and a Checkpoint that cannot be written
//! fails with `file_write_failed` and leaves no file. What the tests read
//! of run files and Checkpoints is what the engine wrote.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Weak};
use std::time::Duration;

use goodfetch_protocol::catalog::{
    Operation, OperationClass, OperationKind, Product, ProtocolVersion, Reach, Stability, schema_of,
};
use goodfetch_protocol::checkpoint::RunBinding;
use goodfetch_protocol::formats::{Compatibility, Format, RUN_STATE};
use goodfetch_protocol::run::{RunId, RunKind, Tags};
use goodfetch_test_support::TestEnv;
use rusqlite::Connection;
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use tokio::io::{AsyncBufReadExt as _, BufReader};

use super::{Client, attached_on, dependencies, engine_with, hello, pretty};
use crate::call_error::CallError;
use crate::composition::{CallContext, Contribution, Handler};
use crate::engine::Engine;
use crate::run::format_refusal;
use crate::run_state::RunStateTables;
use crate::runs::{EndedBy, Run, Start, Started};
use crate::transport::{self, Network};

/// Starts a Collie Run its program drives: `run_id`, bound to this
/// engine's machine when `machine` is true or to the Browser host
/// `browser_host`, and recording the Clean-up rules digest
/// `clean_up_digest`.
const KEEP: Operation = Operation {
    name: "test.keep",
    description: "Starts a Run the tests write notes into.",
    class: OperationClass::Effect,
    kind: OperationKind::RoundTrip,
    reach: Reach::AllSurfaces,
    params: schema_of::<Value>,
    result: schema_of::<Value>,
    events: &[],
    cancellable: false,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: goodfetch_protocol::catalog::PROTOCOL_VERSION,
};

/// Commits the note `value` into the run file of the Run `run_id` that
/// `test.keep` started on this engine.
const NOTE: Operation = Operation {
    name: "test.note",
    description: "Commits a note into a Run's run file.",
    ..KEEP
};

/// The notes the tests commit, a table of every run file.
const NOTES: RunStateTables = RunStateTables {
    name: "test-notes",
    statements: &["CREATE TABLE IF NOT EXISTS notes (value INTEGER NOT NULL) STRICT"],
};

/// The Runs `test.keep` started, each held by a task of its own, which
/// its stop ends, so its writer closes as a driver's does.
type Kept = Arc<tokio::sync::Mutex<BTreeMap<String, Weak<Run>>>>;

fn keeping() -> Contribution {
    let kept = Kept::default();
    let noted = Arc::clone(&kept);
    Contribution::new("test-checkpoint")
        .with_run_state_tables(NOTES)
        .with_handler(Handler::new(
            &KEEP,
            move |params: Value, context: CallContext| {
                let kept = Arc::clone(&kept);
                async move {
                    let binding = if params["machine"] == json!(true) {
                        Some(RunBinding::Machine {
                            machine: context.runs().machine().await.unwrap(),
                        })
                    } else {
                        params["browser_host"].as_str().map(|browser_host| {
                            RunBinding::BrowserHost {
                                browser_host: browser_host.to_owned(),
                            }
                        })
                    };
                    let started = context
                        .runs()
                        .start(Start {
                            run_id: Some(RunId::parse(params["run_id"].as_str().unwrap()).unwrap()),
                            tags: Tags::default(),
                            kind: RunKind::Collie,
                            needs: Product::Collie,
                            inputs: BTreeMap::new(),
                            ended_by: EndedBy::Finish,
                            initialize: None,
                            policy_version: None,
                            weakened: Vec::new(),
                            binding,
                            clean_up_digest: params["clean_up_digest"].as_str().map(str::to_owned),
                            checkpoint_interval: None,
                            timeout: None,
                            settings: None,
                        })
                        .await
                        .unwrap();
                    let Started::New(run) = started else {
                        panic!("the Run began before");
                    };
                    let run_id = run.id().to_string();
                    let held = Arc::new(run.clone());
                    kept.lock()
                        .await
                        .insert(run_id.clone(), Arc::downgrade(&held));
                    run.spawn(async move {
                        held.cancellation().cancelled().await;
                    });
                    Ok::<Value, CallError>(json!({ "run_id": run_id }))
                }
            },
        ))
        .with_handler(Handler::new(&NOTE, move |params: Value, _: CallContext| {
            let noted = Arc::clone(&noted);
            async move {
                let run = noted.lock().await[params["run_id"].as_str().unwrap()]
                    .upgrade()
                    .expect("the Run is live");
                let value = params["value"].as_i64().unwrap();
                run.state()
                    .write(move |connection| {
                        connection
                            .execute("INSERT INTO notes (value) VALUES (?1)", [value])
                            .map(drop)
                    })
                    .await
                    .unwrap();
                Ok::<Value, CallError>(json!({ "noted": value }))
            }
        }))
}

fn engine() -> Engine {
    engine_with(Product::Collie, Some(keeping()))
}

/// A Connection's messages as the tests read them: each call's reply, and
/// the events of its subscription, call 1, on the way.
pub(super) struct Talk {
    client: Client,
    events: Vec<Value>,
}

impl Talk {
    /// Opens the Connection with `hello` and subscribes to every event,
    /// granting credits enough for any test.
    pub(super) async fn open(mut client: Client) -> Self {
        client.send(&[hello("collie", "0.1")]).await;
        let opened = client.next().await.unwrap();
        assert!(opened.get("result").is_some(), "{opened}");
        client
            .send(&[
                json!({ "jsonrpc": "2.0", "id": 1, "method": "events.subscribe", "params": {} }),
                json!({ "jsonrpc": "2.0", "method": "$/credit", "params": { "id": 1, "credits": 10_000 } }),
            ])
            .await;
        Self {
            client,
            events: Vec::new(),
        }
    }

    /// Sends the call `id` and answers its reply, keeping the events read
    /// on the way.
    pub(super) async fn call(&mut self, id: i64, method: &str, params: Value) -> Value {
        self.client
            .send(&[json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params })])
            .await;
        loop {
            let message = self
                .client
                .next()
                .await
                .expect("the engine closed before replying");
            if message["method"] == "$/events" {
                self.events
                    .extend(message["params"]["events"].as_array().unwrap().clone());
            } else if message["id"] == json!(id) {
                return message;
            }
        }
    }

    /// The result of the call `id`, which must succeed.
    pub(super) async fn ok(&mut self, id: i64, method: &str, params: Value) -> Value {
        let reply = self.call(id, method, params).await;
        assert!(reply.get("result").is_some(), "{method}: {reply}");
        reply["result"].clone()
    }

    /// Reads events until `count` of `name` came, and answers how long that
    /// took on the paused clock.
    async fn events_until(&mut self, name: &str, count: usize) -> Duration {
        let began = tokio::time::Instant::now();
        while self
            .events
            .iter()
            .filter(|event| event["event"] == name)
            .count()
            < count
        {
            let message = self.client.next().await.expect("the engine closed");
            if message["method"] == "$/events" {
                self.events
                    .extend(message["params"]["events"].as_array().unwrap().clone());
            }
        }
        began.elapsed()
    }

    /// Each event read, as `[name, data]`, but axis changes and the
    /// progress values, which a Checkpoint changes too.
    fn lifecycle_events(&self) -> Vec<Value> {
        self.events
            .iter()
            .filter(|event| {
                !["run.axes_changed", "run.progress"]
                    .contains(&event["event"].as_str().unwrap_or_default())
            })
            .map(|event| json!([event["event"], event["data"]]))
            .collect()
    }
}

fn run_folder(env: &TestEnv, run_id: &str) -> PathBuf {
    env.home().join("1").join("runs").join(run_id)
}

fn sha256(path: &Path) -> String {
    format!("{:x}", Sha256::digest(std::fs::read(path).unwrap()))
}

/// The notes of the run file or Checkpoint at `path`.
fn notes(path: &Path) -> Vec<i64> {
    let file = Connection::open(path).unwrap();
    let mut statement = file
        .prepare("SELECT value FROM notes ORDER BY rowid")
        .unwrap();
    statement
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<rusqlite::Result<Vec<i64>>>()
        .unwrap()
}

/// The `run` row of the run file or Checkpoint at `path`: its epoch,
/// lifecycle, stop reason and Clean-up rules digest.
fn run_row(path: &Path) -> Value {
    let file = Connection::open(path).unwrap();
    let row: String = file
        .query_row(
            "SELECT json_object('epoch', epoch, 'lifecycle', lifecycle,
                'stop_reason', json(stop_reason), 'clean_up_digest', clean_up_digest)
             FROM run",
            [],
            |row| row.get(0),
        )
        .unwrap();
    serde_json::from_str(&row).unwrap()
}

/// The dependencies of an engine on another machine than `env`'s first
/// one: the same test seed would draw the same machine identity.
fn another_machine(env: &TestEnv) -> crate::engine::Dependencies {
    crate::engine::Dependencies {
        environment: std::collections::BTreeMap::new(),
        keychain: crate::secrets::Keychain::absent(),
        entropy: crate::entropy::Entropy::seeded(env.seed() ^ 1),
        ..dependencies(env)
    }
}

/// Runs `engine` on two Goodfetch homes at once, standing for two
/// machines, while `talk` talks to both.
pub(super) fn two_machines<F, Fut, T>(first: &TestEnv, second: &TestEnv, talk: F) -> T
where
    F: FnOnce(Talk, Talk) -> Fut,
    Fut: Future<Output = T>,
{
    two_machines_on(&engine(), first, second, talk)
}

/// [`two_machines`], with `engine` on both.
pub(super) fn two_machines_on<F, Fut, T>(
    engine: &Engine,
    first: &TestEnv,
    second: &TestEnv,
    talk: F,
) -> T
where
    F: FnOnce(Talk, Talk) -> Fut,
    Fut: Future<Output = T>,
{
    goodfetch_test_support::runtime().block_on(async {
        // The test `tracing` subscriber writes the dispatch records into the
        // test's exercise record (ADR 0072), as `attached_with`'s does.
        let clock = crate::clock::Clock::paused(first.start_time());
        let _subscriber =
            tracing::subscriber::set_default(goodfetch_test_support::subscriber(move || {
                clock.wall()
            }));
        let (first_network, first_dialer) = transport::in_memory();
        let (second_network, second_dialer) = transport::in_memory();
        let connect = |dialer: transport::InMemoryDialer| async move {
            let (reader, writer) = dialer.connect().await.unwrap().split();
            Talk::open(Client {
                lines: BufReader::new(reader).lines(),
                writer,
            })
            .await
        };
        let both = async {
            let first = connect(first_dialer).await;
            let second = connect(second_dialer).await;
            talk(first, second).await
        };
        let (first_served, second_served, observed) = tokio::join!(
            engine.run_attached(Network::InMemory(first_network), dependencies(first)),
            engine.run_attached(Network::InMemory(second_network), another_machine(second)),
            both
        );
        first_served.unwrap();
        second_served.unwrap();
        observed
    })
}

/// Covers: story:3-30, operation:run.checkpoint, event:run.checkpointing
#[test]
fn a_checkpoint_is_complete_at_its_location_when_answered_and_announced_first() {
    let env = TestEnv::new();

    let (checkpoint, events) = attached_on(&engine(), &env, |client| async move {
        let mut talk = Talk::open(client).await;
        talk.ok(2, "test.keep", json!({ "run_id": "kept" })).await;
        talk.ok(3, "test.note", json!({ "run_id": "kept", "value": 1 }))
            .await;
        talk.ok(4, "test.note", json!({ "run_id": "kept", "value": 2 }))
            .await;
        let checkpoint = talk
            .ok(5, "run.checkpoint", json!({ "run_id": "kept" }))
            .await;
        talk.ok(6, "run.finish", json!({ "run_id": "kept" })).await;
        (checkpoint, talk.lifecycle_events())
    });

    let location = run_folder(&env, "kept").join("checkpoint.sqlite");
    assert_eq!(
        (checkpoint, notes(&location), events),
        (
            json!({
                "run_id": "kept",
                "location": location.display().to_string(),
                "size": std::fs::metadata(&location).unwrap().len(),
                "digest": sha256(&location),
                "run_epoch": 1,
                "journal_position": 3,
                "time": "2026-01-05T09:00:00Z",
            }),
            vec![1, 2],
            vec![json!(["run.checkpointing", { "run_id": "kept", "run_epoch": 1 }])],
        )
    );
}

/// A pause checkpoints and stops the Run, resumable; after a restart of
/// its engine, the resume takes the next run epoch and the Run goes on to
/// its end.
/// Covers: story:3-35, operation:run.pause, operation:run.resume, event:run.stopping
#[test]
fn a_paused_run_survives_a_restart_and_resumes_at_the_next_run_epoch() {
    let env = TestEnv::new();
    let (paused, events) = attached_on(&engine(), &env, |client| async move {
        let mut talk = Talk::open(client).await;
        talk.ok(2, "test.keep", json!({ "run_id": "paused" })).await;
        talk.ok(3, "test.note", json!({ "run_id": "paused", "value": 1 }))
            .await;
        let paused = talk.ok(4, "run.pause", json!({ "run_id": "paused" })).await;
        (paused, talk.lifecycle_events())
    });
    let file = run_folder(&env, "paused").join("run.sqlite");
    let after_pause = run_row(&file);

    let (resumed, finished) = attached_on(&engine(), &env, |client| async move {
        let mut talk = Talk::open(client).await;
        let resumed = talk
            .ok(2, "run.resume", json!({ "run_id": "paused" }))
            .await;
        let finished = talk
            .ok(3, "run.finish", json!({ "run_id": "paused" }))
            .await;
        (resumed, finished)
    });

    let checkpoint = run_folder(&env, "paused").join("checkpoint.sqlite");
    assert_eq!(
        (
            paused["lifecycle"].clone(),
            paused["stop_reason"].clone(),
            paused["checkpoint"]["location"].clone(),
            paused["checkpoint"]["run_epoch"].clone(),
            events,
            after_pause,
            run_row(&checkpoint),
            notes(&checkpoint),
            resumed,
            finished["lifecycle"].clone(),
            run_row(&file),
        ),
        (
            json!("stopped"),
            json!({ "kind": "pause" }),
            json!(checkpoint.display().to_string()),
            json!(1),
            vec![
                json!(["run.stopping", { "run_id": "paused", "cause": { "kind": "pause" } }]),
                json!(["run.checkpointing", { "run_id": "paused", "run_epoch": 1 }]),
            ],
            json!({ "epoch": 1, "lifecycle": "stopped", "stop_reason": { "kind": "pause" }, "clean_up_digest": null }),
            json!({ "epoch": 1, "lifecycle": "stopped", "stop_reason": { "kind": "pause" }, "clean_up_digest": null }),
            vec![1],
            json!({ "run_id": "paused", "started_as": "resumed", "lifecycle": "running", "needs": "collie" }),
            json!("finished"),
            json!({ "epoch": 2, "lifecycle": "finished", "stop_reason": null, "clean_up_digest": null }),
        )
    );
}

/// A destination that keeps one Checkpoint per Run and compares and sets
/// its run epoch: one older than the one it holds is refused (ADR 0003).
#[derive(Default)]
struct CompareAndSetStore {
    held: BTreeMap<String, u64>,
    landed: Vec<(String, u64)>,
    refused: Vec<(String, u64)>,
}

impl CompareAndSetStore {
    fn offer(&mut self, checkpoint: &Value) {
        let run_id = checkpoint["run_id"].as_str().unwrap().to_owned();
        let epoch = checkpoint["run_epoch"].as_u64().unwrap();
        let held = self.held.get(&run_id).copied().unwrap_or_default();
        if epoch >= held {
            self.held.insert(run_id.clone(), epoch);
            self.landed.push((run_id, epoch));
        } else {
            self.refused.push((run_id, epoch));
        }
    }
}

/// Two copies of one Run: the first checkpoints, the second resumes from
/// that Checkpoint in another home and redoes what the first committed
/// after it, then both write Checkpoints to a compare-and-set store, where
/// only the current run epoch's land.
/// Covers: story:3-34, story:3-43, operation:run.resume
#[test]
fn a_run_resumed_on_another_machine_redoes_the_work_since_its_checkpoint_and_fences_the_old_copy() {
    let (first, second) = (TestEnv::new(), TestEnv::new());
    let moved = first.folder().join("moved.sqlite");
    let location = moved.display().to_string();

    let (store, resumed) = two_machines(&first, &second, |mut old, mut new| async move {
        let mut store = CompareAndSetStore::default();
        old.ok(2, "test.keep", json!({ "run_id": "moved" })).await;
        old.ok(3, "test.note", json!({ "run_id": "moved", "value": 1 }))
            .await;
        let taken = old
            .ok(
                4,
                "run.checkpoint",
                json!({ "run_id": "moved", "location": location }),
            )
            .await;
        store.offer(&taken);
        old.ok(5, "test.note", json!({ "run_id": "moved", "value": 2 }))
            .await;
        let resumed = new
            .ok(
                2,
                "run.resume",
                json!({ "run_id": "moved", "checkpoint": location }),
            )
            .await;
        store.offer(
            &new.ok(3, "run.checkpoint", json!({ "run_id": "moved" }))
                .await,
        );
        store.offer(
            &old.ok(6, "run.checkpoint", json!({ "run_id": "moved" }))
                .await,
        );
        new.ok(4, "run.finish", json!({ "run_id": "moved" })).await;
        old.ok(7, "run.finish", json!({ "run_id": "moved" })).await;
        (store, resumed)
    });

    let resumed_file = run_folder(&second, "moved").join("run.sqlite");
    assert_eq!(
        (
            resumed["started_as"].clone(),
            notes(&resumed_file),
            run_row(&resumed_file)["epoch"].clone(),
            store.landed,
            store.refused,
        ),
        (
            json!("resumed"),
            vec![1],
            json!(2),
            vec![("moved".to_owned(), 1), ("moved".to_owned(), 2)],
            vec![("moved".to_owned(), 1)],
        )
    );
}

/// Starts `run_id` with `params` on the first home, migrates it to a
/// Checkpoint, and resumes it from that Checkpoint on the second: the
/// resume's reply.
fn migrated_then_resumed(params: Value) -> (Value, TestEnv) {
    let (first, second) = (TestEnv::new(), TestEnv::new());
    let moved = first.folder().join("moved.sqlite");
    let location = moved.display().to_string();
    let run_id = params["run_id"].clone();
    let resumed = two_machines(&first, &second, |mut old, mut new| async move {
        old.ok(2, "test.keep", params).await;
        old.ok(
            3,
            "run.migrate",
            json!({ "run_id": run_id, "location": location }),
        )
        .await;
        new.call(
            2,
            "run.resume",
            json!({ "run_id": run_id, "checkpoint": location }),
        )
        .await
    });
    (resumed, second)
}

/// Covers: story:3-36, error:session_bound_to_machine
#[test]
fn a_run_bound_to_its_machine_is_refused_on_another_machine() {
    let (resumed, second) = migrated_then_resumed(json!({ "run_id": "bound", "machine": true }));

    assert_eq!(
        (
            resumed["error"]["data"]["code"].clone(),
            resumed["error"]["data"]["run_id"].clone(),
            run_folder(&second, "bound").join("run.sqlite").exists(),
        ),
        (json!("session_bound_to_machine"), json!("bound"), false)
    );
}

/// Covers: story:3-36
#[test]
fn a_run_behind_a_browser_host_resumes_on_another_machine() {
    let (resumed, _second) =
        migrated_then_resumed(json!({ "run_id": "hosted", "browser_host": "laptop" }));

    assert_eq!(resumed["result"]["started_as"], json!("resumed"));
}

/// Covers: story:3-37
#[test]
fn a_resumed_run_keeps_the_clean_up_rules_digest_it_recorded() {
    let digest = "6c8abc7a55f570dbb19512bdf4a9321337d1cb2c9b507c105b01b87fabbcc34b";
    let (resumed, second) =
        migrated_then_resumed(json!({ "run_id": "rules", "clean_up_digest": digest }));

    assert_eq!(
        (
            resumed["result"]["started_as"].clone(),
            run_row(&run_folder(&second, "rules").join("run.sqlite"))["clean_up_digest"].clone(),
        ),
        (json!("resumed"), json!(digest))
    );
}

/// A real Checkpoint whose run-state version is fabricated newer than this
/// engine's: refused before anything is written, the message snapshotted.
/// Covers: story:3-38, error:format_version_unsupported
#[rstest::rstest]
#[case::a_newer_minor("0.2")]
#[case::a_newer_major("1.0")]
fn a_checkpoint_of_a_newer_version_is_refused_with_a_message_to_upgrade(#[case] version: &str) {
    let env = TestEnv::new();
    let taken = env.folder().join("taken.sqlite");
    let location = taken.display().to_string();
    let fabricated = taken.clone();
    let version_text = version.to_owned();

    let refused = attached_on(&engine(), &env, |client| async move {
        let mut talk = Talk::open(client).await;
        talk.ok(2, "test.keep", json!({ "run_id": "newer" })).await;
        talk.ok(
            3,
            "run.migrate",
            json!({ "run_id": "newer", "location": location }),
        )
        .await;
        Connection::open(&fabricated)
            .unwrap()
            .execute(
                "UPDATE format SET version = ?1 WHERE name = 'run-state'",
                [&version_text],
            )
            .unwrap();
        let before = sha256(&fabricated);
        let refused = talk
            .call(
                4,
                "run.resume",
                json!({ "run_id": "newer", "checkpoint": location }),
            )
            .await;
        (refused, before == sha256(&fabricated))
    });

    assert!(refused.1, "the refused Checkpoint changed");
    insta::assert_snapshot!(
        format!("checkpoint_of_{}_refused", version.replace('.', "_")),
        pretty(&refused.0["error"])
    );
}

/// A file of an older major than the reader's names `upgrade-files`: the
/// reader's own version fabricated a major ahead, since run state is at
/// major 0.
/// Covers: story:3-38, error:format_version_unsupported
#[test]
fn a_file_of_an_older_major_is_refused_naming_upgrade_files() {
    let reader = Format {
        version: ProtocolVersion { major: 1, minor: 0 },
        ..RUN_STATE
    };

    let refused = format_refusal(&reader, "0.1", Compatibility::OlderMajor);

    insta::assert_debug_snapshot!(refused);
}

/// Covers: story:3-30, setting:checkpoint.interval
#[test]
fn a_run_checkpoints_every_interval_of_the_engines_clock() {
    let env = TestEnv::new();

    let (waited, events) = attached_on(&engine(), &env, |client| async move {
        let mut talk = Talk::open(client).await;
        talk.ok(
            2,
            "run.start",
            json!({ "run_id": "periodic", "settings": { "checkpoint": { "interval": 60_000 } } }),
        )
        .await;
        let waited = talk.events_until("run.checkpointing", 2).await;
        talk.ok(3, "run.finish", json!({ "run_id": "periodic" }))
            .await;
        (waited, talk.lifecycle_events())
    });

    assert_eq!(
        (
            waited,
            events,
            run_folder(&env, "periodic")
                .join("checkpoint.sqlite")
                .exists()
        ),
        (
            Duration::from_mins(2),
            vec![
                json!(["run.checkpointing", { "run_id": "periodic", "run_epoch": 1 }]),
                json!(["run.checkpointing", { "run_id": "periodic", "run_epoch": 1 }]),
            ],
            true
        )
    );
}

/// Covers: story:3-131, operation:run.abort, event:run.aborting, event:run.stopping
#[test]
fn an_abort_announces_itself_and_its_clean_stop_writes_a_checkpoint() {
    let env = TestEnv::new();

    let (aborted, events) = attached_on(&engine(), &env, |client| async move {
        let mut talk = Talk::open(client).await;
        talk.ok(2, "test.keep", json!({ "run_id": "aborted" }))
            .await;
        let aborted = talk
            .ok(3, "run.abort", json!({ "run_id": "aborted" }))
            .await;
        (aborted, talk.lifecycle_events())
    });

    assert_eq!(
        (aborted["checkpoint"]["run_epoch"].clone(), events),
        (
            json!(1),
            vec![
                json!(["run.aborting", { "run_id": "aborted", "by": "client" }]),
                json!(["run.stopping", { "run_id": "aborted", "cause": { "kind": "abort", "by": "client" } }]),
                json!(["run.checkpointing", { "run_id": "aborted", "run_epoch": 1 }]),
            ]
        )
    );
}

/// Covers: story:3-131, story:3-30, operation:run.migrate, event:run.migrating
#[test]
fn a_migration_announces_itself_and_stops_with_its_checkpoint_where_asked() {
    let env = TestEnv::new();
    let taken = env.folder().join("to move.sqlite");
    let location = taken.display().to_string();

    let (migrated, events) = attached_on(&engine(), &env, |client| async move {
        let mut talk = Talk::open(client).await;
        talk.ok(2, "test.keep", json!({ "run_id": "migrated" }))
            .await;
        let migrated = talk
            .ok(
                3,
                "run.migrate",
                json!({ "run_id": "migrated", "location": location }),
            )
            .await;
        (migrated, talk.lifecycle_events())
    });

    assert_eq!(
        (
            migrated["stop_reason"].clone(),
            migrated["checkpoint"]["location"].clone(),
            migrated["checkpoint"]["digest"].clone(),
            events,
        ),
        (
            json!({ "kind": "migrate" }),
            json!(taken.display().to_string()),
            json!(sha256(&taken)),
            vec![
                json!(["run.migrating", { "run_id": "migrated" }]),
                json!(["run.stopping", { "run_id": "migrated", "cause": { "kind": "migrate" } }]),
                json!(["run.checkpointing", { "run_id": "migrated", "run_epoch": 1 }]),
            ]
        )
    );
}

/// A location whose folder refuses the write: `file_write_failed` with the
/// OS's reason, and no file there.
/// Covers: story:3-30, error:file_write_failed
#[cfg(unix)]
#[test]
fn a_checkpoint_that_cannot_be_written_fails_and_leaves_no_file() {
    use std::os::unix::fs::PermissionsExt as _;

    let env = TestEnv::new();
    let refusing = env.folder().join("refusing");
    std::fs::create_dir(&refusing).unwrap();
    std::fs::set_permissions(&refusing, std::fs::Permissions::from_mode(0o555)).unwrap();
    let location = refusing.join("taken.sqlite").display().to_string();

    let failed = attached_on(&engine(), &env, |client| async move {
        let mut talk = Talk::open(client).await;
        talk.ok(2, "test.keep", json!({ "run_id": "refused" }))
            .await;
        talk.call(
            3,
            "run.checkpoint",
            json!({ "run_id": "refused", "location": location }),
        )
        .await
    });

    std::fs::set_permissions(&refusing, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(
        (
            failed["error"]["data"].clone(),
            std::fs::read_dir(&refusing).unwrap().count(),
        ),
        (
            json!({
                "code": "file_write_failed",
                "retryable": false,
                "reason": "permission_denied",
                "os_code": "EACCES",
                "path": run_folder(&env, "refused").display().to_string(),
                "side": "engine",
            }),
            0
        )
    );
}

/// The `head/` set's real Checkpoint (ADR 0066): its size and digest are
/// those its record states, and an engine of today resumes the Run it holds
/// at the next run epoch, keeping its Clean-up rules digest and the
/// Download's file it carries, which saves whole.
/// Covers: story:3-37, story:3-38, story:6-17
#[test]
fn the_head_sets_checkpoint_matches_its_record_and_resumes() {
    let head = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../compat/run-state/head");
    let record: Value =
        serde_json::from_slice(&std::fs::read(head.join("checkpoint.json")).unwrap()).unwrap();
    let env = TestEnv::new();
    let copy = env.folder().join("checkpoint.sqlite");
    std::fs::copy(head.join("checkpoint.sqlite"), &copy).unwrap();
    let location = copy.display().to_string();
    let (size, digest) = (std::fs::metadata(&copy).unwrap().len(), sha256(&copy));
    let saved = env.folder().join("books.csv");
    let path = saved.display().to_string();

    let resumed = attached_on(&engine(), &env, |client| async move {
        let mut talk = Talk::open(client).await;
        let resumed = talk
            .ok(
                2,
                "run.resume",
                json!({ "run_id": "paused", "checkpoint": location }),
            )
            .await;
        talk.ok(
            3,
            "download.save_as",
            json!({ "download_id": "paused.download-1", "path": path }),
        )
        .await;
        talk.ok(4, "run.finish", json!({ "run_id": "paused" }))
            .await;
        resumed
    });

    assert_eq!(
        (
            (record["size"].clone(), record["digest"].clone()),
            resumed["started_as"].clone(),
            run_row(&run_folder(&env, "paused").join("run.sqlite")),
            std::fs::read(&saved).ok(),
        ),
        (
            (json!(size), json!(digest)),
            json!("resumed"),
            json!({
                "epoch": 2,
                "lifecycle": "finished",
                "stop_reason": null,
                "clean_up_digest": "6c8abc7a55f570dbb19512bdf4a9321337d1cb2c9b507c105b01b87fabbcc34b",
            }),
            Some(b"title,price\nDune,9.99\n".to_vec()),
        )
    );
}

/// Secret canaries planted in a Run, in the Session source its settings
/// name (a browser endpoint's token, a proxy's password, a Model API key),
/// are in none of its Checkpoint's bytes nor its run folder's, in any
/// encoding (ADR 0067). A Checkpoint has no encrypted field yet: Sessions
/// and their run key are area 4's.
/// Covers: story:3-30
#[test]
fn secret_canaries_planted_in_a_run_are_absent_from_its_checkpoint() {
    let env = TestEnv::new();
    let mut canaries = goodfetch_test_support::Canaries::new(env.seed());
    let token = canaries
        .plant(goodfetch_test_support::SecretKind::EndpointToken)
        .value()
        .to_owned();
    let password = canaries
        .plant(goodfetch_test_support::SecretKind::ProxyPassword)
        .value()
        .to_owned();
    let key = canaries
        .plant(goodfetch_test_support::SecretKind::ModelApiKey)
        .value()
        .to_owned();
    let source = json!({
        "kind": "cdp_url",
        "url": format!("ws://user:{password}@127.0.0.1:9222/devtools/browser/{token}?key={key}"),
    });

    let checkpoint = attached_on(&engine(), &env, |client| async move {
        let mut talk = Talk::open(client).await;
        talk.ok(
            2,
            "run.start",
            json!({ "run_id": "secrets", "settings": { "session": { "source": source } } }),
        )
        .await;
        let checkpoint = talk
            .ok(3, "run.checkpoint", json!({ "run_id": "secrets" }))
            .await;
        talk.ok(4, "run.finish", json!({ "run_id": "secrets" }))
            .await;
        checkpoint
    });

    let location = PathBuf::from(checkpoint["location"].as_str().unwrap());
    let folder = run_folder(&env, "secrets");
    let leaks = canaries
        .leaks(&[
            goodfetch_test_support::Output::Path(&location),
            goodfetch_test_support::Output::Path(&folder),
        ])
        .unwrap();
    assert_eq!((location.exists(), leaks), (true, Vec::new()));
}
