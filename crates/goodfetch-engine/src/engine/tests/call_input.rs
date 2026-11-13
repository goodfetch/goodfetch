//! A call's input through the engine's public interface over the in-memory
//! network (ADR 0024; STYLE.md, Memory and hot paths): a Client that sends
//! a call more input notifications than the call left unread room for is
//! refused with `invalid_params`, and the engine never queues more of them
//! than that room, whatever the Client sends.

use std::sync::Arc;

use goodfetch_protocol::catalog::{
    Operation, OperationClass, OperationKind, Product, Reach, Stability, schema_of,
};
use serde_json::{Value, json};
use tokio::sync::Notify;

use super::{Client, attached, engine_with, hello};
use crate::call_error::CallError;
use crate::composition::{CallContext, Contribution, Handler};

/// The notification `test.input`'s calls take as their input.
const TEST_INPUT: &str = "$/test_input";

/// How many of its input notifications a `test.input` call leaves unread
/// at most.
const UNREAD: usize = 2;

/// Waits for `test.release`, then reads as many of its input notifications
/// as its params' `read` says, and answers the `number` of each it read.
const INPUT: Operation = Operation {
    name: "test.input",
    description: "Reads its input once released.",
    class: OperationClass::Read,
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

/// Lets the waiting `test.input` call read its input.
const RELEASE: Operation = Operation {
    name: "test.release",
    description: "Lets `test.input` read.",
    ..INPUT
};

fn inputs(released: &Arc<Notify>) -> Contribution {
    let (waiting, releasing) = (Arc::clone(released), Arc::clone(released));
    Contribution::new("test-input")
        .with_handler(
            Handler::new(&INPUT, move |params: Value, context: CallContext| {
                let waiting = Arc::clone(&waiting);
                async move {
                    waiting.notified().await;
                    let wanted = usize::try_from(params["read"].as_u64().unwrap()).unwrap();
                    let mut read = Vec::new();
                    while read.len() < wanted {
                        let Some(input) = context.input().next().await? else {
                            break;
                        };
                        read.push(input.read::<Value>().unwrap()["number"].clone());
                    }
                    Ok::<Value, CallError>(json!({ "read": read }))
                }
            })
            .taking_input(TEST_INPUT, UNREAD),
        )
        .with_handler(Handler::new(&RELEASE, move |_: Value, _: CallContext| {
            let releasing = Arc::clone(&releasing);
            async move {
                releasing.notify_one();
                Ok::<Value, CallError>(json!({ "released": true }))
            }
        }))
}

fn call(id: i64, method: &str, params: &Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params })
}

fn input(id: i64, number: usize) -> Value {
    json!({ "jsonrpc": "2.0", "method": TEST_INPUT, "params": { "id": id, "number": number } })
}

/// The reply to the call `id`, read past every other message.
async fn reply(client: &mut Client, id: i64) -> Value {
    loop {
        let message = client.next().await.expect("the engine replies");
        if message["id"] == json!(id) {
            return message;
        }
    }
}

/// What the call `test.input` answers when its Client sends it `sent` input
/// notifications before it reads any, then lets it read as many.
fn answered_after(sent: usize) -> Value {
    let released = Arc::new(Notify::new());
    let engine = engine_with(Product::Collie, Some(inputs(&released)));
    attached(&engine, |mut client: Client| async move {
        client.send(&[hello("collie", "0.1")]).await;
        assert!(client.next().await.unwrap().get("result").is_some());
        client
            .send(&[call(1, "test.input", &json!({ "read": sent }))])
            .await;
        let notifications: Vec<Value> = (0..sent).map(|number| input(1, number)).collect();
        client.send(&notifications).await;
        client.send(&[call(2, "test.release", &json!({}))]).await;
        assert_eq!(
            reply(&mut client, 2).await["result"],
            json!({ "released": true })
        );
        let answered = reply(&mut client, 1).await;
        client.close().await;
        answered
    })
}

/// Input that fits the room the call left is read whole, in order.
#[test]
fn input_within_the_unread_room_is_read() {
    let answered = answered_after(UNREAD);

    assert_eq!(answered["result"], json!({ "read": [0, 1] }), "{answered}");
}

/// One input notification past the room the call left fails the call with
/// `invalid_params`, before the call reads anything past that room.
#[test]
fn input_past_the_unread_room_fails_the_call_with_invalid_params() {
    let answered = answered_after(UNREAD + 1);

    assert_eq!(
        (
            &answered["error"]["code"],
            answered["error"]["message"].is_string()
        ),
        (&json!(-32602), true),
        "{answered}"
    );
}
