//! The engine's and a Run's transition tables as the engine applies them
//! (ADR 0128), through its public interface over the in-memory network
//! (spec #213's Seam 3): a late event changes nothing where the table says
//! so, a program's refused transition answers its Error code, and a
//! Connection's second `hello` is refused while it goes on serving, and
//! an engine's stop ends its Audit log's writer.

use std::collections::BTreeMap;
use std::fs::OpenOptions;

use goodfetch_protocol::audit::{Actor, SECURITY_SWITCH_TURNED_OFF};
use goodfetch_protocol::catalog::{
    Operation, OperationClass, OperationKind, Product, Reach, Stability, schema_of,
};
use goodfetch_protocol::connection::{StoppedBy, Surface};
use goodfetch_protocol::run::{RunId, RunKind, Tags};
use goodfetch_test_support::TestEnv;
use serde_json::{Value, json};

use super::run::{call, index_row};
use super::{attached_on, dependencies, engine_with, hello};
use crate::audit::{AuditError, Origin};
use crate::call_error::CallError;
use crate::composition::{CallContext, Contribution, Handler};
use crate::runs::{Aborter, EndedBy, Ending, Forwarded, Start, Started};
use crate::tasks::TaskOwner;

/// Starts a Run its driver ends, aborts it, and has the driver finish it
/// while the abort's end is being written; answers what the abort and a
/// later finish report.
const ABORT_THEN_FINISH: Operation = Operation {
    name: "test.abort_then_finish",
    description: "Aborts a Run while its driver finishes it.",
    class: OperationClass::Effect,
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

fn abort_then_finish() -> Contribution {
    Contribution::new("test-lifecycle").with_handler(Handler::new(
        &ABORT_THEN_FINISH,
        |_: Value, context: CallContext| async move {
            let runs = context.runs();
            let started = runs
                .start(Start {
                    run_id: Some(RunId::parse("raced").unwrap()),
                    tags: Tags::default(),
                    kind: RunKind::Collie,
                    needs: Product::Collie,
                    inputs: BTreeMap::new(),
                    ended_by: EndedBy::Driver,
                    initialize: None,
                    policy_version: None,
                    weakened: Vec::new(),
                    binding: None,
                    clean_up_digest: None,
                    checkpoint_interval: None,
                    timeout: None,
                    settings: None,
                    detached: false,
                })
                .await
                .unwrap();
            let Started::New(run) = started else {
                panic!("the Run began before");
            };
            // The abort reaches the registry first; the driver's finish
            // follows it into the registry before the abort's end is written,
            // and the driver lets its Run go, as a driver does once it ends.
            let run_id = run.id().clone();
            let driver = async move { run.end(Ending::Finished(Vec::new())).await };
            let (aborted, ()) = tokio::join!(
                runs.abort(
                    run_id.clone(),
                    Aborter::by(StoppedBy::Client),
                    Forwarded::No
                ),
                driver
            );
            let finished = runs.finish(run_id).await.unwrap();
            Ok::<Value, CallError>(json!({
                "aborted": aborted.unwrap(),
                "finished": finished,
            }))
        },
    ))
}

/// A driver's finish that reaches a Run whose abort is ending it is a late
/// event: the Run stays `stopped` by the abort (ADR 0128).
///
/// Covers: story:3-151
#[test]
fn a_finish_racing_an_abort_leaves_the_run_stopped_by_the_abort() {
    let env = TestEnv::new();

    let answer = attached_on(
        &engine_with(Product::Collie, Some(abort_then_finish())),
        &env,
        |mut client| async move {
            client.send(&[hello("collie", "0.1")]).await;
            client.next().await;
            call(&mut client, 1, "test.abort_then_finish", json!({})).await
        },
    );

    let stopped = json!(["stopped", { "kind": "abort", "by": "client" }]);
    assert_eq!(
        (
            json!([
                answer["result"]["aborted"]["lifecycle"],
                answer["result"]["aborted"]["stop_reason"]
            ]),
            json!([
                answer["result"]["finished"]["lifecycle"],
                answer["result"]["finished"]["stop_reason"]
            ]),
            index_row(&env, "raced")["lifecycle"].clone(),
        ),
        (stopped.clone(), stopped, json!("stopped"))
    );
}

/// A pause, a migration or a finish of a Run that already ended changes
/// nothing: each answers the Run's report (ADR 0128).
///
/// Covers: story:3-151
#[rstest::rstest]
#[case::pause("run.pause")]
#[case::migrate("run.migrate")]
#[case::finish("run.finish")]
fn a_stop_or_finish_of_an_ended_run_answers_its_report_unchanged(#[case] method: &str) {
    let env = TestEnv::new();
    let method = method.to_owned();

    let answer = attached_on(
        &engine_with(Product::Collie, None),
        &env,
        |mut client| async move {
            client.send(&[hello("collie", "0.1")]).await;
            client.next().await;
            call(&mut client, 1, "run.start", json!({ "run_id": "ended" })).await;
            call(&mut client, 2, "run.finish", json!({ "run_id": "ended" })).await;
            call(&mut client, 3, &method, json!({ "run_id": "ended" })).await
        },
    );

    assert_eq!(
        json!([
            answer["result"]["lifecycle"],
            answer["result"]["stop_reason"]
        ]),
        json!(["finished", null]),
        "{answer}"
    );
}

/// A resume of a stopped Run whose writer's lock another writer holds is
/// refused with `run_in_use`, as the Run's table says of a writer found
/// (ADR 0102), and its state is left as it was.
///
/// Covers: story:3-151, error:run_in_use, operation:run.resume
#[test]
fn a_resume_of_a_run_another_writer_holds_is_run_in_use_and_changes_nothing() {
    let env = TestEnv::new();
    let folder = env.home().join("1").join("runs").join("held");

    let refused = attached_on(
        &engine_with(Product::Collie, None),
        &env,
        |mut client| async move {
            client.send(&[hello("collie", "0.1")]).await;
            client.next().await;
            call(&mut client, 1, "run.start", json!({ "run_id": "held" })).await;
            call(&mut client, 2, "run.abort", json!({ "run_id": "held" })).await;
            let lock = OpenOptions::new()
                .write(true)
                .open(folder.join("run.lock"))
                .unwrap();
            lock.lock().unwrap();
            let refused = call(&mut client, 3, "run.resume", json!({ "run_id": "held" })).await;
            drop(lock);
            refused
        },
    );

    assert_eq!(
        (
            &refused["error"]["data"]["code"],
            &index_row(&env, "held")["lifecycle"]
        ),
        (&json!("run_in_use"), &json!("stopped"))
    );
}

/// A second `hello` on an open Connection is refused with
/// `invalid_request`, and the Connection goes on serving (ADR 0128).
///
/// Covers: story:3-152
#[test]
fn a_second_hello_is_invalid_request_and_the_connection_goes_on() {
    let env = TestEnv::new();

    let (again, info) = attached_on(
        &engine_with(Product::Collie, None),
        &env,
        |mut client| async move {
            client.send(&[hello("collie", "0.1")]).await;
            client.next().await;
            client
                .send(&[json!({ "jsonrpc": "2.0", "id": 1, "method": "hello",
                    "params": hello("collie", "0.1")["params"] })])
                .await;
            let again = client.next().await.unwrap();
            let info = call(&mut client, 2, "engine.info", json!({})).await;
            (again, info)
        },
    );

    assert_eq!(
        (&again["error"]["data"]["code"], &info["result"]["product"]),
        (&json!("invalid_request"), &json!("collie"))
    );
}

/// An engine's stop ends its Audit log's writer once its other tasks
/// ended, and waits for it, as for every task it started (ADR 0060): an
/// entry recorded before the stop is appended, and one recorded after it
/// is refused, the writer no longer running.
#[test]
fn an_engines_stop_ends_its_audit_logs_writer() {
    let env = TestEnv::new();
    let engine = engine_with(Product::Collie, None);
    let (before, after) = goodfetch_test_support::runtime().block_on(async {
        let dependencies = dependencies(&env);
        let owner = TaskOwner::root(dependencies.blocking);
        let (runs, _events, context) = engine.start_runs(dependencies, &owner, None, None);
        let origin = Origin {
            actor: Actor::ClientSurface {
                surface: Surface::Cli,
            },
            surface: None,
            layer: None,
            run_id: None,
        };
        let now = context.clock().timestamp();
        let record = || {
            context
                .audit()
                .record(&origin, &SECURITY_SWITCH_TURNED_OFF, json!({}), now)
        };
        let before = record().await;
        runs.stop_all().await;
        drop(runs);
        owner.stop().await;
        (before, record().await)
    });

    assert!(
        before.is_ok() && matches!(after, Err(AuditError::Writer { .. })),
        "{before:?} {after:?}"
    );
}
