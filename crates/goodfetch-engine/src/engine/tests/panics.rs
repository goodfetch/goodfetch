//! A call whose handler panics, through the engine's public interface over
//! the in-memory network (ADR 0148): the call is answered
//! `engine_panicked`, naming what ended and the crash report, never the
//! panic's own text; its place in flight is given back, its Connection
//! goes on, and the ops endpoint counts the panic. The panic comes from an
//! extra operation the test hands the engine, never from a switch in a
//! Product.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use goodfetch_protocol::catalog::{
    Operation, OperationClass, OperationKind, PROTOCOL_VERSION, Product, Reach, Stability,
    schema_of,
};
use goodfetch_test_support::TestEnv;
use serde_json::{Value, json};

use super::ops::{metering, series};
use super::{
    Client, IN_FLIGHT_CAP, attached_with, dependencies, engine_with, opened, owed, request,
};
use crate::call_error::CallError;
use crate::composition::{CallContext, Contribution, Handler};
use crate::crash_report::record_panics;
use crate::mask::Masks;
use crate::tasks::BlockingPolicy;

/// What the panicking handler says, which only the crash report may hold.
const PANIC_TEXT: &str = "the widget count went negative";

/// A call whose handler panics.
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

/// A call whose handler's blocking work panics.
const PANIC_IN_BLOCKING_WORK: Operation = Operation {
    name: "test.panic_in_blocking_work",
    description: "Panics in its blocking work.",
    ..PANIC
};

/// The work of a call that panics.
fn panics() -> Result<Value, CallError> {
    panic!("{PANIC_TEXT}")
}

fn panicking() -> Contribution {
    Contribution::new("test_panic")
        .with_handler(Handler::new(
            &PANIC,
            |_: Value, _: CallContext| async move { panics() },
        ))
        .with_handler(Handler::new(
            &PANIC_IN_BLOCKING_WORK,
            |_: Value, context: CallContext| async move {
                let owner = Arc::clone(context.engine().owner());
                owner.blocking(panics).await
            },
        ))
}

/// The reply to the call `id`, read past every other message.
async fn reply(client: &mut Client, id: i64) -> Value {
    loop {
        let message = owed(client).await.expect("the Connection is open");
        if message["id"] == json!(id) {
            return message;
        }
    }
}

/// What `talk` answers on an Attached engine that carries `test.panic` and
/// runs blocking work on threads, as a Product does, the crash reports it
/// left, each file's text by its name, and every series of the engine's
/// meter once `talk` ended.
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
            let mut dependencies = dependencies(&env);
            dependencies.blocking = BlockingPolicy::Threads;
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
        opened(&mut client).await;
        client.send(&[request(1, "test.panic")]).await;
        let answered = reply(&mut client, 1).await;
        client.send(&[request(2, "test.surface")]).await;
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
        opened(&mut client).await;
        let calls: Vec<Value> = (1..=IN_FLIGHT_CAP)
            .map(|id| request(id, "test.panic"))
            .collect();
        client.send(&calls).await;
        let mut panicked = 0;
        for _ in 1..=IN_FLIGHT_CAP {
            let answered = owed(&mut client).await.expect("the call answers");
            if answered["error"]["data"]["code"] == json!("engine_panicked") {
                panicked += 1;
            }
        }
        client
            .send(&[request(IN_FLIGHT_CAP + 1, "test.surface")])
            .await;
        let served = reply(&mut client, IN_FLIGHT_CAP + 1).await;
        client.close().await;
        (panicked, served)
    });

    assert_eq!(
        (panicked, served["result"]["surface"].as_str()),
        (IN_FLIGHT_CAP, Some("sdk_typescript")),
        "{served}"
    );
}

/// A panic in a call's blocking work, which runs on a thread of its own
/// and resumes in the call, is answered `engine_panicked` with `ended:
/// call` and the name of the crash report that panic wrote.
#[test]
fn a_panic_in_a_calls_blocking_work_is_answered_with_its_own_crash_report() {
    let (answered, reports, _) = panicked(|mut client: Client| async move {
        opened(&mut client).await;
        client
            .send(&[request(1, "test.panic_in_blocking_work")])
            .await;
        let answered = reply(&mut client, 1).await;
        client.close().await;
        answered
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
            &data["ended"],
            report_text.contains(PANIC_TEXT),
        ),
        (&json!("engine_panicked"), &json!("call"), true),
        "{answered} {reports:?}"
    );
}
