//! The Trace context each request carries (ADR 0132): the host program's
//! current OpenTelemetry context, which a [`TraceContextSource`] reads, so
//! that the program's trace continues in the engine. This crate depends on
//! no OpenTelemetry crate: the SDK crates implement the source behind
//! their `opentelemetry` Cargo feature, and a Connection opened without
//! one sends no Trace context. The source is read on the thread that makes
//! the call, as the call becomes a future, so a blocking call, which runs
//! on the Connection's runtime, carries its own thread's context. Only
//! `traceparent` and `tracestate` are sent, never Baggage, in the request's
//! `_meta`, which no public type names.

use std::fmt::Debug;

use goodfetch_protocol::jsonrpc::META;
pub use goodfetch_protocol::jsonrpc::TraceContext;
use serde_json::{Map, Value};

/// Reads the host program's current Trace context, which each request the
/// Connection sends carries (ADR 0132).
pub trait TraceContextSource: Debug + Send + Sync + 'static {
    /// The calling thread's current Trace context, or `None` when the
    /// program has none, such as outside any span.
    fn current(&self) -> Option<TraceContext>;
}

/// Writes `context` into `params`' `_meta`, beside what it already holds,
/// such as a raw call's mark; a context with no `traceparent` writes
/// nothing, since a `tracestate` alone means nothing to a receiver.
pub(crate) fn carry(params: &mut Value, context: TraceContext) {
    if context.traceparent.is_none() {
        return;
    }
    if params.is_null() {
        *params = Value::Object(Map::new());
    }
    let Value::Object(members) = params else {
        return;
    };
    let meta = members
        .entry(META)
        .or_insert_with(|| Value::Object(Map::new()));
    let (Value::Object(meta), Value::Object(context)) = (
        meta,
        serde_json::to_value(context).expect("invariant: a Trace context serializes"),
    ) else {
        return;
    };
    meta.extend(context);
}

#[cfg(test)]
mod tests;
