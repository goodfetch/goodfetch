//! Reading a Run another engine works, across processes on Collie's
//! executable (spec #213's Seam 1; ADRs 0031 and 0102): an Attached engine
//! reads the progress, the Key-value store and the report of a Run the
//! Daemon writes through the Daemon, the owner the Run index names, so the
//! progress is the live one, not the last value of committed state, and
//! the report arrives at the Run's end without ending it.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use goodfetch_test_support::{RuntimeFile, TestEnv};
use serde_json::{Value, json};

use crate::attached::{Attached, hello};
use crate::daemon::{RawClient, StopsTheDaemon, json_of, short_idle};
use crate::run::call;

const COLLIE: &str = env!("CARGO_BIN_EXE_goodfetch-collie");

/// Reads `engine`'s messages until the answer to the call `id`.
fn answer_to(engine: &mut Attached, id: i64) -> Value {
    loop {
        let message = engine.next().expect("the engine closed");
        if message["id"] == json!(id) {
            return message;
        }
    }
}

/// Covers: story:3-165, operation:run.progress, operation:run.report, operation:key_value_store.get
#[test]
fn an_attached_engine_reads_a_run_the_daemon_works_through_the_daemon() {
    let env = TestEnv::new();
    short_idle(&env, 60_000);
    let _stops = StopsTheDaemon::of(&env, COLLIE);
    json_of(COLLIE.as_ref(), &env, &["daemon", "start", "--json"]);
    let found =
        RuntimeFile::read(&RuntimeFile::path(env.home(), env!("CARGO_PKG_VERSION"))).unwrap();
    let mut owner = RawClient::opened(&found, "collie");
    owner.call(1, "run.start", &json!({ "run_id": "followed" }));
    owner.call(
        2,
        "run.set_progress_message",
        &json!({ "run_id": "followed", "message": "Half way" }),
    );
    owner.call(
        3,
        "key_value_store.set",
        &json!({ "run_id": "followed", "key": "cursor", "value": 3 }),
    );
    let mut reader = Attached::start(&env);
    reader.send(&hello("collie", "0.1"));
    let _ = reader.next();

    let progress = call(
        &mut reader,
        1,
        "run.progress",
        &json!({ "run_id": "followed" }),
    );
    let value = call(
        &mut reader,
        2,
        "key_value_store.get",
        &json!({ "run_id": "followed", "key": "cursor" }),
    );
    reader.send(&json!({
        "jsonrpc": "2.0", "id": 3, "method": "run.report",
        "params": { "run_id": "followed" },
    }));
    let finished = owner.call(4, "run.finish", &json!({ "run_id": "followed" }));
    let report = answer_to(&mut reader, 3);
    reader.finish();

    let progress = &progress["result"];
    assert_eq!(
        (
            [&progress["writer"], &progress["message"], &progress["last"]],
            &value["result"]["value"],
            &finished["result"]["lifecycle"],
            &report["result"]["lifecycle"],
        ),
        (
            [&json!("live"), &json!("Half way"), &json!(false)],
            &json!(3),
            &json!("finished"),
            &json!("finished"),
        )
    );
}
