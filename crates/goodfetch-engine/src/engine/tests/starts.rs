//! What every start from a Client is checked for (ADRs 0036, 0078 and
//! 0102): a Schedule's reserved tags are refused, a `detached` start on an
//! engine that ends with its Connection fails with `detach_unsupported`,
//! and a detached Run on the Daemon outlives the Connection that started
//! it.

use std::collections::BTreeMap;

use goodfetch_protocol::catalog::{
    Operation, OperationClass, OperationKind, PROTOCOL_VERSION, Product, Reach, Stability,
    schema_of,
};
use goodfetch_protocol::run::{RunId, RunKind, Tags};
use goodfetch_test_support::TestEnv;
use rstest::rstest;
use serde_json::{Value, json};

use super::daemon::{cli_hello, daemon, opened, sdk_hello, the_pick};
use super::run::{call, index_row};
use super::{attached_on, engine_with, hello};
use crate::call_error::CallError;
use crate::composition::{CallContext, Contribution, Handler};
use crate::runs::{EndedBy, Start, Started};

/// Starts the Run its params' `run_id` names, detached when they say so,
/// which `run.finish` ends.
const START: Operation = Operation {
    name: "test.start",
    description: "Starts a Run, detached when the params say so.",
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
    arrived_in: PROTOCOL_VERSION,
};

fn starting() -> Contribution {
    Contribution::new("test-start").with_handler(Handler::new(
        &START,
        |params: Value, context: CallContext| async move {
            let started = context
                .runs()
                .start(Start {
                    run_id: Some(RunId::parse(params["run_id"].as_str().unwrap()).unwrap()),
                    tags: Tags::default(),
                    kind: RunKind::Collie,
                    needs: Product::Collie,
                    inputs: BTreeMap::new(),
                    ended_by: EndedBy::Finish,
                    initialize: None,
                    policy_version: None,
                    weakened: Vec::new(),
                    binding: None,
                    clean_up_digest: None,
                    checkpoint_interval: None,
                    timeout: None,
                    settings: None,
                    detached: params["detached"] == json!(true),
                })
                .await
                .unwrap();
            let Started::New(run) = started else {
                panic!("the Run began before");
            };
            Ok::<Value, CallError>(json!({ "run_id": run.id().to_string() }))
        },
    ))
}

/// Sends `method` with `params` on an Attached engine, the first call of
/// its Connection, and answers the reply.
fn attached_call(env: &TestEnv, method: &str, params: Value) -> Value {
    attached_on(
        &engine_with(Product::Collie, None),
        env,
        |mut client| async move {
            client.send(&[hello("collie", "0.1")]).await;
            client.next().await;
            call(&mut client, 1, method, params).await
        },
    )
}

/// A Schedule's reserved tags are the Daemon's scheduler's alone, so a
/// start from a Client that carries one, whichever start, is refused before
/// any Run begins; a Recipe's tag and any other tag are not.
///
/// Covers: story:3-173, operation:run.start, operation:recipe.start, operation:catalog_run.start
#[rstest]
#[case::schedule("run.start", "goodfetch.schedule:nightly")]
#[case::occurrence("run.start", "goodfetch.occurrence:2026-11-23T04:00:00Z")]
#[case::trigger("run.start", "goodfetch.trigger:time")]
#[case::creator("run.start", "goodfetch.creator:ada")]
#[case::role("run.start", "goodfetch.role:watch")]
#[case::recipe_start("recipe.start", "goodfetch.schedule:nightly")]
#[case::catalog_run_start("catalog_run.start", "goodfetch.role:target")]
fn a_start_from_a_client_with_a_schedules_reserved_tag_is_invalid_params_and_starts_no_run(
    #[case] method: &str,
    #[case] tag: &str,
) {
    let env = TestEnv::new();
    let mut params = json!({ "run_id": "tagged", "tags": ["nightly", tag] });
    match method {
        "recipe.start" => params["recipe"] = json!("prices"),
        "catalog_run.start" => {
            params["target"] = json!({ "kind": "plan_replay", "plan": "plan.json" });
        }
        _ => {}
    }

    let refused = attached_call(&env, method, params);

    assert_eq!(
        (
            &refused["error"]["data"]["code"],
            refused["error"]["message"]
                .as_str()
                .is_some_and(|message| message.contains(tag)),
            env.home().join("1").exists(),
        ),
        (&json!("invalid_params"), true, false),
        "{refused}"
    );
}

/// A Recipe's tag, which its starter sets, and a tag that only looks
/// reserved start the Run.
///
/// Covers: story:3-173
#[rstest]
#[case::recipe("goodfetch.recipe:nightly-prices")]
#[case::no_key("goodfetch.schedule")]
#[case::other_prefix("schedule:nightly")]
fn a_start_with_a_tag_the_scheduler_does_not_reserve_starts(#[case] tag: &str) {
    let env = TestEnv::new();

    let started = attached_call(
        &env,
        "run.start",
        json!({ "run_id": "tagged", "tags": [tag] }),
    );

    assert_eq!(
        (
            &started["result"]["started_as"],
            index_row(&env, "tagged")["tags"].clone()
        ),
        (&json!("new"), json!([tag])),
        "{started}"
    );
}

/// An Attached engine, and the linked engine that serves its Connection
/// in-process the same way, ends with its Connection, so a `detached`
/// start fails with `detach_unsupported` whose `fix` names `engine:
/// daemon`, before any Run begins.
///
/// Covers: story:3-174, error:detach_unsupported
#[rstest]
#[case::recipe_start(json!({ "recipe": "prices", "detached": true }), "recipe.start")]
#[case::catalog_run_start(
    json!({ "target": { "kind": "plan_replay", "plan": "plan.json" }, "detached": true }),
    "catalog_run.start"
)]
fn a_detached_start_on_an_attached_engine_is_detach_unsupported_naming_the_daemon(
    #[case] params: Value,
    #[case] method: &str,
) {
    let env = TestEnv::new();

    let refused = attached_call(&env, method, params);

    assert_eq!(
        (
            &refused["error"]["data"]["code"],
            &refused["error"]["data"]["key"],
            refused["error"]["data"]["fix"]
                .as_str()
                .is_some_and(|fix| fix.contains("`engine: daemon`")),
            env.home().join("1").exists(),
        ),
        (&json!("detach_unsupported"), &json!("engine"), true, false),
        "{refused}"
    );
}

/// A detached Run belongs to no Connection, so the end of the SDK
/// program's Connection that started it leaves it running on the Daemon,
/// while the Run it started attached stops with it (ADRs 0078 and 0102).
///
/// Covers: story:3-174
#[test]
fn a_detached_run_on_the_daemon_outlives_the_connection_that_started_it() {
    let env = TestEnv::new();

    let (_, (live, kept)) = daemon(
        &engine_with(Product::Collie, Some(starting())),
        &env,
        the_pick(),
        |dialer| async move {
            let mut program = opened(&dialer, sdk_hello()).await;
            let detached = json!({ "run_id": "kept", "detached": true });
            call(&mut program, 1, "test.start", detached).await;
            call(&mut program, 2, "test.start", json!({ "run_id": "left" })).await;
            program.close().await;
            while program.next().await.is_some() {}
            tokio::time::sleep(std::time::Duration::from_secs(60)).await;
            let mut next = opened(&dialer, cli_hello()).await;
            let status = call(&mut next, 1, "daemon.status", json!({})).await;
            let kept = call(&mut next, 2, "run.finish", json!({ "run_id": "kept" })).await;
            next.close().await;
            (
                status["result"]["live_runs"].clone(),
                kept["result"]["lifecycle"].clone(),
            )
        },
    );

    assert_eq!(
        (live, kept, index_row(&env, "left")["lifecycle"].clone()),
        (json!(1), json!("finished"), json!("stopped"))
    );
}
