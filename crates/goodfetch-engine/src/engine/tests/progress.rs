//! A Run's progress message, through the engine's public interface over the
//! in-memory network (spec #213's Seam 3; ADR 0037's RT-15, ADR 0102):
//! `run.set_progress_message` keeps the program's own status line, at most
//! 512 characters, in the Run's state, which a resume keeps, carries it on
//! `run.progress` and in the Run's Run index row, and refuses a Run this
//! engine does not run. What the tests read of the run file and the Run
//! index is what the engine wrote.

use goodfetch_protocol::catalog::Product;
use goodfetch_test_support::TestEnv;
use rstest::rstest;
use rusqlite::Connection;
use serde_json::{Value, json};

use super::run::call;
use super::{Client, attached_on, engine, hello};

/// The progress message `query` reads of `run_id` in the file at `path`.
fn kept(path: &std::path::Path, query: &str, run_id: &str) -> Option<String> {
    Connection::open(path)
        .unwrap()
        .query_row(query, [run_id], |row| row.get(0))
        .unwrap()
}

fn in_run_state(env: &TestEnv, run_id: &str) -> Option<String> {
    kept(
        &env.home()
            .join("1")
            .join("runs")
            .join(run_id)
            .join("run.sqlite"),
        "SELECT progress_message FROM run WHERE run_id = ?1",
        run_id,
    )
}

fn in_run_index(env: &TestEnv, run_id: &str) -> Option<String> {
    kept(
        &env.home().join("1").join("run-index.sqlite"),
        "SELECT progress_message FROM runs WHERE run_id = ?1",
        run_id,
    )
}

/// Opens the Connection with `hello` and a subscription, call 1, granted
/// ample credits.
async fn subscribed(client: &mut Client) {
    client.send(&[hello("collie", "0.1")]).await;
    client.next().await.unwrap();
    client
        .send(&[
            json!({ "jsonrpc": "2.0", "id": 1, "method": "events.subscribe", "params": {} }),
            json!({ "jsonrpc": "2.0", "method": "$/credit", "params": { "id": 1, "credits": 100 } }),
        ])
        .await;
}

/// Reads until the reply to the call `id` and a `run.progress` of the
/// subscription, call 1, both came, and answers them.
async fn reply_and_progress(client: &mut Client, id: i64) -> (Value, Value) {
    let (mut reply, mut progress) = (None, None);
    while reply.is_none() || progress.is_none() {
        let message = client.next().await.expect("the engine closed");
        if message["id"] == json!(id) {
            reply = Some(message);
        } else if message["method"] == "$/events" {
            let events = message["params"]["events"].as_array().unwrap();
            if let Some(event) = events.iter().find(|event| event["event"] == "run.progress") {
                progress = Some(event["data"].clone());
            }
        }
    }
    (reply.unwrap(), progress.unwrap())
}

/// Covers: story:3-129, operation:run.set_progress_message, event:run.progress
#[test]
fn a_progress_message_is_kept_in_run_state_carried_on_progress_and_in_the_run_index_row() {
    let env = TestEnv::new();

    let (answer, event) = attached_on(&engine(Product::Collie), &env, |mut client| async move {
        subscribed(&mut client).await;
        call(&mut client, 2, "run.start", json!({ "run_id": "noted" })).await;
        client
            .send(&[json!({
                "jsonrpc": "2.0", "id": 3, "method": "run.set_progress_message",
                "params": { "run_id": "noted", "message": "Reading page 3 of 10" },
            })])
            .await;
        reply_and_progress(&mut client, 3).await
    });

    let progress = json!({
        "run_id": "noted",
        "position": 4_294_967_297_u64,
        "at": "2026-01-05T09:00:00Z",
        "lifecycle": "running",
        "writer": "live",
        "epoch": 1,
        "pauses": [],
        "message": "Reading page 3 of 10",
        "last": false,
    });
    assert_eq!(
        (
            &answer["result"],
            &event,
            in_run_state(&env, "noted"),
            in_run_index(&env, "noted"),
        ),
        (
            &progress,
            &progress,
            Some("Reading page 3 of 10".to_owned()),
            Some("Reading page 3 of 10".to_owned()),
        )
    );
}

/// Covers: story:3-129
#[rstest]
#[case::at_most_512_characters(512, true, None)]
#[case::longer(513, false, Some("invalid_params"))]
fn a_progress_message_holds_at_most_512_characters(
    #[case] length: usize,
    #[case] kept: bool,
    #[case] refused: Option<&str>,
) {
    let env = TestEnv::new();
    let message = "é".repeat(length);

    let answer = attached_on(&engine(Product::Collie), &env, |mut client| {
        let message = message.clone();
        async move {
            client.send(&[hello("collie", "0.1")]).await;
            client.next().await.unwrap();
            call(&mut client, 1, "run.start", json!({ "run_id": "long" })).await;
            call(
                &mut client,
                2,
                "run.set_progress_message",
                json!({ "run_id": "long", "message": message }),
            )
            .await
        }
    });

    assert_eq!(
        (
            answer.get("result").is_some(),
            answer["error"]["data"]["code"].as_str(),
            in_run_state(&env, "long").is_some()
        ),
        (kept, refused, kept),
        "{answer}"
    );
}

/// Covers: story:3-129, error:run_not_found
#[test]
fn a_progress_message_of_a_run_this_engine_does_not_run_fails_with_run_not_found() {
    let answer = attached_on(
        &engine(Product::Collie),
        &TestEnv::new(),
        |mut client| async move {
            client.send(&[hello("collie", "0.1")]).await;
            client.next().await.unwrap();
            call(
                &mut client,
                1,
                "run.set_progress_message",
                json!({ "run_id": "absent", "message": "Waiting" }),
            )
            .await
        },
    );

    assert_eq!(answer["error"]["data"]["code"], "run_not_found");
}

/// A pause stops the Run; the resume reads its progress message back from
/// its state, so its Run index row, written again, still shows it.
///
/// Covers: story:3-129
#[test]
fn a_resumed_run_keeps_its_progress_message() {
    let env = TestEnv::new();

    attached_on(&engine(Product::Collie), &env, |mut client| async move {
        client.send(&[hello("collie", "0.1")]).await;
        client.next().await.unwrap();
        call(&mut client, 1, "run.start", json!({ "run_id": "kept" })).await;
        call(
            &mut client,
            2,
            "run.set_progress_message",
            json!({ "run_id": "kept", "message": "Half way" }),
        )
        .await;
        call(&mut client, 3, "run.pause", json!({ "run_id": "kept" })).await;
        call(&mut client, 4, "run.resume", json!({ "run_id": "kept" })).await;
    });

    assert_eq!(
        (in_run_state(&env, "kept"), in_run_index(&env, "kept")),
        (Some("Half way".to_owned()), Some("Half way".to_owned()))
    );
}
