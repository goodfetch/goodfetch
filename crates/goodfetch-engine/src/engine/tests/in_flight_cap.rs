//! A Connection at its cap of calls in flight, through the engine's public
//! interface over the in-memory network (ADR 0038): only a new call is
//! refused, with `too_many_calls`; a `$/cancel`, a reply to a handler
//! request and the end of the Client's input are read and acted on as
//! below the cap.

use goodfetch_protocol::catalog::{
    Operation, OperationClass, OperationKind, PROTOCOL_VERSION, Product, Reach, Stability,
    schema_of,
};
use serde_json::{Value, json};

use super::trace::asking;
use super::{Client, IN_FLIGHT_CAP, attached, engine_with, opened, owed, request};
use crate::call_error::CallError;
use crate::composition::{CallContext, Contribution, Handler};

/// A streaming call that answers once its Client granted it a credit.
const CREDITED: Operation = Operation {
    name: "test.credited",
    description: "Answers once it was granted a credit.",
    class: OperationClass::Read,
    kind: OperationKind::RoundTrip,
    reach: Reach::AllSurfaces,
    params: schema_of::<Value>,
    result: schema_of::<Value>,
    events: &[],
    cancellable: true,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: PROTOCOL_VERSION,
    input: None,
    bound: None,
};

/// `test.ask`, whose handler request shows a test that its call started,
/// and `test.credited`.
fn operations() -> Contribution {
    asking().with_handler(
        Handler::new(&CREDITED, |_: Value, context: CallContext| async move {
            let credits = context.stream().spend(1).await;
            Ok::<Value, CallError>(json!({ "credits": credits }))
        })
        .streaming(),
    )
}

/// `test.wait` calls under each id of `ids`: each waits for its `$/cancel`.
fn waits(ids: std::ops::RangeInclusive<i64>) -> Vec<Value> {
    ids.map(|id| request(id, "test.wait")).collect()
}

/// Puts as many calls in flight as the cap, each ending only on a message
/// of its Client: `test.wait` calls after `first`, and a last `test.ask`
/// whose handler request, which it answers, shows that the engine started
/// them all.
async fn in_flight_at_the_cap_after(client: &mut Client, first: &[Value]) -> Value {
    let taken = i64::try_from(first.len()).unwrap();
    client.send(first).await;
    client.send(&waits(taken + 1..=IN_FLIGHT_CAP - 1)).await;
    client.send(&[request(IN_FLIGHT_CAP, "test.ask")]).await;
    let asked = owed(client).await.expect("the engine asks");
    assert_eq!(asked["method"], json!("test.asked"), "{asked}");
    asked
}

/// [`in_flight_at_the_cap_after`], with `test.wait` calls only before the
/// last.
async fn in_flight_at_the_cap(client: &mut Client) -> Value {
    in_flight_at_the_cap_after(client, &[]).await
}

/// With as many calls in flight as the cap, each waiting for its `$/cancel`,
/// the `$/cancel` of one is read and its call answers `call_cancelled`.
///
/// Covers: error:call_cancelled
#[test]
fn a_cancel_is_honoured_with_as_many_calls_in_flight_as_the_cap() {
    let engine = engine_with(Product::Collie, Some(operations()));
    let answered = attached(&engine, |mut client: Client| async move {
        opened(&mut client).await;
        in_flight_at_the_cap(&mut client).await;
        client
            .send(&[json!({ "jsonrpc": "2.0", "method": "$/cancel", "params": { "id": 7 } })])
            .await;
        let answered = owed(&mut client).await.expect("the cancelled call answers");
        client.close().await;
        answered
    });

    assert_eq!(
        (&answered["id"], &answered["error"]["data"]["code"]),
        (&json!(7), &json!("call_cancelled")),
        "{answered}"
    );
}

/// At the cap, the `$/credit` of a streaming call is read: the call it
/// names spends it and answers.
#[test]
fn a_credit_is_granted_with_as_many_calls_in_flight_as_the_cap() {
    let engine = engine_with(Product::Collie, Some(operations()));
    let answered = attached(&engine, |mut client: Client| async move {
        opened(&mut client).await;
        in_flight_at_the_cap_after(&mut client, &[request(1, "test.credited")]).await;
        client
            .send(&[json!({ "jsonrpc": "2.0", "method": "$/credit", "params": { "id": 1, "credits": 1 } })])
            .await;
        let answered = owed(&mut client).await.expect("the credited call answers");
        client.close().await;
        answered
    });

    assert_eq!(
        (&answered["id"], &answered["result"]),
        (&json!(1), &json!({ "credits": 1 })),
        "{answered}"
    );
}

/// A call no Product carries is refused as below the cap, with
/// `method_not_found`: only a call the engine would start is refused for
/// the cap.
#[test]
fn a_call_the_engine_would_not_start_is_refused_for_its_own_reason_at_the_cap() {
    let engine = engine_with(Product::Collie, Some(operations()));
    let refused = attached(&engine, |mut client: Client| async move {
        opened(&mut client).await;
        in_flight_at_the_cap(&mut client).await;
        client
            .send(&[request(IN_FLIGHT_CAP + 1, "test.unknown")])
            .await;
        let refused = owed(&mut client).await.expect("the call is refused");
        client.close().await;
        refused
    });

    assert_eq!(
        (&refused["id"], &refused["error"]["code"]),
        (&json!(IN_FLIGHT_CAP + 1), &json!(-32601)),
        "{refused}"
    );
}

/// A call whose `_meta` the engine refuses is refused for it at the cap as
/// below it, with `invalid_params`: it is no call the engine would start.
#[test]
fn a_call_whose_meta_is_refused_keeps_that_refusal_at_the_cap() {
    let engine = engine_with(Product::Collie, Some(operations()));
    let refused = attached(&engine, |mut client: Client| async move {
        opened(&mut client).await;
        in_flight_at_the_cap(&mut client).await;
        let mut call = request(IN_FLIGHT_CAP + 1, "test.wait");
        call["params"]["_meta"] = json!({ "baggage": "user=a" });
        client.send(&[call]).await;
        let refused = owed(&mut client).await.expect("the call is refused");
        client.close().await;
        refused
    });

    assert_eq!(
        (&refused["id"], &refused["error"]["data"]["code"]),
        (&json!(IN_FLIGHT_CAP + 1), &json!("invalid_params")),
        "{refused}"
    );
}

/// At the cap, one more call is refused at once with `too_many_calls`,
/// retryable, naming the cap; the reply to a handler request still reaches
/// the call that waits for it.
///
/// Covers: error:too_many_calls
#[test]
fn a_call_past_the_cap_is_refused_at_once_and_a_reply_still_reaches_its_waiter() {
    let engine = engine_with(Product::Collie, Some(operations()));
    let (refused, answered) = attached(&engine, |mut client: Client| async move {
        opened(&mut client).await;
        let asked = in_flight_at_the_cap(&mut client).await;
        client
            .send(&[request(IN_FLIGHT_CAP + 1, "test.wait")])
            .await;
        let refused = owed(&mut client).await.expect("the call is refused");
        client
            .send(&[json!({ "jsonrpc": "2.0", "id": asked["id"], "result": { "because": 1 } })])
            .await;
        let answered = owed(&mut client).await.expect("the asking call answers");
        client.close().await;
        (refused, answered)
    });

    assert_eq!(
        (
            &refused["id"],
            &refused["error"]["data"]["code"],
            &refused["error"]["data"]["retryable"],
            &refused["error"]["data"]["max"],
        ),
        (
            &json!(IN_FLIGHT_CAP + 1),
            &json!("too_many_calls"),
            &json!(true),
            &json!(IN_FLIGHT_CAP),
        ),
        "{refused}"
    );
    assert_eq!(
        (&answered["id"], &answered["result"]),
        (&json!(IN_FLIGHT_CAP), &json!({ "because": 1 })),
        "{answered}"
    );
}

/// At the cap, the end of the Client's input ends the Connection and
/// cancels its calls, as below the cap.
#[test]
fn the_end_of_the_clients_input_at_the_cap_cancels_its_calls_and_ends_the_connection() {
    let engine = engine_with(Product::Collie, Some(operations()));
    let (cancelled, others) = attached(&engine, |mut client: Client| async move {
        opened(&mut client).await;
        in_flight_at_the_cap(&mut client).await;
        client.close().await;
        let (mut cancelled, mut others) = (0, Vec::new());
        while let Some(message) = owed(&mut client).await {
            if message["error"]["data"]["code"] == json!("call_cancelled") {
                cancelled += 1;
            } else {
                others.push(message);
            }
        }
        (cancelled, others)
    });

    assert_eq!((cancelled, others), (IN_FLIGHT_CAP, Vec::<Value>::new()));
}
