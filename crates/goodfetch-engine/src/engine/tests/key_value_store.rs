//! A Run's Key-value store, through the engine's public interface over the
//! in-memory network (spec #213's Seam 3; ADRs 0002, 0031, 0102 and 0103):
//! `get` answers null for an absent key and a set value with its nested
//! nulls; `set` refuses null with `value_invalid` naming `delete`; `delete`
//! answers whether the key existed; `keys` sends a prefix's keys in the
//! order of their Unicode scalar values, one per credit; a key that is
//! empty, too long or holds a control character is refused naming it; a
//! Run killed and resumed by its ID, or resumed from a Checkpoint in
//! another home, finds what it committed; and a Run no engine holds is
//! read while another holds its writer lock, never resumed, and refuses a
//! write. An engine dies here as a killed one does: its runtime is dropped
//! with the Run live.

use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};
use std::time::Duration;

use goodfetch_protocol::catalog::Product;
use goodfetch_test_support::TestEnv;
use rstest::rstest;
use rusqlite::Connection;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt as _, BufReader};

use super::checkpoint::two_machines;
use super::run::call;
use super::{Client, attached_on, dependencies, engine, hello};
use crate::transport::{self, Network};

fn collie() -> crate::engine::Engine {
    engine(Product::Collie)
}

/// Opens a Connection with `hello` and starts the Run `run_id`.
async fn started(mut client: Client, run_id: &str) -> Client {
    client.send(&[hello("collie", "0.1")]).await;
    client.next().await.unwrap();
    let reply = call(&mut client, 1, "run.start", json!({ "run_id": run_id })).await;
    assert!(reply.get("result").is_some(), "{reply}");
    client
}

/// The reply of `key_value_store.<verb>` on the Run `run_id`, as call `id`.
async fn store(client: &mut Client, id: i64, verb: &str, params: Value) -> Value {
    call(client, id, &format!("key_value_store.{verb}"), params).await
}

/// Has an engine of `env` start the Run `run_id`, set each of `values`,
/// then kills it: its runtime is dropped with the Run live.
fn killed_after_setting(env: &TestEnv, run_id: &str, values: &[(&str, Value)]) {
    let runtime = goodfetch_test_support::runtime();
    runtime.block_on(async {
        let (network, dialer) = transport::in_memory();
        let engine = collie();
        let serving = engine.run_attached(Network::InMemory(network), dependencies(env));
        let talk = async {
            let (reader, writer) = dialer.connect().await.unwrap().split();
            let mut client = started(
                Client {
                    lines: BufReader::new(reader).lines(),
                    writer,
                },
                run_id,
            )
            .await;
            for (id, (key, value)) in (2..).zip(values) {
                let reply = store(
                    &mut client,
                    id,
                    "set",
                    json!({ "run_id": run_id, "key": key, "value": value }),
                )
                .await;
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

/// The run file's run epoch and lifecycle.
fn run_row(folder: &Path) -> Value {
    let file = Connection::open(folder.join("run.sqlite")).unwrap();
    let row: String = file
        .query_row(
            "SELECT json_object('epoch', epoch, 'lifecycle', lifecycle) FROM run",
            [],
            |row| row.get(0),
        )
        .unwrap();
    serde_json::from_str(&row).unwrap()
}

fn lock_file(folder: &Path) -> File {
    OpenOptions::new()
        .read(true)
        .write(true)
        .open(folder.join("run.lock"))
        .unwrap()
}

/// Covers: story:6-28, operation:key_value_store.get
#[test]
fn get_on_an_absent_key_answers_null() {
    let got = attached_on(&collie(), &TestEnv::new(), |client| async move {
        let mut client = started(client, "fresh").await;
        store(
            &mut client,
            2,
            "get",
            json!({ "run_id": "fresh", "key": "cursor" }),
        )
        .await
    });

    assert_eq!(got["result"], json!({ "value": null }));
}

/// Covers: story:6-28, operation:key_value_store.set
#[test]
fn a_set_value_comes_back_from_get_with_its_nested_nulls() {
    let value = json!({ "page": 3, "seen": [1, null], "next": null });

    let (set, got) = attached_on(&collie(), &TestEnv::new(), |client| {
        let value = value.clone();
        async move {
            let mut client = started(client, "kept").await;
            let set = store(
                &mut client,
                2,
                "set",
                json!({ "run_id": "kept", "key": "cursor", "value": value }),
            )
            .await;
            let got = store(
                &mut client,
                3,
                "get",
                json!({ "run_id": "kept", "key": "cursor" }),
            )
            .await;
            (set, got)
        }
    });

    assert_eq!(
        (set["result"].clone(), got["result"].clone()),
        (json!({}), json!({ "value": value }))
    );
}

/// The later of two sets of one key wins.
/// Covers: story:6-28
#[test]
fn the_last_set_of_a_key_wins() {
    let got = attached_on(&collie(), &TestEnv::new(), |client| async move {
        let mut client = started(client, "twice").await;
        for (id, value) in [(2, 1), (3, 2)] {
            store(
                &mut client,
                id,
                "set",
                json!({ "run_id": "twice", "key": "count", "value": value }),
            )
            .await;
        }
        store(
            &mut client,
            4,
            "get",
            json!({ "run_id": "twice", "key": "count" }),
        )
        .await
    });

    assert_eq!(got["result"], json!({ "value": 2 }));
}

/// Covers: story:6-29, error:value_invalid
#[test]
fn set_of_null_fails_with_value_invalid_naming_delete_and_stores_nothing() {
    let (set, got) = attached_on(&collie(), &TestEnv::new(), |client| async move {
        let mut client = started(client, "nulled").await;
        let set = store(
            &mut client,
            2,
            "set",
            json!({ "run_id": "nulled", "key": "cursor", "value": null }),
        )
        .await;
        let got = store(
            &mut client,
            3,
            "get",
            json!({ "run_id": "nulled", "key": "cursor" }),
        )
        .await;
        (set, got)
    });

    let data = &set["error"]["data"];
    assert_eq!(
        (
            data["code"].clone(),
            data["retryable"].clone(),
            data["key"].clone(),
            data["refused"].clone(),
            data["reason"].clone(),
            data["fix"].as_str().unwrap().contains("`delete`"),
            got["result"].clone(),
        ),
        (
            json!("value_invalid"),
            json!(false),
            json!("cursor"),
            json!("value"),
            json!("value_null"),
            true,
            json!({ "value": null }),
        )
    );
}

/// Covers: story:6-31, operation:key_value_store.delete
#[test]
fn delete_answers_whether_the_key_existed() {
    let (first, second, got) = attached_on(&collie(), &TestEnv::new(), |client| async move {
        let mut client = started(client, "deleting").await;
        store(
            &mut client,
            2,
            "set",
            json!({ "run_id": "deleting", "key": "cursor", "value": 7 }),
        )
        .await;
        let key = json!({ "run_id": "deleting", "key": "cursor" });
        let first = store(&mut client, 3, "delete", key.clone()).await;
        let second = store(&mut client, 4, "delete", key.clone()).await;
        let got = store(&mut client, 5, "get", key).await;
        (first, second, got)
    });

    assert_eq!(
        (
            first["result"].clone(),
            second["result"].clone(),
            got["result"].clone()
        ),
        (
            json!({ "existed": true }),
            json!({ "existed": false }),
            json!({ "value": null })
        )
    );
}

/// What a `key_value_store.keys` call sent: the keys of its `$/key`
/// notifications, then its reply.
async fn keys_read(client: &mut Client, id: i64, until: usize) -> Vec<Value> {
    let mut read = Vec::new();
    while read.len() < until {
        let message = client.next().await.expect("the engine closed");
        if message["method"] == "$/key" && message["params"]["id"] == json!(id) {
            read.push(message["params"]["key"].clone());
        } else if message["id"] == json!(id) {
            read.push(message.get("result").cloned().unwrap_or(message));
        }
    }
    read
}

/// No key is sent before a credit is granted; then one per credit, in the
/// order of their Unicode scalar values, only those with the prefix, and
/// the call answers how many it sent.
/// Covers: story:6-31, operation:key_value_store.keys
#[test]
fn keys_sends_a_prefixs_keys_in_scalar_order_one_per_credit() {
    let (before_credit, first_two, rest) = attached_on(
        &collie(),
        &TestEnv::new(),
        |client| async move {
            let mut client = started(client, "listed").await;
            for (id, key) in (2..).zip(["page/é", "page/2", "other", "page/10", "page/Z"]) {
                store(
                    &mut client,
                    id,
                    "set",
                    json!({ "run_id": "listed", "key": key, "value": true }),
                )
                .await;
            }
            client
                .send(&[json!({
                    "jsonrpc": "2.0", "id": 10, "method": "key_value_store.keys",
                    "params": { "run_id": "listed", "prefix": "page/" },
                })])
                .await;
            let before_credit = tokio::time::timeout(Duration::from_secs(5), client.next())
                .await
                .is_err();
            let credit = |credits: u32| json!({ "jsonrpc": "2.0", "method": "$/credit", "params": { "id": 10, "credits": credits } });
            client.send(&[credit(2)]).await;
            let first_two = keys_read(&mut client, 10, 2).await;
            client.send(&[credit(10)]).await;
            let rest = keys_read(&mut client, 10, 3).await;
            (before_credit, first_two, rest)
        },
    );

    assert_eq!(
        (before_credit, first_two, rest),
        (
            true,
            vec![json!("page/10"), json!("page/2")],
            vec![json!("page/Z"), json!("page/é"), json!({ "count": 4 })],
        )
    );
}

/// Covers: story:6-31
#[test]
fn keys_of_an_empty_store_answers_at_once_with_none() {
    let read = attached_on(&collie(), &TestEnv::new(), |client| async move {
        let mut client = started(client, "empty").await;
        store(&mut client, 2, "keys", json!({ "run_id": "empty" })).await
    });

    assert_eq!(read["result"], json!({ "count": 0 }));
}

/// Covers: story:6-34, error:value_invalid
#[rstest]
#[case::empty("", "key_empty")]
#[case::longer_than_512_scalar_values(&"é".repeat(513), "key_too_long")]
#[case::a_bell("a\u{7}b", "key_control_character")]
#[case::a_line_break("line\nbreak", "key_control_character")]
#[case::a_c1_control("a\u{85}b", "key_control_character")]
fn a_key_out_of_shape_fails_with_value_invalid_naming_it(#[case] key: &str, #[case] reason: &str) {
    let given = key.to_owned();
    let (set, get) = attached_on(&collie(), &TestEnv::new(), |client| async move {
        let mut client = started(client, "keyed").await;
        let set = store(
            &mut client,
            2,
            "set",
            json!({ "run_id": "keyed", "key": given, "value": 1 }),
        )
        .await;
        let get = store(
            &mut client,
            3,
            "get",
            json!({ "run_id": "keyed", "key": given }),
        )
        .await;
        (set, get)
    });

    let refusal = |reply: &Value| {
        let data = &reply["error"]["data"];
        json!([data["code"], data["key"], data["refused"], data["reason"]])
    };
    let expected = json!(["value_invalid", key, "key", reason]);
    assert_eq!((refusal(&set), refusal(&get)), (expected.clone(), expected));
}

/// A key of exactly 512 scalar values, each two bytes in UTF-8, is kept.
/// Covers: story:6-34
#[test]
fn a_key_of_512_scalar_values_is_kept() {
    let key = "é".repeat(512);
    let given = key.clone();

    let got = attached_on(&collie(), &TestEnv::new(), |client| async move {
        let mut client = started(client, "longest").await;
        let set = store(
            &mut client,
            2,
            "set",
            json!({ "run_id": "longest", "key": given, "value": "kept" }),
        )
        .await;
        assert!(set.get("result").is_some(), "{set}");
        store(
            &mut client,
            3,
            "get",
            json!({ "run_id": "longest", "key": given }),
        )
        .await
    });

    assert_eq!(got["result"], json!({ "value": "kept" }));
}

/// Covers: story:6-32
#[test]
fn a_run_killed_and_resumed_by_its_id_finds_every_value_it_committed() {
    let env = TestEnv::new();
    killed_after_setting(
        &env,
        "killed",
        &[("cursor", json!({ "page": 4 })), ("seen", json!(12))],
    );

    let (resumed, cursor, seen) = attached_on(&collie(), &env, |mut client| async move {
        client.send(&[hello("collie", "0.1")]).await;
        client.next().await.unwrap();
        let resumed = call(&mut client, 1, "run.resume", json!({ "run_id": "killed" })).await;
        let cursor = store(
            &mut client,
            2,
            "get",
            json!({ "run_id": "killed", "key": "cursor" }),
        )
        .await;
        let seen = store(
            &mut client,
            3,
            "get",
            json!({ "run_id": "killed", "key": "seen" }),
        )
        .await;
        (resumed, cursor, seen)
    });

    assert_eq!(
        (
            resumed["result"]["started_as"].clone(),
            cursor["result"].clone(),
            seen["result"].clone(),
        ),
        (
            json!("resumed"),
            json!({ "value": { "page": 4 } }),
            json!({ "value": 12 })
        )
    );
}

/// A Run resumed in another Goodfetch home, which stands for another
/// machine, from a Checkpoint finds what it committed before it, and its
/// writes go on there.
/// Covers: story:6-32
#[test]
fn a_run_resumed_on_another_machine_from_a_checkpoint_finds_every_value_it_committed() {
    let (first, second) = (TestEnv::new(), TestEnv::new());
    let location = first.folder().join("moved.sqlite").display().to_string();

    let (cursor, later) = two_machines(&first, &second, |mut old, mut new| async move {
        old.ok(2, "run.start", json!({ "run_id": "moved" })).await;
        old.ok(
            3,
            "key_value_store.set",
            json!({ "run_id": "moved", "key": "cursor", "value": "page-9" }),
        )
        .await;
        old.ok(
            4,
            "run.migrate",
            json!({ "run_id": "moved", "location": location }),
        )
        .await;
        new.ok(
            2,
            "run.resume",
            json!({ "run_id": "moved", "checkpoint": location }),
        )
        .await;
        let cursor = new
            .ok(
                3,
                "key_value_store.get",
                json!({ "run_id": "moved", "key": "cursor" }),
            )
            .await;
        new.ok(
            4,
            "key_value_store.set",
            json!({ "run_id": "moved", "key": "cursor", "value": "page-10" }),
        )
        .await;
        let later = new
            .ok(
                5,
                "key_value_store.get",
                json!({ "run_id": "moved", "key": "cursor" }),
            )
            .await;
        new.ok(6, "run.finish", json!({ "run_id": "moved" })).await;
        (cursor, later)
    });

    assert_eq!(
        (cursor, later),
        (json!({ "value": "page-9" }), json!({ "value": "page-10" }))
    );
}

/// A view of a Run no engine holds reads its store while another holds the
/// Run's writer lock, so it never takes the lock; the Run stays as it was,
/// unresumed, and a write to it is refused.
/// Covers: story:6-35
#[test]
fn a_run_no_engine_holds_is_read_without_its_writer_lock_and_never_resumed() {
    let env = TestEnv::new();
    killed_after_setting(
        &env,
        "left",
        &[("b", json!(2)), ("a", json!(1)), ("c", json!(3))],
    );
    let folder = run_folder(&env, "left");
    let other = lock_file(&folder);
    other.try_lock().unwrap();

    let (got, listed, set) = attached_on(&collie(), &env, |mut client| async move {
        client.send(&[hello("collie", "0.1")]).await;
        client.next().await.unwrap();
        let got = store(
            &mut client,
            1,
            "get",
            json!({ "run_id": "left", "key": "b" }),
        )
        .await;
        client
            .send(&[
                json!({ "jsonrpc": "2.0", "id": 2, "method": "key_value_store.keys", "params": { "run_id": "left" } }),
                json!({ "jsonrpc": "2.0", "method": "$/credit", "params": { "id": 2, "credits": 10 } }),
            ])
            .await;
        let listed = keys_read(&mut client, 2, 4).await;
        let set = store(
            &mut client,
            3,
            "set",
            json!({ "run_id": "left", "key": "d", "value": 4 }),
        )
        .await;
        (got, listed, set)
    });
    other.unlock().unwrap();

    assert_eq!(
        (
            got["result"].clone(),
            listed,
            set["error"]["data"]["code"].clone(),
            run_row(&folder),
        ),
        (
            json!({ "value": 2 }),
            vec![json!("a"), json!("b"), json!("c"), json!({ "count": 3 })],
            json!("run_not_found"),
            json!({ "epoch": 1, "lifecycle": "running" }),
        )
    );
}

/// The params of `key_value_store.<verb>` on the Run `run_id`, through a
/// crawl attempt's `attempt`.
fn through_an_attempt(verb: &str, run_id: &str) -> Value {
    let mut params = json!({
        "run_id": run_id,
        "key": "cursor",
        "attempt": { "crawl_request_id": "r", "lease_epoch": 1 },
    });
    match verb {
        "set" => params["value"] = json!(1),
        "set_bytes" => {
            params["content"] = json!("");
            params["content_type"] = json!("text/plain");
        }
        _ => {}
    }
    params
}

/// A call through a crawl attempt names an attempt only a crawl's Run has:
/// on a live Run with no crawl it fails with `invalid_params`, and on a Run
/// that is not live with `run_not_found`, whatever its verb.
#[rstest]
fn a_call_through_an_attempt_fails_on_a_run_with_no_crawl_and_on_one_not_live(
    #[values("get", "get_bytes", "set", "set_bytes", "delete")] verb: &str,
) {
    let (live, absent) = attached_on(&collie(), &TestEnv::new(), |client| async move {
        let mut client = started(client, "collie").await;
        let live = store(&mut client, 2, verb, through_an_attempt(verb, "collie")).await;
        let absent = store(&mut client, 3, verb, through_an_attempt(verb, "absent")).await;
        (live, absent)
    });

    assert_eq!(
        (
            live["error"]["code"].clone(),
            absent["error"]["data"]["code"].clone()
        ),
        (json!(-32602), json!("run_not_found")),
        "{live} {absent}"
    );
}
