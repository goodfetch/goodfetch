//! A Run on Collie's executable over stdio (spec #213's Seam 1): its file in
//! the per-user, per-major data folder of the Goodfetch home, or of the OS's
//! usual folders when `GOODFETCH_HOME` is unset (ADR 0059), in WAL mode
//! under its writer's lock, listed in the Run index, with its three axes
//! when it ends, and its stop once its `timeout.run` passed.

#![expect(
    clippy::disallowed_methods,
    reason = "a test's own statements, on no Shared path (ADR 0140)"
)]
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use std::path::Path;

use goodfetch_protocol::catalog::Product;
use goodfetch_test_client::HomeLayout;
#[cfg(unix)]
use goodfetch_test_support::MESSAGE_DEADLINE;
use goodfetch_test_support::TestEnv;
use rusqlite::Connection;
use serde_json::{Value, json};

use goodfetch_test_client::{Attached, hello};

use crate::COLLIE;

/// Starts and finishes the Run `run_id`, and answers its report.
fn start_and_finish(engine: &mut Attached, run_id: &str) -> Value {
    engine.send(&hello(Product::Collie));
    let _ = engine.next();
    engine.ask(1, "run.start", &json!({ "run_id": run_id }));
    engine.ask(2, "run.finish", &json!({ "run_id": run_id }))
}

fn journal_mode(file: &Path) -> String {
    Connection::open(file)
        .unwrap()
        .pragma_query_value(None, "journal_mode", |row| row.get(0))
        .unwrap()
}

/// Covers: story:3-25, story:3-28, operation:run.start, operation:run.finish
#[test]
fn a_run_on_collies_executable_keeps_its_own_wal_file_in_the_goodfetch_homes_per_major_folder() {
    let env = TestEnv::new();
    let mut engine = Attached::start(&env, COLLIE);

    let report = start_and_finish(&mut engine, "first");
    let (rest, exited) = engine.finish();

    let layout = HomeLayout::of(env.home());
    let file = layout.run_file("first");
    let listed: String = Connection::open(layout.run_index())
        .unwrap()
        .query_row(
            "SELECT lifecycle FROM runs WHERE run_id = 'first'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        (
            (
                &report["result"]["lifecycle"],
                &report["result"]["completeness"]
            ),
            journal_mode(&file),
            layout.run_folder("first").join("run.lock").exists(),
            listed,
            (rest, exited),
        ),
        (
            (&json!("finished"), &json!("complete")),
            "wal".to_owned(),
            true,
            "finished".to_owned(),
            (vec![], true),
        )
    );
}

/// Without `GOODFETCH_HOME`, the run files go to the OS's usual data folder
/// (ADR 0059). Windows' Known Folders ignore the environment, so the case
/// runs where the environment moves the data folder.
///
/// Covers: story:3-28
#[cfg(unix)]
#[test]
fn without_goodfetch_home_a_run_goes_to_the_oss_usual_data_folder() {
    let env = TestEnv::new();
    let mut engine = Attached::start_with(&env, COLLIE, MESSAGE_DEADLINE, |command| {
        command.env_remove("GOODFETCH_HOME");
    });

    start_and_finish(&mut engine, "usual");
    let _ = engine.finish();

    let data = if cfg!(target_os = "macos") {
        Path::new(&env.env()["HOME"])
            .join("Library")
            .join("Application Support")
    } else {
        Path::new(&env.env()["XDG_DATA_HOME"]).to_owned()
    };
    let file = HomeLayout::of(&data.join("goodfetch")).run_file("usual");
    assert_eq!(
        (file.exists(), HomeLayout::of(env.home()).runs().exists()),
        (true, false)
    );
}

/// A Run past its `timeout.run` stops at its next safe point, at once for a
/// Run its program drives, `stopped` and `incomplete` with the stop reason
/// `timeout` and no Error code of its own; the test sets a short value, as
/// every duration the engine waits on is a setting (ADR 0059).
///
/// Covers: story:3-52, setting:timeout.run, operation:events.subscribe, event:run.axes_changed
#[test]
fn a_run_past_a_short_run_timeout_stops_with_the_stop_reason_timeout() {
    let env = TestEnv::new();
    let mut engine = Attached::start(&env, COLLIE);
    engine.send(&hello(Product::Collie));
    let _ = engine.next();
    engine.send(
        &json!({ "jsonrpc": "2.0", "id": "events", "method": "events.subscribe", "params": {} }),
    );
    engine.send(&json!({ "jsonrpc": "2.0", "method": "$/credit", "params": { "id": "events", "credits": 16 } }));
    engine.send(&json!({
        "jsonrpc": "2.0", "id": "start", "method": "run.start",
        "params": { "run_id": "timed", "settings": { "timeout": { "run": 200 } } },
    }));

    let stopped = loop {
        let message = engine.next().unwrap();
        let events = message["params"]["events"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        if let Some(event) = events.into_iter().find(|event| {
            event["event"] == "run.axes_changed" && event["data"]["lifecycle"] == "stopped"
        }) {
            break event;
        }
    };
    engine.send(&json!({
        "jsonrpc": "2.0", "id": 1, "method": "run.finish", "params": { "run_id": "timed" },
    }));
    // The Run's last progress value may come between.
    let report = loop {
        let message = engine.next().unwrap();
        if message["id"] == 1 {
            break message;
        }
    };
    let (_, exited) = engine.finish();

    assert_eq!(
        (
            &stopped["data"]["run_id"],
            &report["result"]["lifecycle"],
            &report["result"]["completeness"],
            &report["result"]["stop_reason"],
            exited,
        ),
        (
            &json!("timed"),
            &json!("stopped"),
            &json!("incomplete"),
            &json!({ "kind": "timeout" }),
            true,
        )
    );
}

/// A progress value but its position and the moment it was read, which
/// only order values.
fn settled(mut progress: Value) -> Value {
    if let Some(fields) = progress.as_object_mut() {
        fields.remove("position");
        fields.remove("at");
    }
    progress
}

/// The program's own status line on Collie's executable (RT-15): carried
/// on `run.progress`, in the Run's state and in its Run index row.
///
/// Covers: story:3-129, operation:run.set_progress_message, event:run.progress
#[test]
fn a_progress_message_on_collies_executable_is_carried_on_progress_and_kept() {
    let env = TestEnv::new();
    let mut engine = Attached::start(&env, COLLIE);
    engine.send(&hello(Product::Collie));
    let _ = engine.next();
    engine.send(
        &json!({ "jsonrpc": "2.0", "id": "events", "method": "events.subscribe", "params": {} }),
    );
    engine.send(&json!({ "jsonrpc": "2.0", "method": "$/credit", "params": { "id": "events", "credits": 16 } }));
    engine.send(&json!({
        "jsonrpc": "2.0", "id": "start", "method": "run.start", "params": { "run_id": "noted" },
    }));
    while engine.next().unwrap()["id"] != "start" {}
    engine.send(&json!({
        "jsonrpc": "2.0", "id": "message", "method": "run.set_progress_message",
        "params": { "run_id": "noted", "message": "Reading page 3 of 10" },
    }));

    let (mut answer, mut progress) = (None, None);
    while answer.is_none() || progress.is_none() {
        let message = engine.next().unwrap();
        if message["id"] == "message" {
            answer = Some(message);
            continue;
        }
        let events = message["params"]["events"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        if let Some(event) = events
            .into_iter()
            .find(|event| event["event"] == "run.progress")
        {
            progress = Some(event["data"].clone());
        }
    }
    let (_, exited) = engine.finish();

    let layout = HomeLayout::of(env.home());
    let in_index: Option<String> = Connection::open(layout.run_index())
        .unwrap()
        .query_row(
            "SELECT progress_message FROM runs WHERE run_id = 'noted'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let in_state: Option<String> = Connection::open(layout.run_file("noted"))
        .unwrap()
        .query_row("SELECT progress_message FROM run", [], |row| row.get(0))
        .unwrap();
    let expected = json!({
        "run_id": "noted",
        "lifecycle": "running",
        "writer": "live",
        "epoch": 1,
        "pauses": [],
        "message": "Reading page 3 of 10",
        "last": false,
    });
    assert_eq!(
        (
            settled(answer.unwrap()["result"].clone()),
            settled(progress.unwrap()),
            in_state,
            in_index,
            exited,
        ),
        (
            expected.clone(),
            expected,
            Some("Reading page 3 of 10".to_owned()),
            Some("Reading page 3 of 10".to_owned()),
            true,
        )
    );
}

/// A Collie engine leaves the Politeness file's domain limits, which are
/// Foxhound's, untouched (ADR 0098), and a Collie Run takes
/// `politeness.sharing`, which also governs its proxies' limits (ADR 0088),
/// marked in its result when not `shared`.
///
/// Covers: story:3-85, setting:politeness.sharing, format:politeness-file
#[test]
#[expect(
    clippy::disallowed_methods,
    reason = "the test reads the Politeness file's bytes itself"
)]
fn a_collie_run_leaves_the_politeness_files_domain_limits_untouched() {
    let env = TestEnv::new();
    let file = env.home().join("politeness.sqlite");
    Connection::open(&file)
        .unwrap()
        .execute_batch(
            "CREATE TABLE domain_limits (key TEXT NOT NULL PRIMARY KEY, ready_at INTEGER,
                 backoffs INTEGER NOT NULL, ceiling INTEGER, ceiling_changed_at INTEGER,
                 last_start INTEGER) STRICT;
             INSERT INTO domain_limits VALUES ('shop.test', 1767603602000, 1, 2, 1767603600000, 1767603600000);",
        )
        .unwrap();
    let before = std::fs::read(&file).unwrap();
    let mut engine = Attached::start(&env, COLLIE);

    engine.send(&hello(Product::Collie));
    let _ = engine.next();
    engine.ask(
        1,
        "run.start",
        &json!({ "run_id": "program", "settings": { "politeness": { "sharing": "per_daemon" } } }),
    );
    let report = engine.ask(2, "run.finish", &json!({ "run_id": "program" }));
    let (_, exited) = engine.finish();

    assert_eq!(
        (
            &report["result"]["weakened"],
            exited,
            std::fs::read(&file).unwrap() == before,
        ),
        (
            &json!([{ "key": "politeness.sharing", "value": "per_daemon", "layer": "run", "origin": "settings" }]),
            true,
            true,
        )
    );
}
