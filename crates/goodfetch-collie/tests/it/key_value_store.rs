//! A Run's Key-value store across processes on Collie's executable (spec
//! #213's Seam 1; ADRs 0031 and 0102): a value set before a `SIGKILL` is
//! found by the Run resumed by its ID in the same home and by the Run
//! resumed from its Checkpoint in another, which stands for another
//! machine; a view of the Run no process holds reads it without the
//! Run's writer lock and never resumes it; and a key out of shape is
//! refused with `value_invalid` naming it.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]
#![expect(
    clippy::disallowed_methods,
    reason = "the tests hold the Run's lock file and read its run file themselves; their SQL statement is their own, on no Shared path (ADR 0140)"
)]

use std::fs::OpenOptions;
use std::path::Path;

use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::home::RUN_FILE;
use goodfetch_test_client::{Attached, HomeLayout, hello};
use goodfetch_test_support::TestEnv;
use rusqlite::Connection;
use serde_json::{Value, json};

use crate::COLLIE;
use crate::clean_up_rules::result;

/// Collie's Attached engine in `env`'s home, past its `hello`.
fn opened(env: &TestEnv) -> Attached {
    let mut engine = Attached::start(env, COLLIE);
    engine.send(&hello(Product::Collie));
    let _ = engine.next();
    engine
}

/// Starts the Run `run_id` in `env`'s home, sets each of `values` in its
/// store, and kills the engine right after the last answer.
fn killed_after_setting(env: &TestEnv, run_id: &str, values: &[(&str, Value)]) {
    let mut engine = opened(env);
    result(engine.ask(1, "run.start", &json!({ "run_id": run_id })));
    for (id, (key, value)) in (2..).zip(values) {
        result(engine.ask(
            id,
            "key_value_store.set",
            &json!({ "run_id": run_id, "key": key, "value": value }),
        ));
    }
    engine.kill();
}

/// The run epoch and Lifecycle the run file in `folder` holds.
fn run_row(folder: &Path) -> Value {
    let row: String = Connection::open(folder.join(RUN_FILE))
        .unwrap()
        .query_row(
            "SELECT json_object('epoch', epoch, 'lifecycle', lifecycle) FROM run",
            [],
            |row| row.get(0),
        )
        .unwrap();
    serde_json::from_str(&row).unwrap()
}

/// A value set before a `SIGKILL` is found by the Run resumed by its ID,
/// and, once that Run is paused, by the Run resumed from its Checkpoint in
/// another home; there, a key out of shape is refused with `value_invalid`
/// naming the key and what is wrong with it. Each way a key is out of shape
/// is a case of the engine's own tests; one stands for them here.
///
/// Covers: story:6-32, story:6-34, operation:run.resume, operation:key_value_store.set, operation:key_value_store.get, error:value_invalid
#[test]
fn a_value_set_before_a_kill_is_found_by_the_run_resumed_by_its_id_and_from_its_checkpoint_elsewhere()
 {
    let (first, second) = (TestEnv::new(), TestEnv::new());
    killed_after_setting(&first, "kept", &[("cursor", json!({ "page": 4 }))]);

    let mut resuming = opened(&first);
    let resumed = result(resuming.ask(1, "run.resume", &json!({ "run_id": "kept" })));
    let here = result(resuming.ask(
        2,
        "key_value_store.get",
        &json!({ "run_id": "kept", "key": "cursor" }),
    ));
    result(resuming.ask(3, "run.pause", &json!({ "run_id": "kept" })));
    assert!(resuming.finished());
    let checkpoint = HomeLayout::of(first.home())
        .run_folder("kept")
        .join("checkpoint.sqlite");

    let mut elsewhere = opened(&second);
    let moved = result(elsewhere.ask(
        1,
        "run.resume",
        &json!({ "run_id": "kept", "checkpoint": checkpoint }),
    ));
    let there = result(elsewhere.ask(
        2,
        "key_value_store.get",
        &json!({ "run_id": "kept", "key": "cursor" }),
    ));
    let refused = elsewhere.ask(
        3,
        "key_value_store.set",
        &json!({ "run_id": "kept", "key": "a\u{7}b", "value": 1 }),
    );
    let finished = result(elsewhere.ask(4, "run.finish", &json!({ "run_id": "kept" })));
    assert!(elsewhere.finished());

    let data = &refused["error"]["data"];
    assert_eq!(
        (
            (&resumed["started_as"], &here, &moved["started_as"], &there),
            (
                &data["code"],
                &data["key"],
                &data["refused"],
                &data["reason"]
            ),
            &finished["lifecycle"],
        ),
        (
            (
                &json!("resumed"),
                &json!({ "value": { "page": 4 } }),
                &json!("resumed"),
                &json!({ "value": { "page": 4 } }),
            ),
            (
                &json!("value_invalid"),
                &json!("a\u{7}b"),
                &json!("key"),
                &json!("key_control_character"),
            ),
            &json!("finished"),
        )
    );
}

/// A view of a Run a `SIGKILL` left reads its store while the test holds
/// the Run's writer lock, so it never takes the lock: `get` and `keys`
/// answer what was set, a write is refused with `run_not_found`, and the
/// Run stays as it was, in its first run epoch and running.
///
/// Covers: story:6-35, operation:key_value_store.get, operation:key_value_store.keys
#[test]
fn a_view_of_a_run_no_process_holds_reads_its_store_without_its_writer_lock_and_never_resumes_it() {
    let env = TestEnv::new();
    killed_after_setting(
        &env,
        "left",
        &[("b", json!(2)), ("a", json!(1)), ("c", json!(3))],
    );
    let folder = HomeLayout::of(env.home()).run_folder("left");
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .open(folder.join("run.lock"))
        .unwrap();
    lock.try_lock().unwrap();

    let mut viewer = opened(&env);
    let got = viewer.ask(
        1,
        "key_value_store.get",
        &json!({ "run_id": "left", "key": "b" }),
    );
    viewer.send(&json!({
        "jsonrpc": "2.0", "id": 2, "method": "key_value_store.keys", "params": { "run_id": "left" },
    }));
    viewer.send(&json!({
        "jsonrpc": "2.0", "method": "$/credit", "params": { "id": 2, "credits": 10 },
    }));
    let mut listed = Vec::new();
    while listed.len() < 4 {
        let message = viewer.next().expect("the engine closed");
        if message["method"] == "$/key" && message["params"]["id"] == 2 {
            listed.push(message["params"]["key"].clone());
        } else if message["id"] == 2 {
            listed.push(message.get("result").cloned().unwrap_or(message));
        }
    }
    let set = viewer.ask(
        3,
        "key_value_store.set",
        &json!({ "run_id": "left", "key": "d", "value": 4 }),
    );
    assert!(viewer.finished());
    lock.unlock().unwrap();

    assert_eq!(
        (
            &got["result"],
            listed,
            &set["error"]["data"]["code"],
            run_row(&folder),
        ),
        (
            &json!({ "value": 2 }),
            vec![json!("a"), json!("b"), json!("c"), json!({ "count": 3 })],
            &json!("run_not_found"),
            json!({ "epoch": 1, "lifecycle": "running" }),
        )
    );
}
