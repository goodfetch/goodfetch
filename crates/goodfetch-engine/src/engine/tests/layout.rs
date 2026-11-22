//! `run_state.layout` through the engine's public interface over the
//! in-memory network (spec #213's Seam 3; ADR 0002): a Daemon in the
//! per-install layout keeps every Run's run state in the one install
//! database, and a Run moves between the layouts through its Checkpoints,
//! each layout giving the same per-run file.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use goodfetch_protocol::catalog::{
    Operation, OperationClass, OperationKind, Product, Reach, Stability, schema_of,
};
use goodfetch_protocol::daemon::DaemonEndpoint;
use goodfetch_protocol::run::RunId;
use goodfetch_test_support::TestEnv;
use rusqlite::Connection;
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use tokio::io::{AsyncBufReadExt as _, BufReader};

use super::run::call;
use super::{Client, attached_on, dependencies, engine_with, hello};
use crate::call_error::CallError;
use crate::composition::{CallContext, Contribution, Handler};
use crate::daemon::{DaemonOptions, Holds, Succession};
use crate::engine::Engine;
use crate::ops::Ops;
use crate::run_state::install::{INSTALL_FILE, InstallDatabase};
use crate::run_state::{RunStateOptions, RunStateTables};
use crate::transport::{self, InMemoryDialer, Network};

const TOKEN: &str = "the-launch-token";

/// Commits a note into a live Run's run state.
const NOTE: Operation = Operation {
    name: "test.note",
    description: "Commits a note into a live Run's run state.",
    class: OperationClass::Effect,
    kind: OperationKind::RoundTrip,
    reach: Reach::AllSurfaces,
    params: schema_of::<Value>,
    result: schema_of::<Value>,
    events: &[],
    cancellable: false,
    input: None,
    bound: None,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: goodfetch_protocol::catalog::PROTOCOL_VERSION,
};

/// The notes the tests commit, a table of every run file.
const NOTES: RunStateTables = RunStateTables {
    name: "test-notes",
    statements: &["CREATE TABLE IF NOT EXISTS notes (value INTEGER NOT NULL) STRICT"],
};

fn noting() -> Contribution {
    Contribution::new("test-layout")
        .with_run_state_tables(NOTES)
        .with_handler(Handler::new(
            &NOTE,
            |params: Value, context: CallContext| async move {
                let run_id = RunId::parse(params["run_id"].as_str().unwrap()).unwrap();
                let value = params["value"].as_i64().unwrap();
                let run = context.runs().live(run_id).await.unwrap();
                crate::runs::state(&run)
                    .write(move |connection| {
                        connection
                            .execute("INSERT INTO notes (value) VALUES (?1)", [value])
                            .map(drop)
                    })
                    .await
                    .unwrap();
                Ok::<Value, CallError>(json!({ "noted": value }))
            },
        ))
}

fn engine() -> Engine {
    engine_with(Product::Collie, Some(noting()))
}

/// The Daemon's own pick, which never hands over.
struct ThePick;

impl Succession for ThePick {
    fn is_pick(&self) -> bool {
        true
    }

    fn begin_handover(&self) -> Box<dyn Send> {
        Box::new(())
    }
}

/// A Client's Connection: each call's reply.
pub(super) struct Talk(Client);

impl Talk {
    pub(super) async fn ok(&mut self, id: i64, method: &str, params: Value) -> Value {
        let reply = call(&mut self.0, id, method, params).await;
        assert!(reply.get("result").is_some(), "{method}: {reply}");
        reply["result"].clone()
    }

    /// The reply to `method`, a result or an error.
    pub(super) async fn reply(&mut self, id: i64, method: &str, params: Value) -> Value {
        call(&mut self.0, id, method, params).await
    }

    /// The Connection's Client, for a call it reads message by message.
    pub(super) fn client(&mut self) -> &mut Client {
        &mut self.0
    }
}

/// Runs the engine as an Attached engine in `env`, in the per-run layout,
/// while `talk` talks to it.
pub(super) fn per_run<F, Fut, T>(env: &TestEnv, talk: F) -> T
where
    F: FnOnce(Talk) -> Fut,
    Fut: Future<Output = T>,
{
    per_run_on(&engine(), env, talk)
}

/// [`per_run`], with `engine`.
pub(super) fn per_run_on<F, Fut, T>(engine: &Engine, env: &TestEnv, talk: F) -> T
where
    F: FnOnce(Talk) -> Fut,
    Fut: Future<Output = T>,
{
    attached_on(engine, env, |mut client| async move {
        client.send(&[hello("collie", "0.1")]).await;
        client.next().await;
        talk(Talk(client)).await
    })
}

/// Runs the engine as a Daemon in `env`, in the per-install layout, while
/// `talk` talks to it; the Daemon then stops idle.
pub(super) fn per_install<F, Fut, T>(env: &TestEnv, talk: F) -> T
where
    F: FnOnce(Talk) -> Fut,
    Fut: Future<Output = T>,
{
    per_install_on(&engine(), env, talk)
}

/// [`per_install`], with `engine` as the Daemon.
pub(super) fn per_install_on<F, Fut, T>(engine: &Engine, env: &TestEnv, talk: F) -> T
where
    F: FnOnce(Talk) -> Fut,
    Fut: Future<Output = T>,
{
    daemon_on(engine, env, |dialer| async move {
        let mut client = connect(&dialer).await;
        let mut hello = hello("collie", "0.1");
        hello["params"]["token"] = json!(TOKEN);
        client.send(&[hello]).await;
        client.next().await;
        talk(Talk(client)).await
    })
}

async fn connect(dialer: &InMemoryDialer) -> Client {
    let (reader, writer) = dialer.connect().await.unwrap().split();
    Client {
        lines: BufReader::new(reader).lines(),
        writer,
    }
}

/// Runs the engine as a Daemon in `env`, in the per-install layout, while
/// `body` dials it; the Daemon then stops idle.
fn daemon_in<F, Fut, T>(env: &TestEnv, body: F) -> T
where
    F: FnOnce(InMemoryDialer) -> Fut,
    Fut: Future<Output = T>,
{
    daemon_on(&engine(), env, body)
}

/// [`daemon_in`], with `engine` as the Daemon.
fn daemon_on<F, Fut, T>(engine: &Engine, env: &TestEnv, body: F) -> T
where
    F: FnOnce(InMemoryDialer) -> Fut,
    Fut: Future<Output = T>,
{
    goodfetch_test_support::runtime().block_on(async {
        let dependencies = dependencies(env);
        // The test `tracing` subscriber writes the dispatch records into the
        // test's exercise record (ADR 0072), as `attached_with`'s does.
        let clock = dependencies.clock;
        let unset = tracing::dispatcher::get_default(
            tracing::Dispatch::is::<tracing::subscriber::NoSubscriber>,
        );
        let _subscriber = unset.then(|| {
            tracing::subscriber::set_default(goodfetch_test_support::subscriber(move || {
                clock.wall()
            }))
        });
        let install = InstallDatabase::open(
            &install_file(env),
            &env.home().join("1").join("runs"),
            &RunStateOptions::default(),
        )
        .unwrap();
        let options = DaemonOptions {
            token: TOKEN.to_owned(),
            endpoint: DaemonEndpoint::Unix("daemon.sock".into()),
            pid: 4242,
            idle_timeout: Duration::from_secs(60),
            keep_alive: false,
            holds: Holds::new(),
            succession: Arc::new(ThePick),
            drain: crate::daemon::Drain {
                asked: None,
                grace: Duration::from_secs(110),
            },
            install: Some(install),
            ops: Ops::default(),
            websocket: None,
        };
        let (network, dialer) = transport::in_memory();
        let (listener, _) = Network::InMemory(network)
            .listen(Path::new("daemon.sock"))
            .unwrap();
        let (_, observed) = tokio::join!(
            engine.run_daemon(listener, dependencies, options),
            body(dialer)
        );
        observed
    })
}

pub(super) fn install_file(env: &TestEnv) -> PathBuf {
    env.home().join("1").join(INSTALL_FILE)
}

fn run_file(env: &TestEnv, run_id: &str) -> PathBuf {
    env.home()
        .join("1")
        .join("runs")
        .join(run_id)
        .join("run.sqlite")
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

/// Covers: story:3-29, setting:run_state.layout
#[test]
fn a_daemon_in_the_per_install_layout_keeps_its_runs_in_the_install_database() {
    let env = TestEnv::new();

    per_install(&env, |mut talk| async move {
        talk.ok(1, "run.start", json!({ "run_id": "first" })).await;
        talk.ok(2, "run.start", json!({ "run_id": "second" })).await;
        talk.ok(3, "test.note", json!({ "run_id": "first", "value": 1 }))
            .await;
    });

    let install = InstallDatabase::open(
        &install_file(&env),
        &env.home().join("1").join("runs"),
        &RunStateOptions::default(),
    )
    .unwrap();
    assert_eq!(
        (
            install.runs().unwrap(),
            run_file(&env, "first").exists(),
            run_file(&env, "second").exists(),
        ),
        (vec!["first".to_owned(), "second".to_owned()], false, false)
    );
}

/// A Run paused in the per-run layout resumes in the per-install layout
/// and back: the same steps from the same Checkpoint give the same
/// Checkpoint, byte for byte, in either layout.
///
/// Covers: story:3-29, operation:run.resume, operation:run.checkpoint
#[test]
fn a_run_round_trips_between_the_layouts_through_the_same_per_run_file() {
    let (origin, installed, per_run_copy, back) = (
        TestEnv::new(),
        TestEnv::new(),
        TestEnv::new(),
        TestEnv::new(),
    );
    let paused = origin
        .home()
        .join("1")
        .join("runs")
        .join("moving")
        .join("checkpoint.sqlite");
    per_run(&origin, |mut talk| async move {
        talk.ok(1, "run.start", json!({ "run_id": "moving" })).await;
        talk.ok(2, "test.note", json!({ "run_id": "moving", "value": 1 }))
            .await;
        talk.ok(3, "run.pause", json!({ "run_id": "moving" })).await;
    });
    let steps = |location: PathBuf, checkpoint: PathBuf| {
        move |mut talk: Talk| async move {
            talk.ok(
                1,
                "run.resume",
                json!({ "run_id": "moving", "checkpoint": checkpoint }),
            )
            .await;
            talk.ok(2, "test.note", json!({ "run_id": "moving", "value": 2 }))
                .await;
            talk.ok(
                3,
                "run.checkpoint",
                json!({ "run_id": "moving", "location": location }),
            )
            .await
        }
    };
    let from_install = installed.folder().join("from-install.sqlite");
    let from_file = per_run_copy.folder().join("from-file.sqlite");

    let by_install = per_install(&installed, steps(from_install.clone(), paused.clone()));
    let by_file = per_run(&per_run_copy, steps(from_file.clone(), paused));
    let resume_from = from_install.clone();
    per_run(&back, |mut talk| async move {
        talk.ok(
            1,
            "run.resume",
            json!({ "run_id": "moving", "checkpoint": resume_from }),
        )
        .await;
        talk.ok(2, "run.finish", json!({ "run_id": "moving" }))
            .await;
    });

    assert_eq!(
        (
            sha256(&from_install),
            by_install["digest"].clone(),
            run_file(&installed, "moving").exists(),
            notes(&run_file(&back, "moving")),
        ),
        (
            sha256(&from_file),
            by_file["digest"].clone(),
            false,
            vec![1, 2],
        )
    );
}

/// An Attached engine in the per-install layout relays its Client to the
/// Daemon, adding the Daemon's token to the Client's `hello`: the Run it
/// starts is the Daemon's, in the install database.
///
/// Covers: story:3-29
#[test]
fn an_attached_engine_in_the_per_install_layout_hands_its_runs_to_the_daemon() {
    let env = TestEnv::new();

    let (opened, started) = daemon_in(&env, |dialer| async move {
        let (network, attached) = transport::in_memory();
        let daemon = dialer.connect().await.unwrap();
        let relay = Engine::relay_attached(Network::InMemory(network), daemon, TOKEN.to_owned());
        let client = async {
            let mut client = connect(&attached).await;
            client.send(&[hello("collie", "0.1")]).await;
            let opened = client.next().await.unwrap();
            let started = call(&mut client, 1, "run.start", json!({ "run_id": "handed" })).await;
            client.close().await;
            (opened, started)
        };
        let (relayed, answered) = tokio::join!(relay, client);
        relayed.unwrap();
        answered
    });

    let install = InstallDatabase::open(
        &install_file(&env),
        &env.home().join("1").join("runs"),
        &RunStateOptions::default(),
    )
    .unwrap();
    assert_eq!(
        (
            opened["result"]["product"].clone(),
            started["result"]["started_as"].clone(),
            install.runs().unwrap(),
            run_file(&env, "handed").exists(),
        ),
        (
            json!("collie"),
            json!("new"),
            vec!["handed".to_owned()],
            false
        )
    );
}
