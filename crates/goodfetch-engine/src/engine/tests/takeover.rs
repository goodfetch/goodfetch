//! Taking the writer role of a Run whose engine died, through the engine's
//! public interface over the in-memory network (spec #213's Seam 3; ADRs
//! 0003, 0031, 0037's consequence and 0098): `run.abort` takes the writer
//! lock, then the next run epoch, then writes the end; a reader's test that
//! holds the lock for a moment never fails it, nor a start or a resume; a lock another engine keeps
//! fails it as `run_busy` once retried; a forwarded abort reaching an engine
//! that does not own the Run answers `run_not_owned`; a Collie engine
//! refuses a Run that needs Foxhound with `not_in_product`, its run file
//! untouched. An engine dies here as a killed one does: its runtime is
//! dropped with the Run live, so its lock and its run file close and the
//! Run stays `running`.

use std::collections::BTreeMap;
use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};
use std::time::Duration;

use goodfetch_protocol::catalog::{
    Operation, OperationClass, OperationKind, Product, Reach, Stability, schema_of,
};
use goodfetch_protocol::run::{RunId, RunKind, Tags};
use goodfetch_test_support::TestEnv;
use rstest::rstest;
use rusqlite::Connection;
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use tokio::io::{AsyncBufReadExt as _, BufReader};

use super::run::call;
use super::{Client, attached_on, dependencies, engine_with, hello};
use crate::composition::{CallContext, CallError, Contribution, Handler};
use crate::engine::Engine;
use crate::runs::{EndedBy, Start, Started};
use crate::transport::{self, Network};

/// Starts the Run `grows`, which then uses an entry only Foxhound carries,
/// and leaves it running.
const GROW: Operation = Operation {
    name: "test.grow",
    description: "Starts a Run that needs Foxhound and leaves it running.",
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

fn grow() -> Contribution {
    Contribution::new("test-takeover").with_handler(Handler::new(
        &GROW,
        |_: Value, context: CallContext| async move {
            let started = context
                .runs()
                .start(Start {
                    run_id: Some(RunId::parse("grows").unwrap()),
                    tags: Tags::default(),
                    kind: RunKind::Collie,
                    needs: Product::Collie,
                    inputs: BTreeMap::new(),
                    ended_by: EndedBy::Driver,
                    initialize: None,
                    policy_version: None,
                    weakened: Vec::new(),
                    binding: None,
                    clean_up_digest: None,
                    checkpoint_interval: None,
                    timeout: None,
                    settings: None,
                })
                .await
                .unwrap();
            let Started::New(run) = started else {
                panic!("the Run began before");
            };
            run.uses(Product::Foxhound).await;
            Ok::<Value, CallError>(json!({ "run_id": run.id() }))
        },
    ))
}

fn engine(product: Product) -> Engine {
    engine_with(product, Some(grow()))
}

/// Has an engine of `env` answer `calls`, then kills it: its runtime is
/// dropped with its Runs live.
fn killed_after(env: &TestEnv, calls: &[(&str, Value)]) {
    let runtime = goodfetch_test_support::runtime();
    runtime.block_on(async {
        let (network, dialer) = transport::in_memory();
        let engine = engine(Product::Collie);
        let serving = engine.run_attached(Network::InMemory(network), dependencies(env));
        let talk = async {
            let (reader, writer) = dialer.connect().await.unwrap().split();
            let mut client = Client {
                lines: BufReader::new(reader).lines(),
                writer,
            };
            client.send(&[hello("collie", "0.1")]).await;
            client.next().await;
            for (id, (method, params)) in (1..).zip(calls) {
                let reply = call(&mut client, id, method, params.clone()).await;
                assert!(reply.get("result").is_some(), "{reply}");
            }
        };
        tokio::select! {
            served = serving => panic!("the engine ended: {served:?}"),
            () = talk => {}
        }
    });
    drop(runtime);
}

fn run_folder(env: &TestEnv, run_id: &str) -> PathBuf {
    env.home().join("1").join("runs").join(run_id)
}

/// The run file's `run` row, as JSON.
fn run_row(folder: &Path) -> Value {
    let file = Connection::open(folder.join("run.sqlite")).unwrap();
    let row: String = file
        .query_row(
            "SELECT json_object('epoch', epoch, 'needs', needs, 'lifecycle', lifecycle,
                'stop_reason', json(stop_reason))
             FROM run",
            [],
            |row| row.get(0),
        )
        .unwrap();
    serde_json::from_str(&row).unwrap()
}

fn index_row(env: &TestEnv, run_id: &str) -> Value {
    let index = Connection::open(env.home().join("1").join("run-index.sqlite")).unwrap();
    let row: String = index
        .query_row(
            "SELECT json_object('lifecycle', lifecycle, 'owner', owner, 'presence', presence)
             FROM runs WHERE run_id = ?1",
            [run_id],
            |row| row.get(0),
        )
        .unwrap();
    serde_json::from_str(&row).unwrap()
}

/// The SHA-256 of the run file in `folder` and of its WAL, which its
/// writer leaves beside it (#2117).
fn digest(folder: &Path) -> String {
    let mut bytes = std::fs::read(folder.join("run.sqlite")).unwrap();
    bytes.extend(std::fs::read(folder.join("run.sqlite-wal")).unwrap_or_default());
    format!("{:x}", Sha256::digest(bytes))
}

fn lock_file(folder: &Path) -> File {
    OpenOptions::new()
        .read(true)
        .write(true)
        .open(folder.join("run.lock"))
        .unwrap()
}

/// Covers: operation:run.abort
#[test]
fn an_abort_takes_the_writer_role_of_a_run_whose_engine_died_at_the_next_run_epoch() {
    let env = TestEnv::new();
    killed_after(&env, &[("run.start", json!({ "run_id": "orphan" }))]);
    let folder = run_folder(&env, "orphan");
    let left = run_row(&folder);

    let aborted = attached_on(&engine(Product::Collie), &env, |mut client| async move {
        client.send(&[hello("collie", "0.1")]).await;
        client.next().await;
        call(&mut client, 1, "run.abort", json!({ "run_id": "orphan" })).await
    });

    assert_eq!(
        (
            left,
            aborted["result"]["lifecycle"].clone(),
            aborted["result"]["stop_reason"].clone(),
            run_row(&folder),
            index_row(&env, "orphan"),
            lock_file(&folder).try_lock().is_ok(),
        ),
        (
            json!({ "epoch": 1, "needs": "collie", "lifecycle": "running", "stop_reason": null }),
            json!("stopped"),
            json!({ "kind": "abort", "by": "client" }),
            json!({
                "epoch": 2, "needs": "collie", "lifecycle": "stopped",
                "stop_reason": { "kind": "abort", "by": "client" },
            }),
            json!({ "lifecycle": "stopped", "owner": null, "presence": "present" }),
            true,
        )
    );
}

/// A reader's test that holds the lock as the writer first tries it, then
/// lets go, never fails the takeover: the writer retries.
/// Covers: story:3-46, operation:run.abort
#[test]
fn a_readers_test_at_the_moment_of_a_takeover_never_fails_it() {
    let env = TestEnv::new();
    killed_after(&env, &[("run.start", json!({ "run_id": "orphan" }))]);
    let folder = run_folder(&env, "orphan");
    let reader = lock_file(&folder);
    reader.try_lock_shared().unwrap();

    let aborted = attached_on(&engine(Product::Collie), &env, |mut client| async move {
        client.send(&[hello("collie", "0.1")]).await;
        client.next().await;
        let let_go = async {
            tokio::time::sleep(Duration::from_millis(10)).await;
            reader.unlock().unwrap();
        };
        let (aborted, ()) = tokio::join!(
            call(&mut client, 1, "run.abort", json!({ "run_id": "orphan" })),
            let_go
        );
        aborted
    });

    assert_eq!(
        (
            aborted["result"]["lifecycle"].clone(),
            run_row(&folder)["epoch"].clone()
        ),
        (json!("stopped"), json!(2))
    );
}

/// A reader's test that holds the lock as a start or a resume first tries
/// it, then lets go, never fails it with `run_in_use`: the writer retries,
/// as every writer that takes the lock does (ADR 0031).
/// Covers: story:3-46
#[rstest]
/// Covers: operation:run.start
#[case::start("run.start", "fresh")]
/// Covers: operation:run.resume
#[case::resume("run.resume", "orphan")]
fn a_readers_test_at_the_moment_of_a_start_or_a_resume_never_fails_it(
    #[case] method: &str,
    #[case] run_id: &str,
) {
    let env = TestEnv::new();
    killed_after(&env, &[("run.start", json!({ "run_id": "orphan" }))]);
    let folder = run_folder(&env, run_id);
    std::fs::create_dir_all(&folder).unwrap();
    let reader = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(folder.join("run.lock"))
        .unwrap();
    reader.try_lock_shared().unwrap();
    let params = json!({ "run_id": run_id });

    let taken = attached_on(&engine(Product::Collie), &env, |mut client| async move {
        client.send(&[hello("collie", "0.1")]).await;
        client.next().await;
        let let_go = async {
            tokio::time::sleep(Duration::from_millis(10)).await;
            reader.unlock().unwrap();
        };
        let (taken, ()) = tokio::join!(call(&mut client, 1, method, params), let_go);
        taken
    });

    assert_eq!(
        (&taken["result"]["run_id"], &taken["error"]),
        (&json!(run_id), &Value::Null),
        "{taken}"
    );
}

/// Covers: error:run_busy, operation:run.abort
#[test]
fn a_writer_lock_another_engine_keeps_fails_the_abort_as_busy_once_retried() {
    let env = TestEnv::new();
    killed_after(&env, &[("run.start", json!({ "run_id": "held" }))]);
    let folder = run_folder(&env, "held");
    let other = lock_file(&folder);
    other.try_lock().unwrap();
    let before = digest(&folder);

    let aborted = attached_on(&engine(Product::Collie), &env, |mut client| async move {
        client.send(&[hello("collie", "0.1")]).await;
        client.next().await;
        call(&mut client, 1, "run.abort", json!({ "run_id": "held" })).await
    });

    assert_eq!(
        (aborted["error"]["data"]["code"].clone(), digest(&folder)),
        (json!("run_busy"), before)
    );
}

/// Covers: error:run_not_owned, operation:run.abort
#[test]
fn a_forwarded_abort_of_a_run_this_engine_does_not_own_answers_run_not_owned() {
    let env = TestEnv::new();
    killed_after(&env, &[("run.start", json!({ "run_id": "elsewhere" }))]);
    let folder = run_folder(&env, "elsewhere");
    let before = digest(&folder);

    let aborted = attached_on(&engine(Product::Collie), &env, |mut client| async move {
        client.send(&[hello("collie", "0.1")]).await;
        client.next().await;
        call(
            &mut client,
            1,
            "run.abort",
            json!({ "run_id": "elsewhere", "forwarded": true }),
        )
        .await
    });

    assert_eq!(
        (aborted["error"]["data"]["code"].clone(), digest(&folder)),
        (json!("run_not_owned"), before)
    );
}

/// A Collie engine reads a Run that needs Foxhound like any other, and
/// refuses to take its writer role, leaving its run file as it was (ADR 0098).
/// Covers: story:3-141, error:not_in_product, operation:run.abort
#[test]
fn a_collie_engine_refuses_the_writer_role_of_a_run_that_needs_foxhound() {
    assert_eq!(
        abort_of_a_foxhound_run_by(Product::Collie),
        (
            json!(["product"]),
            json!({ "error": "not_in_product", "lifecycle": "running", "epoch": 1 }),
            true
        )
    );
}

/// A Foxhound engine takes the writer role of a Run that needs Foxhound and
/// stops it at the next epoch (ADR 0098).
/// Covers: story:3-141, operation:run.abort
#[test]
fn a_foxhound_engine_takes_the_writer_role_of_a_run_that_needs_foxhound() {
    assert_eq!(
        abort_of_a_foxhound_run_by(Product::Foxhound),
        (
            json!(["product"]),
            json!({ "error": null, "lifecycle": "stopped", "epoch": 2 }),
            false
        )
    );
}

/// Aborts, from an engine of `product`, a Run of a killed engine that needs
/// Foxhound: the differences `run.start` names, the abort's Error code with
/// the Run index row's lifecycle and epoch after it, and whether the run file
/// is unchanged.
fn abort_of_a_foxhound_run_by(product: Product) -> (Value, Value, bool) {
    let env = TestEnv::new();
    killed_after(&env, &[("test.grow", json!({}))]);
    let folder = run_folder(&env, "grows");
    let before = digest(&folder);
    let needs = product.as_str().to_owned();

    let (started, aborted) = attached_on(&engine(product), &env, |mut client| async move {
        client.send(&[hello(&needs, "0.1")]).await;
        client.next().await;
        let started = call(&mut client, 1, "run.start", json!({ "run_id": "grows" })).await;
        let aborted = call(&mut client, 2, "run.abort", json!({ "run_id": "grows" })).await;
        (started, aborted)
    });

    let after = digest(&folder);
    let row = run_row(&folder);
    (
        started["error"]["data"]["differences"].clone(),
        json!({
            "error": aborted["error"]["data"]["code"],
            "lifecycle": row["lifecycle"],
            "epoch": row["epoch"],
        }),
        after == before,
    )
}
