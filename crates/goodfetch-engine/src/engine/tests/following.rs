//! Following a Run's progress, through the engine's public interface over
//! the in-memory network (spec #213's Seam 3; ADR 0102): `run.progress`
//! answers the Run's progress at once, then, given the position of the
//! value a reader last read, the latest value after a change, coalescing
//! the changes in between, until the last value, which says how the Run
//! ended.

use goodfetch_protocol::catalog::Product;
use serde_json::{Value, json};

use super::run::call;
use super::{Client, attached, engine, hello};

/// What a test asserts of a progress value: all but its position and the
/// moment it was read, which only order values.
fn settled(progress: &Value) -> Value {
    let mut progress = progress.clone();
    let fields = progress
        .as_object_mut()
        .expect("a progress value is an object");
    fields.remove("position");
    fields.remove("at");
    progress
}

/// `run.progress` subscribes when called: its first value is the Run's
/// progress at once, with its writer, its run epoch, the time of its last
/// Checkpoint, its Human pauses and its program's message.
///
/// Covers: story:3-161, operation:run.progress
#[test]
fn the_first_progress_value_comes_at_once_with_the_writer_epoch_checkpoint_pauses_and_message() {
    let (checkpoint, progress) = attached(&engine(Product::Collie), |mut client| async move {
        client.send(&[hello("collie", "0.1")]).await;
        client.next().await.unwrap();
        call(&mut client, 1, "run.start", json!({ "run_id": "followed" })).await;
        call(
            &mut client,
            2,
            "run.set_progress_message",
            json!({ "run_id": "followed", "message": "Half way" }),
        )
        .await;
        let checkpoint = call(
            &mut client,
            3,
            "run.checkpoint",
            json!({ "run_id": "followed" }),
        )
        .await;
        let progress = call(
            &mut client,
            4,
            "run.progress",
            json!({ "run_id": "followed" }),
        )
        .await;
        (checkpoint["result"].clone(), progress["result"].clone())
    });

    assert_eq!(
        settled(&progress),
        json!({
            "run_id": "followed",
            "lifecycle": "running",
            "writer": "live",
            "epoch": 1,
            "checkpoint_at": checkpoint["time"],
            "pauses": [],
            "message": "Half way",
            "last": false,
        })
    );
}

/// Sets the progress message of `run_id` to each of `messages`, calls
/// `first_id` onwards.
async fn set_messages(client: &mut Client, first_id: i64, run_id: &str, messages: &[&str]) {
    for (id, message) in (first_id..).zip(messages) {
        call(
            client,
            id,
            "run.set_progress_message",
            json!({ "run_id": run_id, "message": message }),
        )
        .await;
    }
}

/// A reader that asks after the value it last read gets the latest one:
/// the changes since are coalesced into it.
///
/// Covers: story:3-161, story:3-8, operation:run.progress
#[test]
fn a_later_progress_value_is_the_latest_after_a_change_coalescing_those_between() {
    let (first, later) = attached(&engine(Product::Collie), |mut client| async move {
        client.send(&[hello("collie", "0.1")]).await;
        client.next().await.unwrap();
        call(&mut client, 1, "run.start", json!({ "run_id": "busy" })).await;
        let first = call(&mut client, 2, "run.progress", json!({ "run_id": "busy" })).await;
        set_messages(&mut client, 3, "busy", &["Page 1", "Page 2", "Page 3"]).await;
        let later = call(
            &mut client,
            6,
            "run.progress",
            json!({ "run_id": "busy", "after": first["result"]["position"] }),
        )
        .await;
        (first["result"].clone(), later["result"].clone())
    });

    assert_eq!(
        (
            later["message"].clone(),
            later["position"].as_u64().unwrap() - first["position"].as_u64().unwrap()
        ),
        (json!("Page 3"), 3)
    );
}

/// A reader waiting after the last value it read gets the Run's end: the
/// sequence's last value, which says how the Run ended.
///
/// Covers: story:3-161, operation:run.progress
#[test]
fn the_last_progress_value_says_how_the_run_ended() {
    let last = attached(&engine(Product::Collie), |mut client| async move {
        client.send(&[hello("collie", "0.1")]).await;
        client.next().await.unwrap();
        call(&mut client, 1, "run.start", json!({ "run_id": "ending" })).await;
        let first = call(
            &mut client,
            2,
            "run.progress",
            json!({ "run_id": "ending" }),
        )
        .await;
        client
            .send(&[
                json!({
                    "jsonrpc": "2.0", "id": 3, "method": "run.progress",
                    "params": { "run_id": "ending", "after": first["result"]["position"] },
                }),
                json!({
                    "jsonrpc": "2.0", "id": 4, "method": "run.stop",
                    "params": { "run_id": "ending" },
                }),
            ])
            .await;
        loop {
            let message = client.next().await.unwrap();
            if message["id"] == 3 {
                return message["result"].clone();
            }
        }
    });

    assert_eq!(
        settled(&last),
        json!({
            "run_id": "ending",
            "lifecycle": "stopped",
            "writer": "none",
            "epoch": 1,
            "pauses": [],
            "stop_reason": { "kind": "abort", "by": "program" },
            "last": true,
        })
    );
}

/// A value read from committed state once the Run ended comes after every
/// value a reader read while the engine worked it: positions only go up.
///
/// Covers: story:3-161, operation:run.progress
#[test]
fn a_value_read_after_the_run_ended_comes_after_those_read_while_it_ran() {
    let (live, ended) = attached(&engine(Product::Collie), |mut client| async move {
        client.send(&[hello("collie", "0.1")]).await;
        client.next().await.unwrap();
        call(&mut client, 1, "run.start", json!({ "run_id": "read" })).await;
        set_messages(&mut client, 2, "read", &["Page 1", "Page 2"]).await;
        let live = call(&mut client, 4, "run.progress", json!({ "run_id": "read" })).await;
        call(&mut client, 5, "run.finish", json!({ "run_id": "read" })).await;
        let ended = call(&mut client, 6, "run.progress", json!({ "run_id": "read" })).await;
        (live["result"].clone(), ended["result"].clone())
    });

    assert_eq!(
        (
            ended["position"].as_u64() > live["position"].as_u64(),
            &ended["lifecycle"],
            &ended["last"],
        ),
        (true, &json!("finished"), &json!(true))
    );
}
