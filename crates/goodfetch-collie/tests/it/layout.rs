//! The per-install layout through Collie's executable (spec #213's Seam 1;
//! ADR 0002): an Attached engine hands its Runs to the Daemon, which keeps
//! them in the install database, and a killed Daemon leaves every Run
//! resumable.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use std::io::{BufRead as _, BufReader};
use std::process::Stdio;

use goodfetch_test_client::HomeLayout;
use goodfetch_test_support::{RuntimeFile, TestEnv};
use rusqlite::Connection;
use serde_json::{Value, json};

use crate::attached::{Attached, hello};
use crate::daemon::{RawClient, StopsTheDaemon, json_of};
use crate::run::call;

const COLLIE: &str = env!("CARGO_BIN_EXE_goodfetch-collie");

/// Lays out the per-install layout and a Daemon that stops after a minute
/// idle.
fn per_install(env: &TestEnv) {
    env.user_configuration("daemon.idle_timeout = 60000\nrun_state.layout = \"per_install\"\n");
}

fn layout(env: &TestEnv) -> HomeLayout {
    HomeLayout::of(env.home())
}

/// The Run index row of `run_id`: its owner and lifecycle.
fn index_row(env: &TestEnv, run_id: &str) -> Value {
    let index = Connection::open(layout(env).run_index()).unwrap();
    let row: String = index
        .query_row(
            "SELECT json_object('owner', json(owner), 'lifecycle', lifecycle) FROM runs WHERE run_id = ?1",
            [run_id],
            |row| row.get(0),
        )
        .unwrap();
    serde_json::from_str(&row).unwrap()
}

/// Covers: story:3-29, setting:run_state.layout
#[test]
fn an_attached_engine_in_the_per_install_layout_hands_its_runs_to_the_daemon() {
    let env = TestEnv::new();
    per_install(&env);
    let _stops = StopsTheDaemon::of(&env, COLLIE);
    let mut engine = Attached::start(&env);

    engine.send(&hello("collie", "0.1"));
    let opened = engine.next().unwrap();
    let started = call(&mut engine, 1, "run.start", &json!({ "run_id": "handed" }));
    let owner = index_row(&env, "handed")["owner"].clone();
    let _ = engine.finish();

    assert_eq!(
        (
            opened["result"]["product"].clone(),
            started["result"]["started_as"].clone(),
            owner.get("daemon").is_some(),
            layout(&env).per_major().join("run-state.sqlite").is_file(),
            layout(&env).run_file("handed").exists(),
        ),
        (json!("collie"), json!("new"), true, true, false),
        "{owner}"
    );
}

/// Covers: story:3-29, operation:run.resume
#[test]
fn a_killed_daemon_in_the_per_install_layout_leaves_every_run_resumable() {
    let env = TestEnv::new();
    per_install(&env);
    let _stops = StopsTheDaemon::of(&env, COLLIE);
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
    let mut client = RawClient::opened(&found, "collie");
    client.call(1, "run.start", &json!({ "run_id": "first" }));
    client.call(2, "run.start", &json!({ "run_id": "second" }));

    daemon.child().kill().unwrap();
    daemon.child().wait().unwrap();
    json_of(COLLIE.as_ref(), &env, &["daemon", "start", "--json"]);
    let found =
        RuntimeFile::read(&RuntimeFile::path(env.home(), env!("CARGO_PKG_VERSION"))).unwrap();
    let mut client = RawClient::opened(&found, "collie");
    let first = client.call(1, "run.resume", &json!({ "run_id": "first" }));
    let second = client.call(2, "run.resume", &json!({ "run_id": "second" }));

    assert_eq!(
        (
            first["result"]["started_as"].clone(),
            second["result"]["started_as"].clone(),
            layout(&env).run_file("first").exists(),
        ),
        (json!("resumed"), json!("resumed"), false),
        "{first} {second}"
    );
}
