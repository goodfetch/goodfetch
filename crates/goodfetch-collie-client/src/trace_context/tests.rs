//! Where a request's Trace context goes in its params (ADR 0132): params
//! that are null become an object holding it, and a `tracestate` without
//! a `traceparent` writes nothing. A raw call's mark beside it is the
//! `trace-context` client case's, in the SDK crates' tests.
//! Covers: story:11-138

use serde_json::{Value, json};

use super::{TraceContext, carry};

#[test]
fn null_params_become_an_object_holding_the_context() {
    let mut params = Value::Null;
    carry(
        &mut params,
        TraceContext {
            traceparent: Some("00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01".to_owned()),
            tracestate: None,
        },
    );
    assert_eq!(
        params,
        json!({ "_meta": { "traceparent": "00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01" } })
    );
}

#[test]
fn a_tracestate_without_a_traceparent_writes_nothing() {
    let mut params = json!({ "keys": [] });
    carry(
        &mut params,
        TraceContext {
            traceparent: None,
            tracestate: Some("vendor=value".to_owned()),
        },
    );
    assert_eq!(params, json!({ "keys": [] }));
}
