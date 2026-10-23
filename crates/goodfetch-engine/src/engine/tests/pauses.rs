//! Human pauses in a Run's progress, through the engine's public interface
//! over the in-memory network (spec #213's Seam 3; ADRs 0092 and 0102): a
//! pause beginning and ending each give a new value, the pause showing
//! whether the program or the engine raised it. The pauses here are the
//! test's own, raised through the Run as areas 5 and 8 raise theirs: a
//! `test.pause` call holds one until it is cancelled.

use goodfetch_protocol::catalog::{
    Operation, OperationClass, OperationKind, Product, Reach, Stability, schema_of,
};
use goodfetch_protocol::run::{EngineStep, HumanPause, PauseOrigin, RunId};
use rstest::rstest;
use serde_json::{Value, json};

use super::run::call;
use super::{attached, cancel, engine_with, hello};
use crate::composition::{CallContext, CallError, Contribution, Handler};

/// Holds a Human pause on a Run until it is cancelled.
const PAUSE: Operation = Operation {
    name: "test.pause",
    description: "Holds a Human pause on a Run until it is cancelled.",
    class: OperationClass::Effect,
    kind: OperationKind::RoundTrip,
    reach: Reach::AllSurfaces,
    params: schema_of::<Value>,
    result: schema_of::<Value>,
    events: &[],
    cancellable: true,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: goodfetch_protocol::catalog::PROTOCOL_VERSION,
};

fn pauses() -> Contribution {
    Contribution::new("test-pauses").with_handler(Handler::new(
        &PAUSE,
        |params: Value, context: CallContext| async move {
            let run_id = RunId::parse(params["run_id"].as_str().unwrap()).unwrap();
            let run = context.runs().live(run_id).await.unwrap();
            let raised_by: PauseOrigin =
                serde_json::from_value(params["raised_by"].clone()).unwrap();
            let _held = run.human_pause(HumanPause {
                session: "paused.session-1".to_owned(),
                page: "page-1".to_owned(),
                reason: Some("Solve the puzzle".to_owned()),
                until: Some("the URL matches **/account".to_owned()),
                raised_by,
                since: run.point_in_time(),
                deadline: None,
            });
            std::future::pending::<()>().await;
            Ok::<Value, CallError>(json!({}))
        },
    ))
}

/// Covers: story:3-163, operation:run.progress
#[rstest]
#[case::by_the_program(PauseOrigin::Program)]
#[case::by_the_engine(PauseOrigin::Engine { step: EngineStep::ChallengeLadder })]
fn a_human_pause_beginning_and_ending_each_give_a_new_progress_value(
    #[case] raised_by: PauseOrigin,
) {
    let origin = serde_json::to_value(&raised_by).unwrap();
    let (waiting, ended) = attached(
        &engine_with(Product::Collie, Some(pauses())),
        |mut client| async move {
            client.send(&[hello("collie", "0.1")]).await;
            client.next().await.unwrap();
            call(&mut client, 1, "run.start", json!({ "run_id": "paused" })).await;
            let before = call(
                &mut client,
                2,
                "run.progress",
                json!({ "run_id": "paused" }),
            )
            .await;
            client
                .send(&[json!({
                    "jsonrpc": "2.0", "id": 3, "method": "test.pause",
                    "params": { "run_id": "paused", "raised_by": origin },
                })])
                .await;
            let waiting = call(
                &mut client,
                4,
                "run.progress",
                json!({ "run_id": "paused", "after": before["result"]["position"] }),
            )
            .await;
            client.send(&[cancel(3)]).await;
            let ended = call(
                &mut client,
                5,
                "run.progress",
                json!({ "run_id": "paused", "after": waiting["result"]["position"] }),
            )
            .await;
            (
                waiting["result"]["pauses"].clone(),
                ended["result"]["pauses"].clone(),
            )
        },
    );

    assert_eq!(
        (waiting, ended),
        (
            json!([{
                "session": "paused.session-1",
                "page": "page-1",
                "reason": "Solve the puzzle",
                "until": "the URL matches **/account",
                "raised_by": serde_json::to_value(&raised_by).unwrap(),
                "since": "2026-01-05T09:00:00Z",
            }]),
            json!([]),
        )
    );
}
