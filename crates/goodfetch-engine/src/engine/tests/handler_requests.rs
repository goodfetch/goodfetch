//! The handler requests the engine sends its Client, through the engine's
//! public interface over the in-memory network (ADR 0038): one nobody waits
//! for any more, its call cancelled or its sender gone, leaves the
//! Connection's pending requests at once, so a late reply to it is ignored
//! and the Connection goes on, still answering a request that waits.

use goodfetch_protocol::catalog::{
    HandlerRequest, Operation, OperationClass, OperationKind, PROTOCOL_VERSION, Product, Reach,
    Stability, schema_of,
};
use rstest::rstest;
use serde_json::{Value, json};

use super::trace::asking;
use super::{Client, attached, cancel, engine_with, opened, owed, records_of, request};
use crate::call_error::CallError;
use crate::composition::{CallContext, Contribution, Handler};

/// Where the Connection logs what it does with a reply.
const CONNECTION_TARGET: &str = "goodfetch_engine::rpc::connection";

/// The handler request `test.give_up` sends its Client.
const GIVEN_UP: HandlerRequest = HandlerRequest {
    name: "test.given_up",
    description: "Asks the Client a question whose answer nobody reads.",
    params: schema_of::<Value>,
    result: schema_of::<Value>,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: PROTOCOL_VERSION,
};

/// A call that sends its Client [`GIVEN_UP`], cancels it and answers
/// without waiting for its reply, as an attempt whose lease ended does.
const GIVE_UP: Operation = Operation {
    name: "test.give_up",
    description: "Sends a handler request, cancels it and stops waiting.",
    class: OperationClass::Read,
    kind: OperationKind::RoundTrip,
    reach: Reach::AllSurfaces,
    params: schema_of::<Value>,
    result: schema_of::<Value>,
    events: &[],
    cancellable: true,
    input: None,
    bound: None,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: PROTOCOL_VERSION,
};

/// `test.ask`, which waits for its handler request's reply, and
/// `test.give_up`.
fn operations() -> Contribution {
    asking().with_handler(Handler::new(
        &GIVE_UP,
        |_: Value, context: CallContext| async move {
            let sent = context
                .client()
                .send::<Value>(&GIVEN_UP, &json!({}))
                .await
                .expect("the Connection is open");
            sent.cancel().await;
            Ok::<Value, CallError>(json!("gave up"))
        },
    ))
}

/// The next message the engine owes that `wanted` takes, past the others.
async fn owed_until(client: &mut Client, wanted: impl Fn(&Value) -> bool) -> Value {
    loop {
        let message = owed(client).await.expect("the engine owes more");
        if wanted(&message) {
            return message;
        }
    }
}

/// How the call that sent the handler request stops waiting for its reply.
#[derive(Clone, Copy, Debug)]
enum StopsWaiting {
    /// Its Client cancels the call, which drops the handler's wait.
    ItsCallIsCancelled,
    /// Its handler cancels the request and answers without its reply.
    ItsHandlerGivesUp,
}

/// Calls under the id 2 a call that sends its Client a handler request and
/// stops waiting for its reply as `stops` says; answers the handler request
/// once the call answered.
async fn stopped_waiting(client: &mut Client, stops: StopsWaiting) -> Value {
    match stops {
        StopsWaiting::ItsCallIsCancelled => {
            client.send(&[request(2, "test.ask")]).await;
            let asked =
                owed_until(client, |message| message["method"] == json!("test.asked")).await;
            client.send(&[cancel(2)]).await;
            owed_until(client, |message| message["id"] == json!(2)).await;
            asked
        }
        StopsWaiting::ItsHandlerGivesUp => {
            client.send(&[request(2, "test.give_up")]).await;
            // The call's answer may come before its handler request.
            let (mut asked, mut answered) = (None, false);
            while asked.is_none() || !answered {
                let message = owed(client).await.expect("the engine owes more");
                if message["method"] == json!(GIVEN_UP.name) {
                    asked = Some(message);
                } else if message["id"] == json!(2) {
                    answered = true;
                }
            }
            asked.expect("the engine asked")
        }
    }
}

/// A handler request nobody waits for leaves the Connection's pending
/// requests before its Client replies: the late reply is logged as one that
/// answers no handler request and ignored, and the next request's reply,
/// which its call waits for, is delivered.
#[rstest]
#[case::its_call_is_cancelled(StopsWaiting::ItsCallIsCancelled)]
#[case::its_handler_gives_up(StopsWaiting::ItsHandlerGivesUp)]
fn a_late_reply_to_a_handler_request_nobody_waits_for_is_ignored(#[case] stops: StopsWaiting) {
    let engine = engine_with(Product::Collie, Some(operations()));

    let (records, answered) = {
        let mut answered = None;
        let records = records_of(CONNECTION_TARGET, || {
            answered = Some(attached(&engine, |mut client: Client| async move {
                opened(&mut client).await;
                let asked = stopped_waiting(&mut client, stops).await;
                client
                    .send(&[json!({ "jsonrpc": "2.0", "id": asked["id"], "result": "late" })])
                    .await;

                client.send(&[request(3, "test.ask")]).await;
                let waited = owed_until(&mut client, |message| {
                    message["method"] == json!("test.asked")
                })
                .await;
                client
                    .send(&[json!({ "jsonrpc": "2.0", "id": waited["id"], "result": "in time" })])
                    .await;
                let answered = owed_until(&mut client, |message| message["id"] == json!(3)).await;
                client.close().await;
                answered
            }));
        });
        (records, answered.expect("the Client ran"))
    };

    let ignored: Vec<&String> = records
        .iter()
        .filter(|record| record.contains("answers no handler request"))
        .collect();
    assert_eq!(
        (ignored.len(), &answered["result"]),
        (1, &json!("in time")),
        "{records:#?}\n{answered}"
    );
}
