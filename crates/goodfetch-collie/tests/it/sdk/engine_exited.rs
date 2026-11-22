//! The `engine-exited-carries-its-evidence` client conformance case,
//! linked: the linked engine ending before its Connection is closed fails
//! the call it leaves unanswered with `engine_exited`, whose details hold
//! no exit code, signal or stderr, the engine being the program's own
//! process (ADR 0078). The engine ends as an Attached engine does when its
//! Connection's serving task panics (ADR 0148), from an operation the test
//! adds to Collie's composition. The case's Attached part, its evidence
//! and its stderr logged, is `goodfetch-collie-client`'s
//! `an_attached_engine_that_dies_during_a_call_fails_it_with_engine_exited_and_its_evidence`.

use goodfetch_collie::linked::{Composed, ProductConnection};
use goodfetch_collie::{EngineChoice, ErrorCode};
use goodfetch_conformance::{ClientCaseRun, RustMode};
use goodfetch_engine::{CallContext, CallError, Contribution, Handler};
use goodfetch_protocol::catalog::{
    ErrorGroup, Operation, OperationClass, OperationKind, PROTOCOL_VERSION, Product, Reach,
    Stability, schema_of,
};
use goodfetch_protocol::connection::EngineExitedDetails;
use goodfetch_test_support::{TestEnv, case_logs, real_time_runtime};
use serde_json::{Value, json};

use super::options;

/// An operation whose handler panics as it arrives, in the Connection's
/// serving task, which ends the Connection and the linked engine with it.
const PANIC_ON_ARRIVAL: Operation = Operation {
    name: "test.panic_on_arrival",
    description: "Panics as it arrives.",
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

/// The part of `test.panic_on_arrival` that runs as it arrives.
fn arrives(_: Value, _: CallContext) -> std::future::Ready<Result<Value, CallError>> {
    panic!("the test ends the linked engine")
}

/// Collie's engine with `test.panic_on_arrival`.
const ENDING: Composed = Composed {
    product: Product::Collie,
    composition: || {
        goodfetch_collie::composition().with(
            Contribution::new("test_panic_on_arrival")
                .with_handler(Handler::new(&PANIC_ON_ARRIVAL, arrives)),
        )
    },
};

/// Covers: story:11-24, error:engine_exited
#[test]
fn the_linked_engine_ending_fails_the_call_it_leaves_unanswered_with_engine_exited() {
    let _case = ClientCaseRun::start(
        case_logs().as_deref(),
        "engine-exited-carries-its-evidence",
        RustMode::Linked,
    )
    .unwrap();
    let env = TestEnv::new();
    let failed = real_time_runtime().block_on(async {
        let connection =
            ProductConnection::connect(ENDING, options(&env).engine(EngineChoice::Linked))
                .await
                .unwrap();
        let answered = connection
            .connection()
            .call::<Value>("test.panic_on_arrival", &json!({}))
            .await;
        let Err(error) = answered else {
            panic!("{answered:?}");
        };
        error
    });

    assert_eq!(
        (failed.code(), failed.group(), failed.retryable()),
        (ErrorCode::ENGINE_EXITED, ErrorGroup::Unavailable, false)
    );
    assert_eq!(
        failed.details::<EngineExitedDetails>(),
        Some(EngineExitedDetails::without_evidence())
    );
    assert_eq!(
        (failed.operation(), failed.message()),
        (
            Some("test.panic_on_arrival"),
            "test.panic_on_arrival: the engine ended before its Connection was closed"
        )
    );
}
