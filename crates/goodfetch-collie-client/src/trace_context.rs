//! The Trace context each request carries (ADR 0132): the host program's
//! current OpenTelemetry context, which a [`TraceContextSource`] reads, so
//! that the program's trace continues in the engine. This crate depends on
//! no OpenTelemetry crate: the SDK crates implement the source behind
//! their `opentelemetry` Cargo feature, and a Connection opened without
//! one sends no Trace context. The source is read on the thread that makes
//! the call, as the call becomes a future; a blocking call, which runs on
//! the Connection's runtime, carries the Trace context its caller's thread
//! had when it handed the call over, every request it makes included. Only
//! `traceparent` and `tracestate` are sent, never Baggage, in the request's
//! `_meta`, which no public type names. The Connection's own Subscription,
//! shared by every stream of the events it publishes, belongs to no one
//! caller, so its request carries none.

use std::fmt::Debug;
use std::future::Future;

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

tokio::task_local! {
    /// The Trace context of the thread that handed a blocking call to the
    /// Connection's runtime, whose worker has none of its own.
    static CALLERS: Option<TraceContext>;
}

/// The Trace context a call made now carries: its blocking caller's, inside
/// [`as_callers`], and otherwise what `source` reads on this thread.
pub(crate) fn current(source: Option<&dyn TraceContextSource>) -> Option<TraceContext> {
    CALLERS
        .try_with(Clone::clone)
        .unwrap_or_else(|_| source.and_then(TraceContextSource::current))
}

/// `future`, every call of which carries `callers`, the Trace context of
/// the thread that blocks on it.
pub(crate) fn as_callers<F: Future>(
    callers: Option<TraceContext>,
    future: F,
) -> impl Future<Output = F::Output> {
    CALLERS.scope(callers, future)
}

/// Writes `context` into `params`' `_meta`, beside what it already holds,
/// such as a raw call's mark; no context, or one with no `traceparent`,
/// writes nothing, since a `tracestate` alone means nothing to a receiver.
pub(crate) fn carry(params: &mut Value, context: Option<TraceContext>) {
    let Some(context) = context.filter(|context| context.traceparent.is_some()) else {
        return;
    };
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
