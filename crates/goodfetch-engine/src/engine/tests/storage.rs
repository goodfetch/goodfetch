//! The storage levels through the engine's public interface over the
//! in-memory network (spec #213's Seam 3; ADR 0122), on the paused `Clock`,
//! the governor reading the filesystems through a view the test sets: a
//! Run on a filesystem crossing `storage.critical_free` stops resumably
//! with `resource: "storage"`, a start there is refused with
//! `resources_exhausted`, and a resume once space is freed goes on; at
//! `low` a Run goes on, carries `storage_low` and its progress says so; two
//! folders on one filesystem are read once per sample; a level set to `0`
//! is listed in the report's `weakened`; a Run resumed at `low` carries
//! `storage_low` once. A Run whose end could not be recorded leaves
//! `goodfetch.run.active`. A commit the crash VFS fails is
//! rolled back and stops its Run as at `critical`, and when the commit
//! recording the stop fails too, the waiting call raises
//! `resources_exhausted`.

use std::time::Duration;

use goodfetch_protocol::catalog::{
    Operation, OperationClass, OperationKind, Product, Reach, Stability, schema_of,
};
use goodfetch_protocol::run::RunId;
use goodfetch_test_support::{CrashVfs, IoFault, TestEnv};
use opentelemetry::metrics::MeterProvider as _;
use opentelemetry_sdk::metrics::SdkMeterProvider;
use rusqlite::{Connection, OpenFlags};
use serde_json::{Value, json};

use super::disk_view::DiskView;
use super::events::credit;
use super::ops::series;
use super::run::{call, run_folder};
use super::{Client, attached_on, attached_with, dependencies, engine, engine_with, hello};
use crate::Dependencies;
use crate::call_error::CallError;
use crate::composition::{CallContext, Contribution, Handler};
use crate::engine::Engine;
use crate::metrics::{METER, Metrics};
use crate::run_state::{RunStateOptions, RunStateTables};
use crate::tasks::BlockingPolicy;

const MB: u64 = 1_000_000;

/// Writes `settings` as the test home's user configuration, in the folder
/// of the tests' engines' major, after a short sample interval.
pub(super) fn configure(env: &TestEnv, settings: &str) {
    let folder = env.home().join("1");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(
        folder.join("configuration.toml"),
        format!("version = \"0.1\"\n\n[settings]\ngovernor.sample_interval = 250\n{settings}"),
    )
    .unwrap();
}

/// The levels the tests set: `low` below 2 MB, `critical` below 1 MB.
const LEVELS: &str = "storage.low_free = 2000000\nstorage.critical_free = 1000000\n";

/// A Collie engine whose governor reads the filesystems through `view`.
fn viewed(view: &DiskView) -> Engine {
    engine(Product::Collie).with_governor(view.system())
}

async fn opened(client: &mut Client) {
    client.send(&[hello("collie", "0.1")]).await;
    client.next().await;
}

/// Lets the governor take a few samples on the paused clock.
async fn samples() {
    tokio::time::sleep(Duration::from_secs(1)).await;
}

/// Crossing `storage.critical_free` stops the Run whose run file is on that
/// filesystem at its next safe point, `stopped` and `incomplete` with
/// `resource: "storage"` and its folder, writing no Checkpoint copy onto
/// that filesystem; once space is freed, a resume
/// goes on and finishes.
///
/// Covers: row:RT-03, setting:storage.critical_free, operation:run.resume
#[test]
fn a_run_on_a_filesystem_past_critical_stops_resumably_and_resumes_once_space_is_freed() {
    let env = TestEnv::new();
    configure(&env, LEVELS);
    let view = DiskView::default();
    let disk = view.mount(env.home(), 100 * MB, 50 * MB);
    let filled = view.clone();

    let (stopped, resumed, finished) = attached_on(&viewed(&view), &env, |mut client| async move {
        opened(&mut client).await;
        call(&mut client, 1, "run.start", json!({ "run_id": "filled" })).await;
        filled.set_free(disk, MB / 2);
        samples().await;
        let stopped = call(&mut client, 2, "run.finish", json!({ "run_id": "filled" })).await;
        filled.set_free(disk, 50 * MB);
        let resumed = call(&mut client, 3, "run.resume", json!({ "run_id": "filled" })).await;
        let finished = call(&mut client, 4, "run.finish", json!({ "run_id": "filled" })).await;
        (stopped, resumed, finished)
    });

    let folder = run_folder(&env, "filled").display().to_string();
    assert_eq!(
        (
            &stopped["result"]["checkpoint"],
            &stopped["result"]["lifecycle"],
            &stopped["result"]["completeness"],
            &stopped["result"]["stop_reason"],
            &resumed["result"]["started_as"],
            &finished["result"]["lifecycle"],
        ),
        (
            &Value::Null,
            &json!("stopped"),
            &json!("incomplete"),
            &json!({ "kind": "resources", "resource": "storage", "folder": folder }),
            &json!("resumed"),
            &json!("finished"),
        ),
        "{stopped}"
    );
}

/// A sample read while the filesystem was at `critical` that arrives after
/// space was freed and the Run resumed, its resume having read the
/// filesystem afresh, does not stop it again: the Run finishes (ADR 0122).
/// The governor reads on blocking threads, as in a Product, so the test
/// can hold that sample's read past the resume.
///
/// Covers: row:RT-03, setting:storage.critical_free, operation:run.resume
#[test]
fn a_sample_read_before_a_resume_read_afresh_does_not_stop_the_resumed_run() {
    let env = TestEnv::new();
    configure(&env, LEVELS);
    let view = DiskView::default();
    let disk = view.mount(env.home(), 100 * MB, 50 * MB);
    let filled = view.clone();

    let on_threads = || Dependencies {
        blocking: BlockingPolicy::Threads,
        ..dependencies(&env)
    };
    let (resumed, finished) = attached_with(&viewed(&view), on_threads, |mut client| async move {
        opened(&mut client).await;
        call(&mut client, 1, "run.start", json!({ "run_id": "filled" })).await;
        filled.set_free(disk, MB / 2);
        samples().await;
        call(&mut client, 2, "run.finish", json!({ "run_id": "filled" })).await;
        let late = filled.hold_next_read(disk);
        late.read().await;
        filled.set_free(disk, 50 * MB);
        let resumed = call(&mut client, 3, "run.resume", json!({ "run_id": "filled" })).await;
        late.release();
        samples().await;
        let finished = call(&mut client, 4, "run.finish", json!({ "run_id": "filled" })).await;
        (resumed, finished)
    });

    assert_eq!(
        (
            &resumed["result"]["started_as"],
            &finished["result"]["lifecycle"],
            &finished["result"]["stop_reason"],
        ),
        (&json!("resumed"), &json!("finished"), &Value::Null),
        "{finished}"
    );
}

/// A start whose run file would go to a filesystem at `critical` is
/// refused with `resources_exhausted`, an `UnavailableError` that is not
/// retryable, naming the folder, its free space and the setting, with a fix
/// that names the folder; nothing is written for it.
///
/// Covers: row:RT-03, error:resources_exhausted
#[test]
fn a_start_whose_run_file_would_go_to_a_filesystem_at_critical_is_resources_exhausted() {
    let env = TestEnv::new();
    configure(&env, LEVELS);
    let view = DiskView::default();
    view.mount(env.home(), 100 * MB, MB / 2);

    let refused = attached_on(&viewed(&view), &env, |mut client| async move {
        opened(&mut client).await;
        samples().await;
        call(&mut client, 1, "run.start", json!({ "run_id": "refused" })).await
    });

    let mut data = refused["error"]["data"].clone();
    let fix = data
        .as_object_mut()
        .unwrap()
        .remove("fix")
        .unwrap_or(Value::Null);
    let runs = env.home().join("1").join("runs").display().to_string();
    assert_eq!(
        (data, fix.as_str().is_some_and(|fix| fix.contains(&runs))),
        (
            json!({
                "code": "resources_exhausted",
                "retryable": false,
                "resource": "storage",
                "run_id": "refused",
                "path": runs,
                "free_bytes": MB / 2,
                "limit": { "key": "storage.critical_free", "value": MB, "layer": "user_configuration" },
            }),
            true,
        ),
        "{refused}"
    );
    assert!(!run_folder(&env, "refused").exists());
}

/// Crossing `storage.low_free` leaves the Run working: its progress reads
/// `storage` at `low` with the folder and its free space, its report
/// carries `storage_low`, and freeing space brings its progress back to
/// `normal`.
///
/// Covers: row:RT-03, setting:storage.low_free, event:run.progress
#[test]
fn a_run_on_a_filesystem_past_low_goes_on_and_carries_storage_low() {
    let env = TestEnv::new();
    configure(&env, LEVELS);
    let view = DiskView::default();
    let disk = view.mount(env.home(), 100 * MB, 50 * MB);
    let filling = view.clone();

    let (progress, finished) = attached_on(&viewed(&view), &env, |mut client| async move {
        opened(&mut client).await;
        client
            .send(&[
                json!({ "jsonrpc": "2.0", "id": 1, "method": "events.subscribe", "params": {} }),
                credit(1, 64),
            ])
            .await;
        call(&mut client, 2, "run.start", json!({ "run_id": "low" })).await;
        filling.set_free(disk, MB * 3 / 2);
        let mut progress = vec![next_storage(&mut client).await];
        filling.set_free(disk, 50 * MB);
        progress.push(next_storage(&mut client).await);
        let finished = call(&mut client, 3, "run.finish", json!({ "run_id": "low" })).await;
        (progress, finished)
    });

    let folder = run_folder(&env, "low").display().to_string();
    let warnings: Vec<&Value> = finished["result"]["warnings"]
        .as_array()
        .map(|warnings| warnings.iter().map(|warning| &warning["code"]).collect())
        .unwrap_or_default();
    assert_eq!(
        (
            progress,
            &finished["result"]["lifecycle"],
            warnings,
            &finished["result"]["warnings"][0]["folder"],
        ),
        (
            vec![
                json!({ "level": "low", "folder": folder, "free_bytes": MB * 3 / 2 }),
                json!({ "level": "normal", "folder": folder, "free_bytes": 50 * MB }),
            ],
            &json!("finished"),
            vec![&json!("storage_low")],
            &json!(folder),
        ),
        "{finished}"
    );
}

/// A Run warned of `storage_low` for its folder and resumed while that
/// filesystem is still at `low` carries the warning once: a resume hands
/// over no second warning for a folder its report already names (ADR
/// 0122).
///
/// Covers: row:RT-03, setting:storage.low_free, operation:run.resume
#[test]
fn a_run_resumed_on_a_filesystem_still_at_low_carries_storage_low_once() {
    let env = TestEnv::new();
    configure(&env, LEVELS);
    let view = DiskView::default();
    view.mount(env.home(), 100 * MB, MB * 3 / 2);

    let finished = attached_on(&viewed(&view), &env, |mut client| async move {
        opened(&mut client).await;
        call(&mut client, 1, "run.start", json!({ "run_id": "warned" })).await;
        samples().await;
        call(&mut client, 2, "run.pause", json!({ "run_id": "warned" })).await;
        call(&mut client, 3, "run.resume", json!({ "run_id": "warned" })).await;
        samples().await;
        call(&mut client, 4, "run.finish", json!({ "run_id": "warned" })).await
    });

    let folder = run_folder(&env, "warned").display().to_string();
    let warnings: Vec<(&Value, &Value)> = finished["result"]["warnings"]
        .as_array()
        .map(|warnings| {
            warnings
                .iter()
                .map(|warning| (&warning["code"], &warning["folder"]))
                .collect()
        })
        .unwrap_or_default();
    assert_eq!(
        (&finished["result"]["lifecycle"], warnings),
        (
            &json!("finished"),
            vec![(&json!("storage_low"), &json!(folder))]
        ),
        "{finished}"
    );
}

/// The next `run.progress` that carries `storage`, answered as its
/// `storage`.
async fn next_storage(client: &mut Client) -> Value {
    loop {
        let message = client.next().await.unwrap();
        let events = message["params"]["events"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        for event in events {
            if event["event"] == "run.progress" && event["data"]["storage"].is_object() {
                return event["data"]["storage"].clone();
            }
        }
        if message["params"]["events"].is_array() {
            client.send(&[credit(1, 1)]).await;
        }
    }
}

/// The run folders and the Goodfetch home are on one filesystem: each
/// sample reads it once, however many of the sampled folders it holds.
///
/// Covers: row:RT-03, event:engine.system_info
#[test]
fn two_folders_on_one_filesystem_are_read_once_per_sample() {
    let env = TestEnv::new();
    configure(&env, "");
    let view = DiskView::default();
    let disk = view.mount(env.home(), 100 * MB, 50 * MB);
    let counted = view.clone();

    let (samples, reads) = attached_on(&viewed(&view), &env, |mut client| async move {
        opened(&mut client).await;
        call(&mut client, 1, "run.start", json!({ "run_id": "counted" })).await;
        client
            .send(&[
                json!({ "jsonrpc": "2.0", "id": 2, "method": "events.subscribe", "params": { "system_info": true } }),
                credit(2, 64),
            ])
            .await;
        let mut samples = 0;
        let before = counted.reads(disk);
        while samples < 4 {
            let message = client.next().await.unwrap();
            samples += message["params"]["events"].as_array().map_or(0, |events| {
                events
                    .iter()
                    .filter(|event| event["event"] == "engine.system_info")
                    .count()
            });
        }
        (samples, counted.reads(disk) - before)
    });

    assert!(
        (samples..=samples + 1).contains(&usize::try_from(reads).unwrap()),
        "{reads} reads in {samples} samples"
    );
}

/// A level set to `0` is off, and the Run's report lists it in
/// `weakened`.
///
/// Covers: row:RT-03, setting:storage.low_free
#[test]
fn a_level_set_to_zero_is_listed_in_the_reports_weakened() {
    let env = TestEnv::new();
    configure(&env, "storage.low_free = 0\n");
    let view = DiskView::default();
    view.mount(env.home(), 100 * MB, 50 * MB);

    let finished = attached_on(&viewed(&view), &env, |mut client| async move {
        opened(&mut client).await;
        call(&mut client, 1, "run.start", json!({ "run_id": "off" })).await;
        call(&mut client, 2, "run.finish", json!({ "run_id": "off" })).await
    });

    let weakened: Vec<&Value> = finished["result"]["weakened"]
        .as_array()
        .map(|weakened| weakened.iter().map(|setting| &setting["key"]).collect())
        .unwrap_or_default();
    assert_eq!(weakened, vec![&json!("storage.low_free")], "{finished}");
}

/// Commits the note `value` into the run file of the live Run `run_id`,
/// answering whether it was acknowledged.
const NOTE: Operation = Operation {
    name: "test.note",
    description: "Commits a note into a live Run's run file.",
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

/// Charges one request to the live Run `run_id`, answering whether the
/// Usage ledger took it.
const CHARGE: Operation = Operation {
    name: "test.charge",
    description: "Charges one request to a live Run.",
    ..NOTE
};

/// The notes the tests commit, a table of every run file.
const NOTES: RunStateTables = RunStateTables {
    name: "test-notes",
    statements: &["CREATE TABLE IF NOT EXISTS notes (value INTEGER NOT NULL) STRICT"],
};

fn noting() -> Contribution {
    Contribution::new("test-storage")
        .with_run_state_tables(NOTES)
        .with_handler(Handler::new(
            &NOTE,
            |params: Value, context: CallContext| async move {
                let run_id = RunId::parse(params["run_id"].as_str().unwrap()).unwrap();
                let value = params["value"].as_i64().unwrap();
                let Ok(run) = context.runs().live(run_id).await else {
                    return Ok::<Value, CallError>(json!({ "acknowledged": false }));
                };
                let written = run
                    .state()
                    .write(move |connection| {
                        connection
                            .execute("INSERT INTO notes (value) VALUES (?1)", [value])
                            .map(drop)
                    })
                    .await;
                Ok(json!({ "acknowledged": written.is_ok() }))
            },
        ))
        .with_handler(Handler::new(
            &CHARGE,
            |params: Value, context: CallContext| async move {
                let run_id = RunId::parse(params["run_id"].as_str().unwrap()).unwrap();
                let run = context.runs().live(run_id).await.unwrap();
                let charged = run.charge("requests", 1).await;
                Ok::<Value, CallError>(json!({ "acknowledged": charged.is_ok() }))
            },
        ))
}

/// A Collie engine with `test.note`, whose run files open through the
/// crash VFS `vfs`, on the test's paused runtime.
fn crashing<F, Fut, T>(env: &TestEnv, vfs: &CrashVfs, client: F) -> T
where
    F: FnOnce(Client) -> Fut,
    Fut: Future<Output = T>,
{
    crashing_on(
        &engine_with(Product::Collie, Some(noting())),
        env,
        vfs,
        client,
    )
}

/// [`crashing`], with `engine`, which has `test.note`.
fn crashing_on<F, Fut, T>(engine: &Engine, env: &TestEnv, vfs: &CrashVfs, client: F) -> T
where
    F: FnOnce(Client) -> Fut,
    Fut: Future<Output = T>,
{
    let name = vfs.name().to_owned();
    attached_with(
        engine,
        || Dependencies {
            run_state: RunStateOptions {
                vfs: Some(name),
                ..RunStateOptions::default()
            },
            ..dependencies(env)
        },
        client,
    )
}

/// The notes in the run file of `run_id` and its integrity check, read
/// through `vfs` once the engine stopped.
fn notes_and_integrity(env: &TestEnv, vfs: &CrashVfs, run_id: &str) -> (Vec<i64>, String) {
    let file = run_folder(env, run_id).join("run.sqlite");
    let connection = Connection::open_with_flags_and_vfs(
        &file,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        vfs.name(),
    )
    .unwrap();
    let notes = connection
        .prepare("SELECT value FROM notes ORDER BY rowid")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<Vec<i64>, _>>()
        .unwrap();
    let integrity: String = connection
        .query_row("PRAGMA integrity_check", [], |row| row.get(0))
        .unwrap();
    (notes, integrity)
}

/// The crash VFS fails the I/O of a commit on `run_state`: the commit is
/// rolled back and never acknowledged, the Run stops with `resource:
/// "storage"` from its last acknowledged commit, and its resume redoes the
/// write that was in flight, which is then in the file once.
///
/// Covers: row:RT-03, operation:run.resume
#[test]
fn a_commit_that_fails_its_io_is_rolled_back_and_stops_the_run_resumably() {
    let env = TestEnv::new();
    let vfs = CrashVfs::register(&env.unique_name("storage-io"), env.seed()).unwrap();
    let faulted = vfs.clone();

    let (failed, stopped, resumed, redone) = crashing(&env, &vfs, |mut client| async move {
        opened(&mut client).await;
        call(&mut client, 1, "run.start", json!({ "run_id": "faulted" })).await;
        call(
            &mut client,
            2,
            "test.note",
            json!({ "run_id": "faulted", "value": 1 }),
        )
        .await;
        faulted.fail_at(faulted.operations(), IoFault::Error);
        let failed = call(
            &mut client,
            3,
            "test.note",
            json!({ "run_id": "faulted", "value": 2 }),
        )
        .await;
        let stopped = call(&mut client, 4, "run.finish", json!({ "run_id": "faulted" })).await;
        let resumed = call(&mut client, 5, "run.resume", json!({ "run_id": "faulted" })).await;
        let redone = call(
            &mut client,
            6,
            "test.note",
            json!({ "run_id": "faulted", "value": 2 }),
        )
        .await;
        call(&mut client, 7, "run.finish", json!({ "run_id": "faulted" })).await;
        (failed, stopped, resumed, redone)
    });

    let folder = run_folder(&env, "faulted").display().to_string();
    assert_eq!(
        (
            &failed["result"]["acknowledged"],
            &stopped["result"]["stop_reason"],
            &resumed["result"]["started_as"],
            &redone["result"]["acknowledged"],
            notes_and_integrity(&env, &vfs, "faulted"),
        ),
        (
            &json!(false),
            &json!({ "kind": "resources", "resource": "storage", "folder": folder }),
            &json!("resumed"),
            &json!(true),
            (vec![1, 2], "ok".to_owned()),
        ),
        "{stopped}"
    );
}

/// The crash VFS's disk is full from a transaction of the Usage ledger on:
/// it is rolled back and the charge never acknowledged, and every Run of
/// the engine stops with `resource: "storage"`, the one charging and one
/// that charged nothing alike; their stops meet the same full disk, so
/// each waiting call raises `resources_exhausted`, and once space is freed
/// each resumes, the ledger passing `PRAGMA integrity_check`.
///
/// Covers: row:RT-03, error:resources_exhausted, operation:usage.set
#[test]
fn a_usage_ledger_transaction_that_meets_a_full_disk_stops_every_run_of_the_engine() {
    let env = TestEnv::new();
    let vfs = CrashVfs::register(&env.unique_name("ledger-full"), env.seed()).unwrap();
    let faulted = vfs.clone();
    let limit = json!({
        "scope": "install",
        "usage_kind": "requests",
        "period": { "every": "month", "time_zone": "UTC" },
        "amount": 100,
    });

    let (charged, other, resumed) = crashing(&env, &vfs, |mut client| async move {
        opened(&mut client).await;
        call(&mut client, 1, "usage.set", limit).await;
        call(&mut client, 2, "run.start", json!({ "run_id": "charging" })).await;
        call(&mut client, 3, "run.start", json!({ "run_id": "other" })).await;
        faulted.fail_at(faulted.operations(), IoFault::Full);
        let charged = call(
            &mut client,
            4,
            "test.charge",
            json!({ "run_id": "charging" }),
        )
        .await;
        samples().await;
        faulted.free_space();
        let other = call(&mut client, 5, "run.finish", json!({ "run_id": "other" })).await;
        let resumed = [
            call(
                &mut client,
                6,
                "run.resume",
                json!({ "run_id": "charging" }),
            )
            .await,
            call(&mut client, 7, "run.resume", json!({ "run_id": "other" })).await,
        ];
        call(
            &mut client,
            8,
            "run.finish",
            json!({ "run_id": "charging" }),
        )
        .await;
        call(&mut client, 9, "run.finish", json!({ "run_id": "other" })).await;
        (charged, other, resumed)
    });

    let ledger = Connection::open_with_flags_and_vfs(
        crate::home::Home::new(env.home().to_owned()).usage_ledger(),
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        vfs.name(),
    )
    .unwrap();
    let integrity: String = ledger
        .query_row("PRAGMA integrity_check", [], |row| row.get(0))
        .unwrap();
    assert_eq!(
        (
            &charged["result"]["acknowledged"],
            &other["error"]["data"]["code"],
            &other["error"]["data"]["resource"],
            &resumed[0]["result"]["started_as"],
            &resumed[1]["result"]["started_as"],
            integrity.as_str(),
        ),
        (
            &json!(false),
            &json!("resources_exhausted"),
            &json!("storage"),
            &json!("resumed"),
            &json!("resumed"),
            "ok",
        ),
        "{other}"
    );
}

/// The crash VFS's disk is full from a commit on `run_state` on, so the
/// commit recording the stop fails too: the Run is left with no live
/// writer, interrupted as after a crash, its waiting `run.finish` raises
/// `resources_exhausted`, and once space is freed a resume goes on from its
/// last acknowledged commit.
///
/// Covers: row:RT-03, error:resources_exhausted, operation:run.resume
#[test]
fn when_the_commit_recording_the_stop_fails_too_the_waiting_call_raises_resources_exhausted() {
    let env = TestEnv::new();
    let vfs = CrashVfs::register(&env.unique_name("storage-full"), env.seed()).unwrap();
    let faulted = vfs.clone();

    let (failed, stopped, resumed, finished) = crashing(&env, &vfs, |mut client| async move {
        opened(&mut client).await;
        call(&mut client, 1, "run.start", json!({ "run_id": "full" })).await;
        call(
            &mut client,
            2,
            "test.note",
            json!({ "run_id": "full", "value": 1 }),
        )
        .await;
        faulted.fail_at(faulted.operations(), IoFault::Full);
        let failed = call(
            &mut client,
            3,
            "test.note",
            json!({ "run_id": "full", "value": 2 }),
        )
        .await;
        let stopped = call(&mut client, 4, "run.finish", json!({ "run_id": "full" })).await;
        faulted.free_space();
        let resumed = call(&mut client, 5, "run.resume", json!({ "run_id": "full" })).await;
        call(
            &mut client,
            6,
            "test.note",
            json!({ "run_id": "full", "value": 2 }),
        )
        .await;
        let finished = call(&mut client, 7, "run.finish", json!({ "run_id": "full" })).await;
        (failed, stopped, resumed, finished)
    });

    assert_eq!(
        (
            &failed["result"]["acknowledged"],
            &stopped["error"]["data"]["code"],
            &stopped["error"]["data"]["resource"],
            &resumed["result"]["started_as"],
            &finished["result"]["lifecycle"],
            notes_and_integrity(&env, &vfs, "full"),
        ),
        (
            &json!(false),
            &json!("resources_exhausted"),
            &json!("storage"),
            &json!("resumed"),
            &json!("finished"),
            (vec![1, 2], "ok".to_owned()),
        ),
        "{stopped}"
    );
}

/// A Run whose end its run file could not record, left interrupted with
/// no live writer (ADR 0122), is no longer counted live: the runtime's
/// `goodfetch.run.active` goes back down, as for an end that was recorded.
///
/// Covers: story:3-103, row:RT-03
#[test]
fn a_run_whose_end_could_not_be_recorded_is_no_longer_counted_active() {
    let env = TestEnv::new();
    let vfs = CrashVfs::register(&env.unique_name("storage-gauge"), env.seed()).unwrap();
    let faulted = vfs.clone();
    let registry = prometheus::Registry::new();
    let exporter = opentelemetry_prometheus::exporter()
        .with_registry(registry.clone())
        .without_target_info()
        .scope_info_enabled(false)
        .build()
        .unwrap();
    let provider = SdkMeterProvider::builder().with_reader(exporter).build();
    let engine = engine_with(Product::Collie, Some(noting()))
        .with_metrics(Metrics::new(&provider.meter(METER)));
    let active = move || {
        series(&registry)
            .into_iter()
            .filter(|series| series.starts_with("goodfetch_run_active"))
            .collect::<Vec<_>>()
    };
    let scraped = active.clone();

    let (while_live, stopped) = crashing_on(&engine, &env, &vfs, |mut client| async move {
        opened(&mut client).await;
        call(&mut client, 1, "run.start", json!({ "run_id": "gauged" })).await;
        let while_live = scraped();
        faulted.fail_at(faulted.operations(), IoFault::Full);
        call(
            &mut client,
            2,
            "test.note",
            json!({ "run_id": "gauged", "value": 1 }),
        )
        .await;
        let stopped = call(&mut client, 3, "run.finish", json!({ "run_id": "gauged" })).await;
        (while_live, stopped)
    });

    assert_eq!(
        (while_live, &stopped["error"]["data"]["code"], active()),
        (
            vec![r#"goodfetch_run_active{run_kind="collie"} 1"#.to_owned()],
            &json!("resources_exhausted"),
            vec![r#"goodfetch_run_active{run_kind="collie"} 0"#.to_owned()],
        ),
        "{stopped}"
    );
}

/// At `low`, a periodic Checkpoint onto that filesystem is skipped, while
/// the run file still holds the state; once space is freed the next one is
/// written.
///
/// Covers: row:RT-03, setting:checkpoint.interval
#[test]
fn a_periodic_checkpoint_onto_a_filesystem_at_low_is_skipped() {
    let env = TestEnv::new();
    configure(&env, LEVELS);
    let view = DiskView::default();
    let disk = view.mount(env.home(), 100 * MB, MB * 3 / 2);
    let freed = view.clone();
    let checkpoint = run_folder(&env, "periodic").join("checkpoint.sqlite");
    let watched = checkpoint.clone();

    let (skipped, written) = attached_on(&viewed(&view), &env, |mut client| async move {
        opened(&mut client).await;
        call(
            &mut client,
            1,
            "run.start",
            json!({ "run_id": "periodic", "settings": { "checkpoint": { "interval": 2000 } } }),
        )
        .await;
        tokio::time::sleep(Duration::from_secs(5)).await;
        let skipped = !watched.exists();
        freed.set_free(disk, 50 * MB);
        tokio::time::sleep(Duration::from_secs(5)).await;
        let written = watched.exists();
        call(
            &mut client,
            2,
            "run.finish",
            json!({ "run_id": "periodic" }),
        )
        .await;
        (skipped, written)
    });

    assert_eq!((skipped, written), (true, true), "{}", checkpoint.display());
}

/// At `critical`, `run.checkpoint` onto that filesystem fails at once with
/// `file_write_failed`, side `engine`, reason `storage_full`, writing
/// nothing.
///
/// Covers: row:RT-03, operation:run.checkpoint, error:file_write_failed
#[test]
fn a_checkpoint_onto_a_filesystem_at_critical_fails_and_writes_nothing() {
    let env = TestEnv::new();
    configure(&env, LEVELS);
    let view = DiskView::default();
    view.mount(env.home(), 100 * MB, 50 * MB);
    let full = env.folder().join("full");
    std::fs::create_dir_all(&full).unwrap();
    view.mount(&full, 100 * MB, MB / 2);
    let location = full.join("copy.sqlite");
    let asked = location.display().to_string();

    let failed = attached_on(&viewed(&view), &env, |mut client| async move {
        opened(&mut client).await;
        call(&mut client, 1, "run.start", json!({ "run_id": "copied" })).await;
        let failed = call(
            &mut client,
            2,
            "run.checkpoint",
            json!({ "run_id": "copied", "location": asked }),
        )
        .await;
        call(&mut client, 3, "run.finish", json!({ "run_id": "copied" })).await;
        failed
    });

    let data = &failed["error"]["data"];
    assert_eq!(
        (
            &data["code"],
            &data["reason"],
            &data["side"],
            location.exists()
        ),
        (
            &json!("file_write_failed"),
            &json!("storage_full"),
            &json!("engine"),
            false
        ),
        "{failed}"
    );
}
