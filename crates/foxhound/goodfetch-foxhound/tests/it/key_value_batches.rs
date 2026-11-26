//! A crawl attempt's Key-value store writes and the Run-state batches, on
//! Foxhound's executable over stdio (spec #2138's Seam 1; ADRs 0103 and
//! 0142): the writes an attempt staged commit in the batch that carries
//! its outcome, so a `SIGKILL` before that batch is written redoes the
//! attempt and one after it keeps them; meanwhile the attempt reads back
//! what it set, and another attempt or the program the last written value.
//! A `key_value_store.set` outside an attempt is a barrier: it is answered
//! once it is in the run file, with the open batch it commits.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use goodfetch_test_client::Attached;
use goodfetch_test_support::TestEnv;
use rstest::rstest;
use serde_json::{Value, json};

use crate::frontier::crawl_with;
use crate::site::Site;
use crate::staging::keys;

/// The Run ID of every crawl here.
const RUN_ID: &str = "batched";

/// The Run's settings at the Durability level `level`, with no threshold
/// sealing a batch at `batch` and `off`: only a barrier, or the Run's end,
/// writes the open batch.
fn at(level: &str) -> Value {
    let mut run_state = json!({ "durability": level });
    if matches!(level, "batch" | "off") {
        run_state["batch"] = json!({ "pages": "none", "interval": "none", "bytes": "none" });
    }
    json!({ "run_state": run_state })
}

/// The seeds `/a` then `/b`, which one handler at a time takes in order.
fn seeds(site: &Site) -> Vec<Value> {
    ["/a", "/b"]
        .map(|path| json!({ "url": site.page(path), "label": "page" }))
        .to_vec()
}

/// The params of `key_value_store.<verb>` for the key `key`, through the
/// attempt `handle` gave when there is one, else as the program.
fn store(key: &str, handle: Option<&Value>) -> Value {
    let mut params = json!({ "run_id": RUN_ID, "key": key });
    if let Some(handle) = handle {
        params["attempt"] = json!({
            "crawl_request_id": handle["crawl_request"]["id"],
            "lease_epoch": handle["lease_epoch"],
        });
    }
    params
}

/// The params of `key_value_store.set` setting `key` to `value`, as
/// [`store`] names its caller.
fn setting(key: &str, value: &Value, handle: Option<&Value>) -> Value {
    let mut params = store(key, handle);
    params["value"] = value.clone();
    params
}

/// The handler's answer that commits one Record naming its page.
fn record_the_page(handle: &Value) -> Value {
    json!({ "result": { "records": [{ "dataset": "pages", "data": { "page": handle["crawl_request"]["url"] } }] } })
}

/// The result of the call of `method` with `params` that `engine` answers
/// while a handler holds its one request: nothing else comes meanwhile but
/// the answer to `crawl.start`, which this drops.
fn called(engine: &mut Attached, id: &str, method: &str, params: &Value) -> Value {
    let reply = engine.call(id, method, params, &mut |request| {
        panic!("no other request comes while one is in flight: {request}")
    });
    reply
        .get("result")
        .cloned()
        .unwrap_or_else(|| panic!("{reply}"))
}

/// What a crawl of `/a` then `/b` under `settings` saw before a `SIGKILL`
/// while `/b`'s attempt was in flight: the program sets `progress` to 0
/// and `/a`'s attempt sets it to 1 and reads it back; then, after a
/// barrier when `barrier` holds, `/b`'s attempt and the program read it,
/// and the test reads the run file's keys.
fn killed_during_b(env: &TestEnv, site: &Site, settings: &Value, barrier: bool) -> Vec<Value> {
    let mut engine = site.engine(env);
    let mut settings = settings.clone();
    settings["crawl"]["handler"] = json!({ "concurrency": 1 });
    engine.send(&json!({
        "jsonrpc": "2.0", "id": "start", "method": "crawl.start",
        "params": { "run_id": RUN_ID, "seeds": seeds(site), "labels": { "page": {} }, "settings": settings },
    }));
    let mut own = Value::Null;
    loop {
        let message = engine.next().expect("the engine closed before /b");
        if message["method"] != "crawl_request.handle" {
            continue;
        }
        let handle = &message["params"];
        if handle["crawl_request"]["url"] == site.page("/a") {
            called(
                &mut engine,
                "earlier",
                "key_value_store.set",
                &setting("progress", &json!(0), None),
            );
            called(
                &mut engine,
                "staged",
                "key_value_store.set",
                &setting("progress", &json!(1), Some(handle)),
            );
            own = called(
                &mut engine,
                "own",
                "key_value_store.get",
                &store("progress", Some(handle)),
            );
            let mut reply = record_the_page(handle);
            reply["jsonrpc"] = json!("2.0");
            reply["id"] = message["id"].clone();
            engine.send(&reply);
            continue;
        }
        if barrier {
            called(
                &mut engine,
                "barrier",
                "key_value_store.set",
                &setting("marker", &json!(true), None),
            );
        }
        let other = called(
            &mut engine,
            "other",
            "key_value_store.get",
            &store("progress", Some(handle)),
        );
        let program = called(
            &mut engine,
            "program",
            "key_value_store.get",
            &store("progress", None),
        );
        let written = json!(keys(env, RUN_ID));
        engine.kill();
        return vec![own, other, program, written];
    }
}

/// A kill while `/b`'s attempt is in flight, after `/a`'s attempt staged
/// `progress` and answered: at `batch`, with no batch written since, `/b`'s
/// attempt and the program still read the earlier value, the run file
/// holds it, and the resumed crawl redoes `/a`; once a barrier wrote the
/// batch carrying `/a`'s outcome, and at `full` with none, they read the
/// new value, which the resumed Run keeps without redoing `/a`. `/a`'s
/// attempt reads back what it set either way.
///
/// Covers: story:6-236, setting:run_state.durability, operation:key_value_store.set, operation:key_value_store.get
#[rstest]
#[case::batch_with_no_batch_written("batch", false, false)]
#[case::batch_once_a_barrier_wrote_the_batch("batch", true, true)]
#[case::full("full", false, true)]
fn an_attempts_staged_values_commit_with_the_batch_that_carries_its_outcome(
    #[case] level: &str,
    #[case] barrier: bool,
    #[case] committed: bool,
) {
    let env = TestEnv::new();
    let site = Site::start(&env);
    let seen = killed_during_b(&env, &site, &at(level), barrier);

    let mut resuming = site.engine(&env);
    let mut handled = Vec::new();
    let finished = crawl_with(
        &mut resuming,
        RUN_ID,
        &seeds(&site),
        &at(level),
        &mut |_, handle| {
            handled.push(handle["crawl_request"]["url"].clone());
            record_the_page(handle)
        },
    );
    assert!(resuming.finished());

    let progress = if committed { "1" } else { "0" };
    let mut held = vec![json!(["progress", progress])];
    if barrier {
        held.insert(0, json!(["marker", "true"]));
    }
    let value = json!({ "value": i32::from(committed) });
    let mut redone = vec![json!(site.page("/b"))];
    if !committed {
        redone.insert(0, json!(site.page("/a")));
    }
    assert_eq!(
        (
            seen,
            handled,
            &finished["result"]["lifecycle"],
            json!(keys(&env, RUN_ID)),
        ),
        (
            vec![
                json!({ "value": 1 }),
                value.clone(),
                value,
                Value::Array(held.clone()),
            ],
            redone,
            &json!("finished"),
            Value::Array(held),
        )
    );
}

/// The program's `key_value_store.set` made while `/b`'s attempt is in
/// flight is answered only once it is in the run file, with `/a`'s
/// attempt's staged value, at each Durability level: at `batch` and `off`,
/// where no threshold sealed the open batch, the barrier writes it.
///
/// Covers: setting:run_state.durability, operation:key_value_store.set
#[rstest]
fn a_set_outside_an_attempt_is_answered_once_it_is_written_at_every_level(
    #[values("batch", "full", "fast", "periodic", "off")] level: &str,
) {
    let env = TestEnv::new();
    let site = Site::start(&env);
    let mut engine = site.engine(&env);
    let mut written = Vec::new();

    crawl_with(
        &mut engine,
        RUN_ID,
        &seeds(&site),
        &at(level),
        &mut |inside, handle| {
            if handle["crawl_request"]["url"] == site.page("/a") {
                inside.call(
                    "key_value_store.set",
                    &setting("progress", &json!(1), Some(handle)),
                );
            } else {
                inside.call("key_value_store.set", &setting("cursor", &json!(7), None));
                written = keys(&env, RUN_ID);
            }
            record_the_page(handle)
        },
    );
    assert!(engine.finished());

    assert_eq!(
        written,
        [
            ("cursor".to_owned(), "7".to_owned()),
            ("progress".to_owned(), "1".to_owned()),
        ]
    );
}
