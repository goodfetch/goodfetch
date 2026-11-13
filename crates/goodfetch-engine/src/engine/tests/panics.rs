//! A panic, through the engine's public interface over the in-memory
//! network (ADR 0148). A call whose handler panics is answered
//! `engine_panicked`, naming what ended and the crash report, never the
//! panic's own text; its place in flight is given back, its Connection
//! goes on, and the ops endpoint counts the panic. A Connection whose own
//! task panics ends alone, as when its Client goes away, and an Attached
//! engine ends with it. A panic in a part every Run shares stops the
//! engine and nothing is restarted: a Daemon drains its Runs and ends; an
//! Attached engine stops its Runs for the panic, waiting for them at most
//! its drain grace, and fails. Each such Run resumes by its Run ID. The
//! panic comes from an extra operation the test hands the engine, or a task
//! it starts on a Connection's or the engine's owner, never from a switch
//! in a Product.

use std::cell::Cell;
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
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use super::daemon::{
    cli_hello, daemon, daemon_within, draining, driven, opened as daemon_opened, sdk_hello,
    stuck_handler, the_pick,
};
use super::ops::{metering, series};
use super::run::{call, index_row, run_folder, run_row};
use super::{
    Client, IN_FLIGHT_CAP, OWED, attached_on, attached_with, dependencies, engine_with, opened,
    owed, records_of, request,
};
use crate::call_error::CallError;
use crate::composition::{CallContext, Contribution, Handler};
use crate::crash_report::record_panics;
use crate::engine::Engine;
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

/// A call that starts a panicking task on the engine's own owner, as a
/// part every Run shares would panic: the event stream, the governor's
/// sampler, Politeness's store, the Needs-you list or a listener.
const PANIC_IN_A_SHARED_PART: Operation = Operation {
    name: "test.panic_in_a_shared_part",
    description: "Panics in a part every Run shares.",
    ..PANIC
};

fn panicking_parts() -> Contribution {
    Contribution::new("test_panicking_parts").with_handler(Handler::new(
        &PANIC_IN_A_SHARED_PART,
        |_: Value, context: CallContext| async move {
            context
                .engine()
                .owner()
                .spawn(async { panic!("{PANIC_TEXT}") });
            Ok::<Value, CallError>(json!({}))
        },
    ))
}

/// How the Daemon `engine` ended in `env` once a Client that started the
/// Run `run_id` made a part every Run shares panic, and whether that
/// Client's Connection closed; a Daemon still serving is asked to drain
/// after the wait, so a Daemon that goes on ends too.
fn daemon_panicked(engine: &Engine, env: &TestEnv, run_id: &str) -> (String, bool) {
    let asked = CancellationToken::new();
    let (end, closed) = daemon(engine, env, draining(&asked), |dialer| async move {
        let mut client = daemon_opened(&dialer, cli_hello()).await;
        call(&mut client, 1, "run.start", json!({ "run_id": run_id })).await;
        client
            .send(&[request(2, "test.panic_in_a_shared_part")])
            .await;
        let (_, closed) = until_closed(&mut client).await;
        asked.cancel();
        closed
    });
    (format!("{end:?}"), closed)
}

/// The report of the Run `run_id` in `env`, read by a new Attached engine,
/// then the answer to `run.resume` with its Run ID.
fn reported_then_resumed(env: &TestEnv, run_id: &str) -> (Value, Value) {
    attached_on(
        &engine_with(Product::Collie, None),
        env,
        |mut client: Client| async move {
            opened(&mut client).await;
            let report = call(&mut client, 1, "run.report", json!({ "run_id": run_id })).await;
            let resumed = call(&mut client, 2, "run.resume", json!({ "run_id": run_id })).await;
            client.close().await;
            (report, resumed)
        },
    )
}

/// The codes of the warnings a Run's `report` answer carries.
fn warnings(report: &Value) -> Vec<Value> {
    report["result"]["warnings"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|warning| warning["code"].clone())
        .collect()
}

/// A part every Run shares that panics stops a Daemon (ADR 0148): while
/// its Runs' registry runs, it drains as on a requested drain, each live
/// Run `stopped` for the drain and carrying the warning `engine_panicked`
/// in its report, resumable by its Run ID, and ends as panicked, which its
/// executable exits 1 on; the ops endpoint counts one panic that ended the
/// engine.
#[test]
fn a_daemon_whose_shared_part_panics_drains_its_runs_and_ends() {
    let env = TestEnv::new();
    let (metrics, registry, _provider) = metering();
    let engine = engine_with(Product::Collie, Some(panicking_parts())).with_metrics(metrics);

    let (end, closed) = daemon_panicked(&engine, &env, "drained");

    let stopped = run_row(&run_folder(&env, "drained"));
    let (report, resumed) = reported_then_resumed(&env, "drained");
    assert_eq!(
        (
            end,
            closed,
            stopped["lifecycle"].clone(),
            stopped["stop_reason"].clone(),
            warnings(&report),
            resumed["result"]["started_as"].clone(),
            series(&registry).contains(r#"goodfetch_engine_panics_total{ended="engine"} 1"#),
        ),
        (
            "Panicked".to_owned(),
            true,
            json!("stopped"),
            json!({ "kind": "drain" }),
            vec![json!("engine_panicked")],
            json!("resumed"),
            true,
        ),
        "{report}"
    );
}

/// A part every Run shares that panics stops an Attached engine, which
/// has no drain (ADR 0148): it stops its Runs through the registry, as
/// when its program goes away, each `stopped` for the engine's panic,
/// never as aborted by its Client, with the warning `engine_panicked` in
/// its report and resumable by its Run ID; its Connection closes and the
/// engine returns its failure, which its executable exits 1 on, so its
/// program reads `engine_exited` with that code.
#[test]
fn an_attached_engine_whose_shared_part_panics_stops_its_runs_and_fails() {
    let env = TestEnv::new();
    let (metrics, registry, _provider) = metering();
    let engine = engine_with(Product::Collie, Some(panicking_parts())).with_metrics(metrics);

    let (served, closed) = goodfetch_test_support::runtime().block_on(async {
        let (network, dialer) = transport::in_memory();
        let talk = async {
            let (reader, writer) = dialer.connect().await.unwrap().split();
            let mut client = Client {
                lines: BufReader::new(reader).lines(),
                writer,
            };
            opened(&mut client).await;
            call(&mut client, 1, "run.start", json!({ "run_id": "stopped" })).await;
            client
                .send(&[request(2, "test.panic_in_a_shared_part")])
                .await;
            let (_, closed) = until_closed(&mut client).await;
            if !closed {
                client.close().await;
            }
            closed
        };
        let (served, closed) = tokio::join!(
            engine.run_attached(Network::InMemory(network), dependencies(&env)),
            talk
        );
        (format!("{served:?}"), closed)
    });

    let stopped = run_row(&run_folder(&env, "stopped"));
    let (report, resumed) = reported_then_resumed(&env, "stopped");
    assert_eq!(
        (
            served,
            closed,
            stopped["lifecycle"].clone(),
            stopped["stop_reason"].clone(),
            warnings(&report),
            resumed["result"]["started_as"].clone(),
            series(&registry).contains(r#"goodfetch_engine_panics_total{ended="engine"} 1"#),
        ),
        (
            "Err(Panicked)".to_owned(),
            true,
            json!("stopped"),
            json!({ "kind": "engine_panicked" }),
            vec![json!("engine_panicked")],
            json!("resumed"),
            true,
        ),
        "{report}"
    );
}

/// Where the engine's own records are logged.
const ENGINE_TARGET: &str = "goodfetch_engine::engine";

/// An Attached engine stopping on a panic in a part every Run shares waits
/// for its Runs at most its drain grace (`daemon.drain_grace`, ADR 0148):
/// a Run that has not stopped by then stays `running`, resumable from its
/// run file; the engine logs how many it left so, and fails at the grace.
#[test]
fn an_attached_engine_that_panicked_fails_at_its_drain_grace_leaving_a_stuck_run_resumable() {
    let env = TestEnv::new();
    let grace = Duration::from_secs(30);
    // Past the grace, a test whose engine still waits fails, never hangs.
    let bound = grace + Duration::from_secs(60);
    let engine = engine_with(
        Product::Collie,
        Some(panicking_parts().with_handler(stuck_handler())),
    )
    .with_drain_grace(grace);

    let mut ended = None;
    let records = records_of(ENGINE_TARGET, || {
        ended = Some(goodfetch_test_support::runtime().block_on(async {
            let (network, dialer) = transport::in_memory();
            let dependencies = dependencies(&env);
            let clock = dependencies.clock;
            // When the Client asked for the panic, which the engine's wait
            // is measured from.
            let panic_asked = Cell::new(None);
            let served = async {
                tokio::select! {
                    served = engine.run_attached(Network::InMemory(network), dependencies) => {
                        let waited = panic_asked.get().map(|asked: Instant| asked.elapsed());
                        Some((format!("{served:?}"), waited))
                    }
                    () = clock.sleep(bound) => None,
                }
            };
            let talk = async {
                let (reader, writer) = dialer.connect().await.unwrap().split();
                let mut client = Client {
                    lines: BufReader::new(reader).lines(),
                    writer,
                };
                opened(&mut client).await;
                call(&mut client, 1, "test.stuck", json!({ "run_id": "stuck" })).await;
                panic_asked.set(Some(Instant::now()));
                client
                    .send(&[request(2, "test.panic_in_a_shared_part")])
                    .await;
                let _ =
                    tokio::time::timeout(bound, async { while client.next().await.is_some() {} })
                        .await;
            };
            tokio::join!(served, talk).0
        }));
    });

    let served = ended.unwrap();
    // Read before the Run resumes, which rewrites its row.
    let lifecycle = run_row(&run_folder(&env, "stuck"))["lifecycle"].clone();
    let (_, resumed) = reported_then_resumed(&env, "stuck");
    assert_eq!(
        (
            served,
            lifecycle,
            resumed["result"]["started_as"].clone(),
            records.iter().any(|record| record.contains("live_runs=1")),
        ),
        (
            Some(("Err(Panicked)".to_owned(), Some(grace))),
            json!("running"),
            json!("resumed"),
            true,
        ),
        "{records:#?}"
    );
}

/// Starts a Run whose run state takes no more writes, as one whose writer
/// is wedged: the Run's task holds every place in its writer's queue, so
/// the Runs' registry never finishes a command that writes to that Run.
const WEDGED: Operation = Operation {
    name: "test.wedged",
    description: "Starts a Run whose run state takes no more writes.",
    class: OperationClass::Effect,
    ..PANIC
};

/// The places in a Run's writer's queue (`run_state`'s mailbox).
const WRITER_QUEUE: usize = 1024;

fn wedged_handler() -> Handler {
    Handler::new(&WEDGED, |params: Value, context: CallContext| async move {
        let run = driven(&context, &params).await;
        let mut held = Vec::with_capacity(WRITER_QUEUE);
        for _ in 0..WRITER_QUEUE {
            held.push(run.state().reserve().await.unwrap());
        }
        run.spawn(async move {
            let _held = held;
            std::future::pending::<()>().await;
        });
        Ok::<Value, CallError>(json!({ "run_id": run.id().to_string() }))
    })
}

/// Where the Daemon's own records are logged.
const DAEMON_TARGET: &str = "goodfetch_engine::daemon";

/// A Daemon stopping on a panic in a part every Run shares ends within its
/// drain grace even when its Runs' registry never finishes the drain (ADR
/// 0148): it asks the registry nothing more once the grace passed, logs
/// that the registry did not answer, and its Run stays `running`,
/// resumable from its run file.
#[test]
fn a_daemon_whose_registry_never_finishes_the_drain_ends_at_its_drain_grace() {
    let env = TestEnv::new();
    let engine = engine_with(
        Product::Collie,
        Some(panicking_parts().with_handler(wedged_handler())),
    );
    let asked = CancellationToken::new();
    let options = draining(&asked);
    let grace = options.drain.grace;
    // The Client's calls before the panic take no time on the paused
    // Clock, so a Daemon still serving a second past the grace overran it.
    let bound = grace + Duration::from_secs(1);

    let mut ended = None;
    let records = records_of(DAEMON_TARGET, || {
        ended = Some(daemon_within(
            &engine,
            &env,
            BlockingPolicy::Inline,
            options,
            bound,
            |dialer| async move {
                let mut client = daemon_opened(&dialer, cli_hello()).await;
                call(&mut client, 1, "test.wedged", json!({ "run_id": "wedged" })).await;
                client
                    .send(&[request(2, "test.panic_in_a_shared_part")])
                    .await;
                let _ =
                    tokio::time::timeout(bound, async { while client.next().await.is_some() {} })
                        .await;
            },
        ));
    });

    let (end, ()) = ended.unwrap();
    // Read before the Run resumes, which rewrites its row.
    let lifecycle = run_row(&run_folder(&env, "wedged"))["lifecycle"].clone();
    let (_, resumed) = reported_then_resumed(&env, "wedged");
    assert_eq!(
        (
            end.map(|end| format!("{end:?}")),
            lifecycle,
            resumed["result"]["started_as"].clone(),
            records
                .iter()
                .any(|record| record.contains("registry did not answer")),
        ),
        (
            Some("Panicked".to_owned()),
            json!("running"),
            json!("resumed"),
            true,
        ),
        "{records:#?}"
    );
}
