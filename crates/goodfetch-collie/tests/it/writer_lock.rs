//! The writer lock and the Run index across processes on Collie's
//! executable (spec #213's Seam 1; ADRs 0031 and 0037's consequence): an
//! abort of a Run the Daemon writes goes to the Daemon, the owner the Run
//! index names; a killed Daemon's stale row never blocks another engine's
//! takeover, since a dead process holds no lock; and `runs reindex`
//! rebuilds the Run index from the run folders, keeping a missing Run's row
//! as missing and an erased one erased.
#![expect(
    clippy::disallowed_methods,
    reason = "the tests start processes and read and change the Run index and run folders themselves"
)]

use std::io::{BufRead as _, BufReader};
use std::path::Path;
use std::process::Stdio;

use goodfetch_engine::run_state::run_row;
use goodfetch_protocol::run::Lifecycle;
use goodfetch_test_support::{RuntimeFile, TestEnv, cli};
use rusqlite::Connection;
use serde_json::{Value, json};

use crate::attached::{Attached, hello};
use crate::daemon::{RawClient, StopsTheDaemon, json_of, short_idle, validate};
use crate::run::call;

const COLLIE: &str = env!("CARGO_BIN_EXE_goodfetch-collie");

fn index(env: &TestEnv) -> Connection {
    Connection::open(env.home().join("0.0").join("run-index.sqlite")).unwrap()
}

/// The Run index row of `run_id`: its lifecycle, owner, presence and
/// location.
fn row(env: &TestEnv, run_id: &str) -> Value {
    let row: String = index(env)
        .query_row(
            "SELECT json_object('lifecycle', lifecycle, 'owner', json(owner), 'presence', presence, 'location', location)
             FROM runs WHERE run_id = ?1",
            [run_id],
            |row| row.get(0),
        )
        .unwrap();
    serde_json::from_str(&row).unwrap()
}

fn lifecycle(folder: &Path) -> Lifecycle {
    run_row::lifecycle(&Connection::open(folder.join("run.sqlite")).unwrap()).unwrap()
}

fn runs(env: &TestEnv) -> std::path::PathBuf {
    env.home().join("0.0").join("runs")
}

/// Starts the Daemon of the test's home as a child of the test, and
/// answers its runtime file.
fn launched(env: &TestEnv) -> (goodfetch_test_support::Process, RuntimeFile) {
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
    let found = serde_json::from_str(&ready).unwrap();
    (daemon, found)
}

/// Aborts `run_id` through an Attached engine of the test's home.
fn aborted_by_an_attached_engine(env: &TestEnv, run_id: &str) -> Value {
    let mut engine = Attached::start(env);
    engine.send(&hello("collie", "0.1"));
    let _ = engine.next();
    let aborted = call(&mut engine, 1, "run.abort", &json!({ "run_id": run_id }));
    engine.finish();
    aborted
}

/// Covers: operation:run.abort
#[test]
fn an_abort_of_a_run_the_daemon_writes_goes_to_the_daemon_the_run_index_names() {
    let env = TestEnv::new();
    short_idle(&env, 60_000);
    let _stops = StopsTheDaemon::of(&env, COLLIE);
    json_of(COLLIE.as_ref(), &env, &["daemon", "start", "--json"]);
    let found =
        RuntimeFile::read(&RuntimeFile::path(env.home(), env!("CARGO_PKG_VERSION"))).unwrap();
    let mut client = RawClient::opened(&found, "collie");
    client.call(1, "run.start", &json!({ "run_id": "live" }));
    let owned = row(&env, "live");

    let aborted = aborted_by_an_attached_engine(&env, "live");

    assert_eq!(
        (
            owned["owner"].clone(),
            aborted["result"]["lifecycle"].clone(),
            row(&env, "live"),
        ),
        (
            json!({ "daemon": serde_json::to_value(&found.endpoint).unwrap() }),
            json!("stopped"),
            json!({ "lifecycle": "stopped", "owner": null, "presence": "present", "location": "runs/live" }),
        )
    );
}

/// Covers: operation:run.abort
#[test]
fn a_killed_daemons_stale_row_never_blocks_another_engines_takeover() {
    let env = TestEnv::new();
    short_idle(&env, 60_000);
    let (daemon, found) = launched(&env);
    let mut client = RawClient::opened(&found, "collie");
    client.call(1, "run.start", &json!({ "run_id": "cut" }));
    daemon.child().kill().unwrap();
    daemon.child().wait().unwrap();
    let stale = row(&env, "cut");

    let aborted = aborted_by_an_attached_engine(&env, "cut");

    assert_eq!(
        (
            stale["owner"].is_object(),
            aborted["result"]["lifecycle"].clone(),
            lifecycle(&runs(&env).join("cut")),
            row(&env, "cut")["owner"].clone(),
        ),
        (true, json!("stopped"), Lifecycle::Stopped, json!(null))
    );
}

/// Covers: command:runs.reindex, format:run-index
#[test]
fn runs_reindex_rebuilds_the_run_index_keeping_missing_rows_and_erased_ones() {
    let env = TestEnv::new();
    let mut engine = Attached::start(&env);
    engine.send(&hello("collie", "0.1"));
    let _ = engine.next();
    for (id, run_id) in (1..).step_by(2).zip(["kept", "gone", "erased", "moved"]) {
        call(&mut engine, id, "run.start", &json!({ "run_id": run_id }));
        call(
            &mut engine,
            id + 1,
            "run.finish",
            &json!({ "run_id": run_id }),
        );
    }
    engine.finish();
    let elsewhere = env.folder().join("elsewhere");
    std::fs::create_dir_all(&elsewhere).unwrap();
    std::fs::rename(runs(&env).join("moved"), elsewhere.join("moved")).unwrap();
    std::fs::remove_dir_all(runs(&env).join("gone")).unwrap();
    // A Schedule's retention discards a Run so (ADR 0036).
    std::fs::remove_dir_all(runs(&env).join("erased")).unwrap();
    index(&env)
        .execute(
            "UPDATE runs SET presence = 'erased' WHERE run_id = 'erased'",
            [],
        )
        .unwrap();
    index(&env)
        .execute("DELETE FROM runs WHERE run_id = 'kept'", [])
        .unwrap();

    let output = cli(&env, COLLIE)
        .args([
            "runs",
            "reindex",
            &elsewhere.display().to_string(),
            "--json",
        ])
        .run()
        .unwrap();

    let printed: Value = serde_json::from_str(&output.stdout).unwrap();
    validate(&printed, "RunsReindexOutput");
    let moved = elsewhere
        .join("moved")
        .display()
        .to_string()
        .replace('\\', "/");
    assert_eq!(
        (
            output.exit,
            [
                printed["present"].clone(),
                printed["missing"].clone(),
                printed["erased"].clone()
            ],
            ["kept", "gone", "erased", "moved"].map(|run_id| row(&env, run_id)),
        ),
        (
            Some(0),
            [json!(2), json!(1), json!(1)],
            [
                json!({ "lifecycle": "finished", "owner": null, "presence": "present", "location": "runs/kept" }),
                json!({ "lifecycle": "finished", "owner": null, "presence": "missing", "location": "runs/gone" }),
                json!({ "lifecycle": "finished", "owner": null, "presence": "erased", "location": "runs/erased" }),
                json!({ "lifecycle": "finished", "owner": null, "presence": "present", "location": moved }),
            ],
        )
    );
}
