//! A call on one Run answers while another Run's call waits on its disk,
//! through the engine's public interface over the in-memory network (spec
//! #213's Seam 3; ADR 0145, issue 2258): the Runs' registry does a Run's
//! disk work beside its loop, so a slow run file holds that Run's calls
//! only. The holding VFS holds the next read or write of the run file or
//! its WAL. A Run's handle says its end is decided in the step where
//! `Runs::live` stops finding it, before that disk work (issue 3269).

use std::sync::Arc;

use goodfetch_protocol::catalog::{
    Operation, OperationClass, OperationKind, Product, Reach, Stability, schema_of,
};
use goodfetch_protocol::run::RunId;
use goodfetch_test_support::{HeldVfs, TestEnv};
use rstest::rstest;
use serde_json::{Value, json};
use tokio::sync::Mutex;

use super::run::{call, run_folder};
use super::{attached_with, dependencies, engine_with, hello};
use crate::call_error::CallError;
use crate::composition::{CallContext, Contribution, Handler};
use crate::engine::Dependencies;
use crate::run_state::RunStateOptions;
use crate::runs::{self, Run};
use crate::tasks::BlockingPolicy;

/// The parameters of `method` on `run_id`.
fn params(method: &str, run_id: &str) -> Value {
    if method == "run.set_progress_message" {
        json!({ "run_id": run_id, "message": "on its way" })
    } else {
        json!({ "run_id": run_id })
    }
}

/// While Run `a`'s call waits on its run file, a progress message on the
/// live Run `b` is answered, the run file still held: a start that finds an ended Run, a
/// progress message, the report of an ended Run and a resume reading the
/// stored key each wait beside the registry, whatever they answer.
///
/// Covers: operation:run.set_progress_message
#[rstest]
#[case::start_found(&["run.finish"], "run.start")]
#[case::progress_message(&[], "run.set_progress_message")]
#[case::ended_report(&["run.finish"], "run.finish")]
#[case::stored_key(&["run.finish"], "run.resume")]
fn a_call_on_another_run_answers_while_one_runs_call_waits_on_its_disk(
    #[case] setup: &[&str],
    #[case] held_call: &str,
) {
    let env = TestEnv::new();
    let vfs = HeldVfs::register();
    let run_file = run_folder(&env, "a").join("run.sqlite");

    let replies = attached_with(
        &engine_with(Product::Collie, None),
        || Dependencies {
            blocking: BlockingPolicy::Threads,
            run_state: RunStateOptions {
                vfs: Some(vfs.name().to_owned()),
                ..RunStateOptions::default()
            },
            ..dependencies(&env)
        },
        |mut client| {
            let vfs = &vfs;
            async move {
                client.send(&[hello("collie", "0.1")]).await;
                client.next().await;
                call(&mut client, 1, "run.start", json!({ "run_id": "b" })).await;
                call(&mut client, 2, "run.start", json!({ "run_id": "a" })).await;
                for (id, method) in (3..).zip(setup) {
                    call(&mut client, id, method, params(method, "a")).await;
                }
                let held = vfs.hold_next(&run_file);
                client
                    .send(&[json!({
                        "jsonrpc": "2.0", "id": 10, "method": held_call,
                        "params": params(held_call, "a"),
                    })])
                    .await;
                assert!(
                    held.reached().await,
                    "the call on Run a never reached its run file"
                );
                client
                    .send(&[json!({
                        "jsonrpc": "2.0", "id": 11, "method": "run.set_progress_message",
                        "params": params("run.set_progress_message", "b"),
                    })])
                    .await;
                let mut replies = Vec::new();
                let mut holding = false;
                while replies.len() < 2 {
                    let message = client.next().await.expect("the engine closed");
                    if message["id"] == 11 {
                        holding = held.holding();
                        held.let_go();
                    }
                    if message["id"] == 10 || message["id"] == 11 {
                        replies.push(message);
                    }
                }
                (holding, replies)
            }
        },
    );

    let (holding, replies) = replies;
    let order: Vec<_> = replies.iter().map(|reply| reply["id"].clone()).collect();
    assert_eq!(
        (holding, order),
        (true, vec![json!(11), json!(10)]),
        "{replies:?}"
    );
}

/// An operation of the tests' on a live Run, as a crawl's code holds it.
const fn on_a_run(name: &'static str, description: &'static str) -> Operation {
    Operation {
        name,
        description,
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
    }
}

/// Holds the handle of the live Run `run_id`, as a crawl's delivery does.
const HOLD_RUN: Operation = on_a_run(
    "test.hold_run",
    "Holds the handle of the live Run `run_id`.",
);

/// Whether the held handle's end is decided, and whether `Runs::live`
/// still finds its Run; with `let_go`, the handle is dropped after.
const END_DECIDED: Operation = on_a_run(
    "test.end_decided",
    "Answers whether the held Run's end is decided, and whether its Run is still found live, then drops the handle when `let_go` says so.",
);

/// The tests' contribution that holds a Run's handle in `held`.
fn holding_a_run(held: &Arc<Mutex<Option<Run>>>) -> Contribution {
    let (holding, reading) = (Arc::clone(held), Arc::clone(held));
    Contribution::new("test-holding-a-run")
        .with_handler(Handler::new(
            &HOLD_RUN,
            move |params: Value, context: CallContext| {
                let held = Arc::clone(&holding);
                async move {
                    let run_id = RunId::parse(params["run_id"].as_str().unwrap()).unwrap();
                    let run = context.runs().live(run_id).await.unwrap();
                    *held.lock().await = Some(run);
                    Ok::<Value, CallError>(json!({}))
                }
            },
        ))
        .with_handler(Handler::new(
            &END_DECIDED,
            move |params: Value, context: CallContext| {
                let held = Arc::clone(&reading);
                async move {
                    let mut held = held.lock().await;
                    let run = held.as_ref().unwrap();
                    let found = context
                        .runs()
                        .live(runs::identity(run).id().clone())
                        .await
                        .is_ok();
                    let decided = runs::end(run).is_decided();
                    if params["let_go"] == true {
                        // The engine's end waits for the Run's writer, which
                        // its last handle keeps.
                        held.take();
                    }
                    Ok::<Value, CallError>(json!({ "decided": decided, "found": found }))
                }
            },
        ))
}

/// A Run's handle says its end is decided from the registry's step that
/// decides it, when `Runs::live` stops finding the Run, before the end is
/// recorded in its run file, and still says so once the Run ended: a crawl
/// that answers through the handle it holds answers as the registry does.
#[test]
fn a_runs_handle_says_its_end_is_decided_once_runs_live_no_longer_finds_it() {
    let env = TestEnv::new();
    let vfs = HeldVfs::register();
    let run_file = run_folder(&env, "a").join("run.sqlite");
    let held = Arc::new(Mutex::new(None));

    let answers = attached_with(
        &engine_with(Product::Collie, Some(holding_a_run(&held))),
        || Dependencies {
            blocking: BlockingPolicy::Threads,
            run_state: RunStateOptions {
                vfs: Some(vfs.name().to_owned()),
                ..RunStateOptions::default()
            },
            ..dependencies(&env)
        },
        |mut client| {
            let vfs = &vfs;
            async move {
                client.send(&[hello("collie", "0.1")]).await;
                client.next().await;
                call(&mut client, 1, "run.start", json!({ "run_id": "a" })).await;
                call(&mut client, 2, "test.hold_run", json!({ "run_id": "a" })).await;
                let live = call(&mut client, 3, "test.end_decided", json!({})).await;
                let recording = vfs.hold_next(&run_file);
                client
                    .send(&[json!({
                        "jsonrpc": "2.0", "id": 10, "method": "run.finish",
                        "params": { "run_id": "a" },
                    })])
                    .await;
                assert!(
                    recording.reached().await,
                    "the end never reached its run file"
                );
                let deciding = call(&mut client, 11, "test.end_decided", json!({})).await;
                let held_then = recording.holding();
                recording.let_go();
                let finished = loop {
                    let message = client.next().await.expect("the engine closed");
                    if message["id"] == 10 {
                        break message;
                    }
                };
                assert!(finished.get("result").is_some(), "{finished}");
                let ended = call(
                    &mut client,
                    12,
                    "test.end_decided",
                    json!({ "let_go": true }),
                )
                .await;
                (
                    live["result"].clone(),
                    (held_then, deciding["result"].clone()),
                    ended["result"].clone(),
                )
            }
        },
    );

    assert_eq!(
        answers,
        (
            json!({ "decided": false, "found": true }),
            (true, json!({ "decided": true, "found": false })),
            json!({ "decided": true, "found": false }),
        )
    );
}
