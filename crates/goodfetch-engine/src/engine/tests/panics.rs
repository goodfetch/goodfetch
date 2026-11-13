//! A panic, through the engine's public interface over the in-memory
//! network (ADR 0148). A call whose handler panics is answered
//! `engine_panicked`, naming what ended and the crash report, never the
//! panic's own text; its place in flight is given back, its Connection
//! goes on, and the ops endpoint counts the panic. A Connection whose own
//! task panics ends alone, as when its Client goes away, and an Attached
//! engine ends with it. The panic comes from an extra operation the test
//! hands the engine, or a task it starts on a Connection's owner, never
//! from a switch in a Product.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Duration;

use goodfetch_protocol::catalog::{
    Operation, OperationClass, OperationKind, PROTOCOL_VERSION, Product, Reach, Stability,
    schema_of,
};
use goodfetch_test_support::TestEnv;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt as _, BufReader};
use tokio::sync::oneshot;

use super::daemon::{daemon, opened as daemon_opened, sdk_hello, the_pick};
use super::ops::{metering, series};
use super::run::{call, index_row};
use super::{
    Client, IN_FLIGHT_CAP, OWED, attached_on, attached_with, dependencies, engine_with, opened,
    owed, request,
};
use crate::call_error::CallError;
use crate::composition::{CallContext, Contribution, Handler};
use crate::crash_report::record_panics;
use crate::mask::Masks;
use crate::tasks::{BlockingPolicy, OwnerKind, TaskOwner};
use crate::transport::{self, Connection, Network};

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
    input: None,
    bound: None,
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

/// What a Connection sends until it closes, and whether it closed within
/// [`OWED`] of the paused clock.
async fn until_closed(client: &mut Client) -> (Vec<Value>, bool) {
    let mut sent = Vec::new();
    let closed = tokio::time::timeout(OWED, async {
        while let Some(message) = client.next().await {
            sent.push(message);
        }
    })
    .await
    .is_ok();
    (sent, closed)
}

/// Whether `sent` holds the answer `call_cancelled` to the call `id`.
fn cancelled(sent: &[Value], id: i64) -> bool {
    sent.iter().any(|message| {
        message["id"] == json!(id) && message["error"]["data"]["code"] == json!("call_cancelled")
    })
}

/// A Connection one of whose own tasks panics ends as when its Client goes
/// away, alone: its call in flight is answered `call_cancelled`, it closes,
/// and serving it returns, after which its caller stops its Runs as for
/// any Connection that ended (the Daemon's test, below), while another
/// Connection of the same engine is answered as ever; the ops endpoint counts one panic that ended a
/// Connection. Its writer and its log forwarder are both tasks of its
/// owner, which a panicking task the test starts on that owner stands for.
#[test]
fn a_connection_whose_own_task_panics_ends_alone() {
    let env = TestEnv::new();
    let (metrics, registry, _provider) = metering();
    let engine = engine_with(Product::Collie, None).with_metrics(metrics);
    let (sent, closed, served, answered) = goodfetch_test_support::runtime().block_on(async {
        let dependencies = dependencies(&env);
        let owner = TaskOwner::root(dependencies.blocking);
        let (runs, events, context) = engine.start_runs(dependencies, &owner, None, None);
        let (failing_owner, staying_owner) = (
            owner.child_for(OwnerKind::Connection),
            owner.child_for(OwnerKind::Connection),
        );
        let (mut failing, failing_connection) = connected().await;
        let (mut staying, staying_connection) = connected().await;
        let (ended, serving_ended) = oneshot::channel();
        let failing_served = async {
            engine
                .serve(
                    failing_connection,
                    &failing_owner,
                    runs.clone(),
                    events.clone(),
                    context.clone(),
                )
                .await;
            let _ = ended.send(());
        };
        let staying_served = engine.serve(
            staying_connection,
            &staying_owner,
            runs.clone(),
            events.clone(),
            context.clone(),
        );
        let talk = async {
            opened(&mut failing).await;
            opened(&mut staying).await;
            failing
                .send(&[request(1, "test.wait"), request(2, "test.surface")])
                .await;
            // Its answer comes once the call before it started.
            reply(&mut failing, 2).await;
            failing_owner.spawn(async { panic!("{PANIC_TEXT}") });
            let (sent, closed) = until_closed(&mut failing).await;
            let served = tokio::time::timeout(OWED, serving_ended).await.is_ok();
            staying.send(&[request(1, "test.surface")]).await;
            let answered = reply(&mut staying, 1).await;
            staying.close().await;
            (sent, closed, served, answered)
        };
        let observed = tokio::select! {
            biased;
            observed = talk => observed,
            ((), ()) = async { tokio::join!(failing_served, staying_served) } => {
                unreachable!("the staying Connection is served until its Client closes")
            }
        };
        runs.stop_all().await;
        drop(runs);
        failing_owner.stop().await;
        staying_owner.stop().await;
        owner.stop().await;
        observed
    });

    assert_eq!(
        (
            cancelled(&sent, 1),
            closed,
            served,
            answered["result"]["surface"].as_str(),
            series(&registry).contains(r#"goodfetch_engine_panics_total{ended="connection"} 1"#),
        ),
        (true, true, true, Some("sdk_typescript"), true),
        "{sent:?} {answered}"
    );
}

/// A Client's end of an in-memory Connection, and the engine's.
async fn connected() -> (Client, Connection) {
    let (network, dialer) = transport::in_memory();
    let mut network = Network::InMemory(network);
    let (client, connection) = tokio::join!(dialer.connect(), network.attached_connection());
    let (reader, writer) = client.unwrap().split();
    (
        Client {
            lines: BufReader::new(reader).lines(),
            writer,
        },
        connection.unwrap(),
    )
}

/// A call whose handler panics as the request arrives, before its future:
/// in the task that serves its Connection, which runs that part in the
/// Connection's arrival order (ADR 0080).
const PANIC_ON_ARRIVAL: Operation = Operation {
    name: "test.panic_on_arrival",
    description: "Panics as it arrives.",
    ..PANIC
};

/// The part of `test.panic_on_arrival` that runs as it arrives.
fn arrives(_: Value, _: CallContext) -> std::future::Ready<Result<Value, CallError>> {
    panic!("{PANIC_TEXT}")
}

fn panicking_on_arrival() -> Contribution {
    Contribution::new("test_panic_on_arrival")
        .with_handler(Handler::new(&PANIC_ON_ARRIVAL, arrives))
}

/// On a Daemon, a Connection whose serving task panics while it handles a
/// message ends alone: its call in flight is answered `call_cancelled`, it
/// closes, and the Run its SDK Client started is stopped, while another
/// Client's call on its own Connection is answered and the Daemon goes on;
/// the ops endpoint counts one panic that ended a Connection.
#[test]
fn a_daemon_connection_whose_serving_task_panics_ends_alone() {
    let env = TestEnv::new();
    let (metrics, registry, _provider) = metering();
    let engine = engine_with(Product::Collie, Some(panicking_on_arrival())).with_metrics(metrics);

    let (_, (sent, closed, live)) = daemon(&engine, &env, the_pick(), |dialer| async move {
        let mut failing = daemon_opened(&dialer, sdk_hello()).await;
        let mut staying = daemon_opened(&dialer, sdk_hello()).await;
        call(&mut failing, 1, "run.start", json!({ "run_id": "left" })).await;
        call(&mut staying, 1, "run.start", json!({ "run_id": "stays" })).await;
        failing
            .send(&[request(2, "test.wait"), request(3, "test.panic_on_arrival")])
            .await;
        let (sent, closed) = until_closed(&mut failing).await;
        tokio::time::sleep(Duration::from_mins(1)).await;
        let status = call(&mut staying, 2, "daemon.status", json!({})).await;
        staying.close().await;
        (sent, closed, status["result"]["live_runs"].clone())
    });

    assert_eq!(
        (
            cancelled(&sent, 2),
            closed,
            live,
            index_row(&env, "left")["lifecycle"].clone(),
            series(&registry).contains(r#"goodfetch_engine_panics_total{ended="connection"} 1"#),
        ),
        (true, true, json!(1), json!("stopped"), true),
        "{sent:?}"
    );
}

/// On an Attached engine, whose one Connection is its program, a serving
/// task that panics while it handles a message ends the engine with its
/// Connection: its call in flight is answered `call_cancelled`, the
/// Connection closes and the engine returns, as when that Connection
/// closes.
#[test]
fn an_attached_engine_whose_serving_task_panics_ends() {
    let env = TestEnv::new();
    let engine = engine_with(Product::Collie, Some(panicking_on_arrival()));

    let (sent, closed) = attached_on(&engine, &env, |mut client: Client| async move {
        opened(&mut client).await;
        client
            .send(&[request(1, "test.wait"), request(2, "test.panic_on_arrival")])
            .await;
        until_closed(&mut client).await
    });

    assert_eq!((cancelled(&sent, 1), closed), (true, true), "{sent:?}");
}
