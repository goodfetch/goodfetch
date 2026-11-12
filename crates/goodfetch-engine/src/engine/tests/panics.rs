//! A call whose handler panics, through the engine's public interface over
//! the in-memory network (ADR 0148): the call is answered
//! `engine_panicked`, naming what ended and the crash report, never the
//! panic's own text; its place in flight is given back, its Connection
//! goes on, and the ops endpoint counts the panic. The panic comes from an
//! extra operation the test hands the engine, never from a switch in a
//! Product.

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use goodfetch_protocol::catalog::{
    Operation, OperationClass, OperationKind, PROTOCOL_VERSION, Product, Reach, Stability,
    schema_of,
};
use goodfetch_test_support::TestEnv;
use serde_json::{Value, json};

use super::ops::{metering, series};
use super::{Client, attached_with, dependencies, engine_with, hello};
use crate::call_error::CallError;
use crate::composition::{CallContext, Contribution, Handler};
use crate::crash_report::record_panics;
use crate::mask::Masks;

/// What the panicking handler says, which only the crash report may hold.
const PANIC_TEXT: &str = "the widget count went negative";

/// How many calls a Connection keeps in flight.
const CAP: i64 = 256;

/// A call whose handler panics unless its params say `fine`.
const PANIC: Operation = Operation {
    name: "test.panic",
    description: "Panics.",
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
};

fn panicking() -> Contribution {
    Contribution::new("test_panic").with_handler(Handler::new(
        &PANIC,
        |params: Value, _: CallContext| async move {
            assert!(params["fine"] == json!(true), "{PANIC_TEXT}");
            Ok::<Value, CallError>(Value::Null)
        },
    ))
}

fn call(id: i64, method: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": {} })
}

/// How long a test waits, on the paused clock, for a message the engine
/// owes: one it never sends fails the test instead of hanging it.
const OWED: Duration = Duration::from_secs(5);

/// The next message the engine owes.
async fn owed(client: &mut Client) -> Value {
    tokio::time::timeout(OWED, client.next())
        .await
        .expect("the engine sends what it owes")
        .expect("the Connection is open")
}

/// The reply to the call `id`, read past every other message.
async fn reply(client: &mut Client, id: i64) -> Value {
    loop {
        let message = owed(client).await;
        if message["id"] == json!(id) {
            return message;
        }
    }
}

/// What `talk` answers on an Attached engine that carries `test.panic`, the
/// crash reports it left, each file's text by its name, and every series of
/// the engine's meter once `talk` ended.
fn panicked<F, Fut, T>(talk: F) -> (T, BTreeMap<String, String>, BTreeSet<String>)
where
    F: FnOnce(Client) -> Fut,
    Fut: Future<Output = T>,
{
    let env = TestEnv::new();
    let folder = env.home().join("crashes");
    let (metrics, registry, _provider) = metering();
    let engine = engine_with(Product::Collie, Some(panicking())).with_metrics(metrics);
    let writing = folder.clone();
    let answered = attached_with(
        &engine,
        || {
            let dependencies = dependencies(&env);
            record_panics(
                writing,
                "goodfetch-collie 0.1.0".to_owned(),
                dependencies.clock,
                Masks::new(),
            );
            dependencies
        },
        talk,
    );
    let _ = std::panic::take_hook();
    let reports = std::fs::read_dir(&folder)
        .into_iter()
        .flatten()
        .map(|entry| {
            let entry = entry.unwrap();
            (
                entry.file_name().into_string().unwrap(),
                std::fs::read_to_string(entry.path()).unwrap(),
            )
        })
        .collect();
    (answered, reports, series(&registry))
}

/// A call whose handler panics is answered `engine_panicked`, not
/// retryable, with `ended: call` and the name of a crash report that
/// exists; the panic's own text is in that report and nowhere in the
/// answer. The next call on the Connection is answered as ever, and the
/// ops endpoint counts one panic that ended a call.
///
/// Covers: error:engine_panicked
#[test]
fn a_call_whose_handler_panics_is_answered_engine_panicked_and_its_connection_goes_on() {
    let ((answered, next), reports, series) = panicked(|mut client: Client| async move {
        client.send(&[hello("collie", "0.1")]).await;
        assert!(client.next().await.unwrap().get("result").is_some());
        client.send(&[call(1, "test.panic")]).await;
        let answered = reply(&mut client, 1).await;
        client.send(&[call(2, "test.surface")]).await;
        let next = reply(&mut client, 2).await;
        client.close().await;
        (answered, next)
    });

    let data = &answered["error"]["data"];
    let report_text = data["crash_report"]
        .as_str()
        .and_then(|name| reports.get(name))
        .cloned()
        .unwrap_or_default();
    assert_eq!(
        (
            &data["code"],
            &data["retryable"],
            &data["ended"],
            report_text.contains(PANIC_TEXT),
            answered.to_string().contains(PANIC_TEXT),
            next["result"]["surface"].as_str(),
            series.contains(r#"goodfetch_engine_panics_total{ended="call"} 1"#),
        ),
        (
            &json!("engine_panicked"),
            &json!(false),
            &json!("call"),
            true,
            false,
            Some("sdk_typescript"),
            true,
        ),
        "{answered} {next} {series:?}"
    );
}

/// Each panicked call gave its place in flight back: after as many of
/// them as a Connection keeps in flight, one more call is served.
#[test]
fn a_panicked_call_gives_its_place_in_flight_back() {
    let ((panicked, served), _, _) = panicked(|mut client: Client| async move {
        client.send(&[hello("collie", "0.1")]).await;
        assert!(client.next().await.unwrap().get("result").is_some());
        let calls: Vec<Value> = (1..=CAP).map(|id| call(id, "test.panic")).collect();
        client.send(&calls).await;
        let mut panicked = 0;
        for _ in 1..=CAP {
            let answered = owed(&mut client).await;
            if answered["error"]["data"]["code"] == json!("engine_panicked") {
                panicked += 1;
            }
        }
        client.send(&[call(CAP + 1, "test.surface")]).await;
        let served = reply(&mut client, CAP + 1).await;
        client.close().await;
        (panicked, served)
    });

    assert_eq!(
        (panicked, served["result"]["surface"].as_str()),
        (CAP, Some("sdk_typescript")),
        "{served}"
    );
}
