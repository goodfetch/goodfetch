//! A Run its program drives, through the engine's public interface over the
//! in-memory network (spec #213's Seam 3): `run.start` in the per-user,
//! per-major data folder or where `run_state.location` says, with its
//! writer's lock held beside its state and its Run index row; a Run ID the
//! caller chose, or a lowercase ULID from the `Clock` and the `Entropy`;
//! `run.finish` and `run.abort` answering the three axes; a Connection's end
//! stopping its live Runs; and the Product each Run needs (ADR 0098). What
//! the tests read of the run file, the lock file and the Run index is what
//! the engine wrote.

use std::collections::BTreeMap;
use std::fs::{File, OpenOptions, TryLockError};
use std::path::{Path, PathBuf};

use goodfetch_protocol::catalog::{
    Operation, OperationClass, OperationKind, Product, Reach, Stability, schema_of,
};
use goodfetch_protocol::run::{RunKind, Tags};
use goodfetch_test_support::TestEnv;
use rstest::rstest;
use rusqlite::Connection;
use serde_json::{Value, json};

use super::{Client, attached_on, attached_with, dependencies, engine_with, hello};
use crate::call_error::CallError;
use crate::composition::{CallContext, Contribution, Handler};
use crate::engine::Dependencies;
use crate::home::RunStateLocation;
use crate::runs::{EndedBy, Ending, Start, Started};

/// Starts a Run of its own that then uses an entry only Foxhound carries,
/// and ends it.
const USE_FOXHOUND: Operation = Operation {
    name: "test.use_foxhound",
    description: "Starts a Run that uses an entry only Foxhound carries.",
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

fn use_foxhound() -> Contribution {
    Contribution::new("test-runs").with_handler(Handler::new(
        &USE_FOXHOUND,
        |_: Value, context: CallContext| async move {
            let started = context
                .runs()
                .start(Start {
                    run_id: Some(goodfetch_protocol::run::RunId::parse("grows").unwrap()),
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
                    detached: false,
                })
                .await
                .unwrap();
            let Started::New(run) = started else {
                panic!("the Run began before");
            };
            let run_id = crate::runs::identity(&run).id();
            context.runs().uses(run_id.clone(), Product::Foxhound).await;
            crate::runs::end(&run)
                .end(Ending::Finished(Vec::new()))
                .await;
            Ok::<Value, CallError>(json!({ "run_id": run_id }))
        },
    ))
}

/// Sends the call `method` with `params` and answers its reply.
pub(super) async fn call(client: &mut Client, id: i64, method: &str, params: Value) -> Value {
    client
        .send(&[json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params })])
        .await;
    loop {
        let message = client
            .next()
            .await
            .expect("the engine closed before replying");
        if message["id"] == json!(id) {
            return message;
        }
    }
}

pub(super) fn run_folder(env: &TestEnv, run_id: &str) -> PathBuf {
    env.home().join("1").join("runs").join(run_id)
}

pub(super) fn index_row(env: &TestEnv, run_id: &str) -> Value {
    let index = Connection::open(env.home().join("1").join("run-index.sqlite")).unwrap();
    let row: String = index
        .query_row(
            "SELECT json_object('location', location, 'kind', kind, 'needs', needs, 'tags', json(tags),
                'lifecycle', lifecycle, 'verdict', verdict, 'completeness', completeness)
             FROM runs WHERE run_id = ?1",
            [run_id],
            |row| row.get(0),
        )
        .unwrap();
    serde_json::from_str(&row).unwrap()
}

pub(super) fn run_row(folder: &Path) -> Value {
    let file = Connection::open(folder.join("run.sqlite")).unwrap();
    let row: String = file
        .query_row(
            "SELECT json_object('run_id', run_id, 'kind', kind, 'needs', needs, 'epoch', epoch,
                'lifecycle', lifecycle, 'stop_reason', json(stop_reason), 'verdict', verdict,
                'completeness', completeness)
             FROM run",
            [],
            |row| row.get(0),
        )
        .unwrap();
    serde_json::from_str(&row).unwrap()
}

/// Whether another holder can take the lock file's exclusive lock now.
fn lock_is_free(folder: &Path) -> bool {
    let lock: File = OpenOptions::new()
        .write(true)
        .open(folder.join("run.lock"))
        .unwrap();
    match lock.try_lock() {
        Ok(()) => true,
        Err(TryLockError::WouldBlock) => false,
        Err(TryLockError::Error(error)) => panic!("{error}"),
    }
}

fn journal_mode(folder: &Path) -> String {
    Connection::open(folder.join("run.sqlite"))
        .unwrap()
        .pragma_query_value(None, "journal_mode", |row| row.get(0))
        .unwrap()
}

/// Covers: story:3-25, story:3-28, operation:run.start, operation:run.finish
#[test]
fn a_run_starts_in_the_per_major_data_folder_under_its_writers_lock_and_is_listed_in_the_run_index()
{
    let env = TestEnv::new();
    let folder = run_folder(&env, "one");
    let (env_seen, folder_seen) = (&env, &folder);

    let (started, live, report) = attached_on(
        &engine_with(Product::Collie, None),
        &env,
        |mut client| async move {
            client.send(&[hello("collie", "0.1")]).await;
            client.next().await;
            let started = call(
                &mut client,
                1,
                "run.start",
                json!({ "run_id": "one", "tags": ["nightly"] }),
            )
            .await;
            let live = (
                lock_is_free(folder_seen),
                journal_mode(folder_seen),
                index_row(env_seen, "one"),
            );
            let report = call(&mut client, 2, "run.finish", json!({ "run_id": "one" })).await;
            (started, live, report)
        },
    );

    assert_eq!(
        (
            started["result"].clone(),
            live,
            report["result"]["lifecycle"].clone(),
            report["result"]["verdict"].clone(),
            report["result"]["completeness"].clone(),
            index_row(&env, "one"),
            lock_is_free(&folder),
        ),
        (
            json!({ "run_id": "one", "started_as": "new", "lifecycle": "running", "needs": "collie" }),
            (
                false,
                "wal".to_owned(),
                json!({
                    "location": "runs/one", "kind": "collie", "needs": "collie", "tags": ["nightly"],
                    "lifecycle": "running", "verdict": null, "completeness": null,
                }),
            ),
            json!("finished"),
            json!("verified"),
            json!("complete"),
            json!({
                "location": "runs/one", "kind": "collie", "needs": "collie", "tags": ["nightly"],
                "lifecycle": "finished", "verdict": "verified", "completeness": "complete",
            }),
            true,
        )
    );
}

/// A seeded test gets the same Run ID each time: a lowercase ULID whose time
/// is the `Clock`'s and whose randomness is the `Entropy`'s (ADR 0059).
#[test]
fn a_generated_run_id_is_a_lowercase_ulid_from_the_clock_and_the_entropy() {
    let start_without_an_id = || {
        let env = TestEnv::new();
        attached_on(
            &engine_with(Product::Collie, None),
            &env,
            |mut client| async move {
                client.send(&[hello("collie", "0.1")]).await;
                client.next().await;
                call(&mut client, 1, "run.start", json!({})).await["result"]["run_id"]
                    .as_str()
                    .unwrap()
                    .to_owned()
            },
        )
    };

    let (first, second) = (start_without_an_id(), start_without_an_id());

    assert_eq!(
        (
            &first[..10],
            first.len(),
            first.to_lowercase() == first,
            first == second
        ),
        ("01ke6p4ym0", 26, true, true)
    );
}

/// Covers: story:3-28, setting:run_state.location
#[test]
fn run_state_location_puts_the_run_folders_where_it_names() {
    let env = TestEnv::new();
    let elsewhere = env.folder().join("elsewhere");

    attached_with(
        &engine_with(Product::Collie, None),
        || Dependencies {
            environment: std::collections::BTreeMap::new(),
            keychain: crate::secrets::Keychain::absent(),
            run_state_location: RunStateLocation::Folder(elsewhere.clone()),
            ..dependencies(&env)
        },
        |mut client| async move {
            client.send(&[hello("collie", "0.1")]).await;
            client.next().await;
            call(&mut client, 1, "run.start", json!({ "run_id": "away" })).await
        },
    );

    assert_eq!(
        (
            elsewhere.join("away").join("run.sqlite").exists(),
            run_folder(&env, "away").exists(),
            index_row(&env, "away")["location"].clone(),
        ),
        (
            true,
            false,
            json!(elsewhere.join("away").to_string_lossy().replace('\\', "/")),
        )
    );
}

/// TESTING.md's path traversal case at the protocol: the engine refuses the
/// Run ID and writes nothing.
///
/// Covers: error:run_id_invalid
#[test]
fn a_run_id_that_could_name_another_folder_is_refused_and_nothing_is_written() {
    let env = TestEnv::new();

    let refused = attached_on(
        &engine_with(Product::Collie, None),
        &env,
        |mut client| async move {
            client.send(&[hello("collie", "0.1")]).await;
            client.next().await;
            call(
                &mut client,
                1,
                "run.start",
                json!({ "run_id": "../escape" }),
            )
            .await
        },
    );

    assert_eq!(
        (
            &refused["error"]["data"]["code"],
            env.home().join("1").exists()
        ),
        (&json!("run_id_invalid"), false)
    );
}

/// Params, and each object inside them, are a JSON object (ADR 0038):
/// serde's derived structs would also read them from an array, by
/// position, and `serde_json`'s map from `null`, which protocol fuzzing
/// found.
#[rstest]
#[case::params_by_position(json!(["fixed"]))]
#[case::project_by_position(json!({ "run_id": "fixed", "project": ["", {}] }))]
#[case::settings_null(json!({ "run_id": "fixed", "settings": null }))]
fn a_start_whose_params_or_an_object_in_them_are_no_json_object_is_invalid_params_and_nothing_is_written(
    #[case] params: Value,
) {
    let env = TestEnv::new();

    let refused = attached_on(
        &engine_with(Product::Collie, None),
        &env,
        |mut client| async move {
            client.send(&[hello("collie", "0.1")]).await;
            client.next().await;
            call(&mut client, 1, "run.start", params).await
        },
    );

    assert_eq!(
        (
            &refused["error"]["data"]["code"],
            env.home().join("1").exists()
        ),
        (&json!("invalid_params"), false)
    );
}

/// An Attached engine's Connection ends its Client's Runs, `stopped` and
/// `incomplete` (ADR 0078).
#[test]
fn the_end_of_the_connection_stops_its_live_runs_as_aborted_by_their_client() {
    let env = TestEnv::new();

    attached_on(
        &engine_with(Product::Collie, None),
        &env,
        |mut client| async move {
            client.send(&[hello("collie", "0.1")]).await;
            client.next().await;
            call(&mut client, 1, "run.start", json!({ "run_id": "left" })).await
        },
    );

    assert_eq!(
        (
            run_row(&run_folder(&env, "left")),
            index_row(&env, "left")["lifecycle"].clone()
        ),
        (
            json!({
                "run_id": "left", "kind": "collie", "needs": "collie", "epoch": 1,
                "lifecycle": "stopped", "stop_reason": { "kind": "abort", "by": "client" },
                "verdict": "verified", "completeness": "incomplete",
            }),
            json!("stopped"),
        )
    );
}

/// An aborted Run ends stopped and incomplete, and a start under its Run
/// ID resumes it (ADR 0102's start table).
///
/// Covers: operation:run.abort, story:3-166
#[test]
fn an_aborted_run_is_stopped_and_incomplete_and_a_start_under_its_run_id_resumes_it() {
    let env = TestEnv::new();

    let (report, again) = attached_on(
        &engine_with(Product::Collie, None),
        &env,
        |mut client| async move {
            client.send(&[hello("collie", "0.1")]).await;
            client.next().await;
            call(&mut client, 1, "run.start", json!({ "run_id": "short" })).await;
            let report = call(&mut client, 2, "run.abort", json!({ "run_id": "short" })).await;
            let again = call(&mut client, 3, "run.start", json!({ "run_id": "short" })).await;
            (report, again)
        },
    );

    assert_eq!(
        (
            &report["result"]["lifecycle"],
            &report["result"]["completeness"],
            &report["result"]["stop_reason"],
            &again["result"]["started_as"],
            &again["result"]["lifecycle"],
        ),
        (
            &json!("stopped"),
            &json!("incomplete"),
            &json!({ "kind": "abort", "by": "client" }),
            &json!("resumed"),
            &json!("running"),
        )
    );
}

/// A Run its program drives stops at once when its `timeout.run`, which
/// its Connection sets (ADR 0083), passes: the engine holds nothing in
/// flight for it. Its clock starts at its start, so before then it still
/// runs.
///
/// Covers: story:3-52, setting:timeout.run
#[test]
fn a_run_past_its_run_timeout_stops_incomplete_with_the_stop_reason_timeout() {
    let env = TestEnv::new();
    let listed = &env;
    let mut timed = hello("collie", "0.1");
    timed["params"]["settings"] = json!({ "timeout": { "run": 60_000 } });

    let (before, report) = attached_on(
        &engine_with(Product::Collie, None),
        &env,
        |mut client| async move {
            client.send(&[timed]).await;
            client.next().await;
            call(&mut client, 1, "run.start", json!({ "run_id": "timed" })).await;
            tokio::time::sleep(std::time::Duration::from_secs(59)).await;
            let before = index_row(listed, "timed");
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            let report = call(&mut client, 2, "run.finish", json!({ "run_id": "timed" })).await;
            (before, report)
        },
    );

    assert_eq!(
        (
            &before["lifecycle"],
            &report["result"]["lifecycle"],
            &report["result"]["completeness"],
            &report["result"]["stop_reason"],
        ),
        (
            &json!("running"),
            &json!("stopped"),
            &json!("incomplete"),
            &json!({ "kind": "timeout" }),
        )
    );
}

/// Covers: error:run_not_found
#[test]
fn finishing_a_run_no_engine_knows_is_run_not_found() {
    let env = TestEnv::new();

    let refused = attached_on(
        &engine_with(Product::Collie, None),
        &env,
        |mut client| async move {
            client.send(&[hello("collie", "0.1")]).await;
            client.next().await;
            call(&mut client, 1, "run.finish", json!({ "run_id": "nowhere" })).await
        },
    );

    assert_eq!(refused["error"]["data"]["code"], "run_not_found");
}

/// A Run needs Collie until it uses an entry the catalog marks Foxhound's,
/// and Foxhound from then on, in its own state and its Run index row
/// (ADR 0098).
#[test]
fn a_run_that_uses_an_entry_only_foxhound_carries_needs_foxhound_from_then_on() {
    let env = TestEnv::new();

    attached_on(
        &engine_with(Product::Foxhound, Some(use_foxhound())),
        &env,
        |mut client| async move {
            client.send(&[hello("foxhound", "0.1")]).await;
            client.next().await;
            call(&mut client, 1, "test.use_foxhound", json!({})).await
        },
    );

    assert_eq!(
        (
            run_row(&run_folder(&env, "grows"))["needs"].clone(),
            index_row(&env, "grows")["needs"].clone(),
        ),
        (json!("foxhound"), json!("foxhound"))
    );
}

/// While another engine holds a Run's writer lock, this one writes nothing
/// to it (ADR 0031).
///
/// Covers: error:run_in_use
#[test]
fn a_start_whose_run_folder_another_writer_locks_is_run_in_use() {
    let env = TestEnv::new();
    let folder = run_folder(&env, "held");
    std::fs::create_dir_all(&folder).unwrap();
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(folder.join("run.lock"))
        .unwrap();
    lock.lock().unwrap();

    let refused = attached_on(
        &engine_with(Product::Collie, None),
        &env,
        |mut client| async move {
            client.send(&[hello("collie", "0.1")]).await;
            client.next().await;
            call(&mut client, 1, "run.start", json!({ "run_id": "held" })).await
        },
    );

    assert_eq!(
        (
            &refused["error"]["data"]["code"],
            &refused["error"]["data"]["retryable"],
            folder.join("run.sqlite").exists()
        ),
        (&json!("run_in_use"), &json!(true), false)
    );
}

/// A Run whose state cannot be read is never started afresh under its Run
/// ID (ADR 0102).
///
/// Covers: error:run_state_unavailable
#[test]
fn a_start_whose_run_state_cannot_be_read_is_run_state_unavailable() {
    let env = TestEnv::new();
    let folder = run_folder(&env, "damaged");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("run.sqlite"), b"no SQLite file").unwrap();

    let refused = attached_on(
        &engine_with(Product::Collie, None),
        &env,
        |mut client| async move {
            client.send(&[hello("collie", "0.1")]).await;
            client.next().await;
            call(&mut client, 1, "run.start", json!({ "run_id": "damaged" })).await
        },
    );

    assert_eq!(
        (
            &refused["error"]["data"]["code"],
            std::fs::read(folder.join("run.sqlite")).unwrap(),
        ),
        (&json!("run_state_unavailable"), b"no SQLite file".to_vec())
    );
}

/// Covers: format:run-index
#[test]
fn the_run_index_records_its_format_version() {
    let env = TestEnv::new();

    attached_on(
        &engine_with(Product::Collie, None),
        &env,
        |mut client| async move {
            client.send(&[hello("collie", "0.1")]).await;
            client.next().await;
            call(&mut client, 1, "run.start", json!({ "run_id": "listed" })).await
        },
    );

    let index = Connection::open(env.home().join("1").join("run-index.sqlite")).unwrap();
    let format: (String, String) = index
        .query_row("SELECT name, version FROM format", [], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })
        .unwrap();
    assert_eq!(format, ("run-index".to_owned(), "0.1".to_owned()));
}
