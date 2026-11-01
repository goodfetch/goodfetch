//! Trace context and the request spans (ADR 0132; RT-18). The RPC layer
//! opens one `SERVER` span per request it receives and one `CLIENT` span
//! per request it sends, each named by its operation, with fields of
//! bounded values only: OpenTelemetry's RPC and JSON-RPC conventions, the
//! reply's Error code and the glossary IDs its params hold. A request's
//! `_meta` carries its caller's Trace context, read by W3C Trace Context's
//! rules, which drop a value they cannot use and never fail the call; a
//! sent request carries its `CLIENT` span's. All of it only while the
//! process exports its spans: without an OpenTelemetry layer the received
//! Trace context is ignored and none is sent. A Run is a root of its own,
//! linked to the request that started or resumed it. Trace and span IDs
//! come from the [`Entropy`](crate::entropy::Entropy) ([`EntropyIds`]), so a seeded test exports
//! the same IDs every run (ADRs 0059 and 0060).

mod ids;

use std::str::FromStr as _;

use goodfetch_protocol::jsonrpc::{RequestId, TraceContext};
use goodfetch_protocol::run::RunId;
use opentelemetry::Context;
use opentelemetry::trace::{
    SpanContext, SpanId, TraceContextExt as _, TraceFlags, TraceId, TraceState,
};
use serde_json::Value;
use tracing::Span;
use tracing::field::Empty;
use tracing_opentelemetry::OpenTelemetrySpanExt as _;

pub use ids::EntropyIds;

/// The `tracing` target of the request spans and of the `debug` events of
/// a Trace context's dropped values.
pub const TRACE_TARGET: &str = "goodfetch::rpc::trace";

/// The params members whose glossary IDs a request span records.
const ID_FIELDS: [&str; 2] = ["run_id", "session_id"];

/// The longest Error code a `CLIENT` span records of a reply.
const MAX_ERROR_CODE: usize = 64;

/// The longest `traceparent` a receiver reads (ADR 0132).
const MAX_TRACEPARENT: usize = 256;

/// The longest `tracestate` a receiver keeps (ADR 0132), the least W3C
/// Trace Context has a vendor pass on.
const MAX_TRACESTATE: usize = 512;

/// The length of a version-00 `traceparent`, which a later version's
/// starts with.
const TRACEPARENT_LENGTH: usize = 55;

/// What a receiver keeps of a request's Trace context.
#[derive(Debug)]
pub(crate) struct Received {
    /// The caller's span, remote, with its `tracestate` when that was
    /// kept; none when the request starts a new trace.
    pub(crate) parent: Option<SpanContext>,
    /// What it dropped, each worth one `debug` event.
    pub(crate) dropped: Vec<Dropped>,
}

/// A value of a Trace context its receiver dropped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Dropped {
    /// A `traceparent` longer than 256 characters.
    TraceparentTooLong,
    /// A `traceparent` W3C Trace Context's rules do not read.
    TraceparentUnreadable,
    /// A `tracestate` longer than 512 characters.
    TracestateTooLong,
    /// A `tracestate` W3C Trace Context's rules do not read.
    TracestateUnreadable,
    /// A `tracestate` with no valid `traceparent`.
    TracestateWithoutParent,
}

impl Dropped {
    /// What the `debug` event says of it.
    pub(crate) fn reason(self) -> &'static str {
        match self {
            Self::TraceparentTooLong => "traceparent_too_long",
            Self::TraceparentUnreadable => "traceparent_unreadable",
            Self::TracestateTooLong => "tracestate_too_long",
            Self::TracestateUnreadable => "tracestate_unreadable",
            Self::TracestateWithoutParent => "tracestate_without_parent",
        }
    }
}

/// What a receiver keeps of `context`: a `traceparent` it cannot read, or
/// longer than 256 characters, starts a new trace and drops `tracestate`;
/// a `tracestate` longer than 512 characters, unreadable, or without a
/// valid `traceparent` is dropped, a valid `traceparent` kept.
pub(crate) fn receive(context: &TraceContext) -> Received {
    let mut dropped = Vec::new();
    let parent = context.traceparent.as_deref().and_then(|traceparent| {
        if traceparent.len() > MAX_TRACEPARENT {
            dropped.push(Dropped::TraceparentTooLong);
            return None;
        }
        let parent = traceparent_of(traceparent);
        if parent.is_none() {
            dropped.push(Dropped::TraceparentUnreadable);
        }
        parent
    });
    let state = match (&parent, context.tracestate.as_deref()) {
        (_, None) => TraceState::NONE,
        (None, Some(_)) => {
            dropped.push(Dropped::TracestateWithoutParent);
            TraceState::NONE
        }
        (Some(_), Some(tracestate)) if tracestate.len() > MAX_TRACESTATE => {
            dropped.push(Dropped::TracestateTooLong);
            TraceState::NONE
        }
        (Some(_), Some(tracestate)) => TraceState::from_str(tracestate).unwrap_or_else(|_| {
            dropped.push(Dropped::TracestateUnreadable);
            TraceState::NONE
        }),
    };
    let parent = parent
        .map(|(trace_id, span_id, flags)| SpanContext::new(trace_id, span_id, flags, true, state));
    Received { parent, dropped }
}

/// The trace ID, parent ID and flags of `traceparent`, by W3C Trace
/// Context's rules: lowercase hex fields, a version other than `ff`, IDs
/// other than all zeros, exactly 55 characters at version `00`, and at a
/// later version a `-` after the four fields when more follows.
fn traceparent_of(traceparent: &str) -> Option<(TraceId, SpanId, TraceFlags)> {
    let lowercase_hex = |field: &str, length: usize| {
        field.len() == length
            && field
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    };
    let head = traceparent.get(..TRACEPARENT_LENGTH)?;
    let rest = &traceparent[TRACEPARENT_LENGTH..];
    let mut fields = head.split('-');
    let (version, trace_id, parent_id, flags) = (
        fields.next()?,
        fields.next()?,
        fields.next()?,
        fields.next()?,
    );
    let well_formed = fields.next().is_none()
        && lowercase_hex(version, 2)
        && version != "ff"
        && lowercase_hex(trace_id, 32)
        && lowercase_hex(parent_id, 16)
        && lowercase_hex(flags, 2)
        && if version == "00" {
            rest.is_empty()
        } else {
            rest.is_empty() || rest.starts_with('-')
        };
    if !well_formed {
        return None;
    }
    let trace_id = TraceId::from_hex(trace_id).ok()?;
    let span_id = SpanId::from_hex(parent_id).ok()?;
    let flags = u8::from_str_radix(flags, 16).ok()?;
    (trace_id != TraceId::INVALID && span_id != SpanId::INVALID).then_some((
        trace_id,
        span_id,
        TraceFlags::new(flags),
    ))
}

/// The `SERVER` span of the request `id` the process received, of the
/// operation `method`: a new root, or the child of the caller's span its
/// Trace context `meta` names while the process exports its spans, with
/// the Run ID and Session ID `params` hold. Only the RPC layer's dispatcher
/// opens one.
pub(crate) fn server_span(
    method: &'static str,
    id: &RequestId,
    params: Option<&Value>,
    meta: Option<&TraceContext>,
) -> Span {
    let span = tracing::debug_span!(
        target: TRACE_TARGET,
        parent: None,
        "request",
        otel.name = method,
        otel.kind = "server",
        rpc.system.name = "jsonrpc",
        rpc.method = method,
        jsonrpc.request.id = Empty,
        run_id = Empty,
        session_id = Empty,
        "error.type" = Empty,
        rpc.response.status_code = Empty,
    );
    record_id(&span, id);
    if let Some(Value::Object(params)) = params {
        for field in ID_FIELDS {
            if let Some(value) = params.get(field).and_then(Value::as_str)
                && RunId::parse(value).is_ok()
            {
                span.record(field, value);
            }
        }
    }
    // A span no OpenTelemetry layer takes has no parent to set: the
    // process exports no spans, and ignores the Trace context it received.
    let exported = span.set_parent(Context::new()).is_ok();
    if let (true, Some(meta)) = (exported, meta) {
        let received = receive(meta);
        for dropped in received.dropped {
            tracing::debug!(
                target: TRACE_TARGET,
                parent: &span,
                reason = dropped.reason(),
                "a request's Trace context held a value it cannot use, which is dropped"
            );
        }
        if let Some(parent) = received.parent {
            let _ = span.set_parent(Context::new().with_remote_span_context(parent));
        }
    }
    span
}

/// The `CLIENT` span of a request of `method` the process sends, the child
/// of the current span. Only the RPC layer opens one.
pub(crate) fn client_span(method: &'static str) -> Span {
    tracing::debug_span!(
        target: TRACE_TARGET,
        "request",
        otel.name = method,
        otel.kind = "client",
        rpc.system.name = "jsonrpc",
        rpc.method = method,
        jsonrpc.request.id = Empty,
        "error.type" = Empty,
        rpc.response.status_code = Empty,
    )
}

/// The Trace context a request sent under `span` carries in its `_meta`:
/// none while the process exports no spans.
pub(crate) fn sent(span: &Span) -> Option<TraceContext> {
    let context = span.context();
    let sent = context.span().span_context().clone();
    sent.is_valid().then(|| {
        let tracestate = sent.trace_state().header();
        TraceContext {
            traceparent: Some(format!(
                "00-{}-{}-{:02x}",
                sent.trace_id(),
                sent.span_id(),
                sent.trace_flags().to_u8()
            )),
            tracestate: (!tracestate.is_empty()).then_some(tracestate),
        }
    })
}

/// Records on a request's `span` the id it went under, when that is a
/// number: a text id is its sender's choice of any length and content,
/// which a span never holds.
pub(crate) fn record_id(span: &Span, id: &RequestId) {
    if let RequestId::Number(number) = id {
        span.record("jsonrpc.request.id", tracing::field::display(number));
    }
}

/// Records on a request's `span` the failure of its reply: its Error code
/// and its JSON-RPC numeric code. A peer's code that is no Error code's
/// shape, lowercase words joined by `_`, is not recorded.
pub(crate) fn record_failure(span: &Span, error_code: &str, numeric_code: i32) {
    let shaped = !error_code.is_empty()
        && error_code.len() <= MAX_ERROR_CODE
        && error_code
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_');
    if shaped {
        span.record("error.type", error_code);
    }
    span.record("rpc.response.status_code", numeric_code);
}

/// The request span a Run's span links to: the one current when it is
/// taken, while the process exports its spans.
#[derive(Clone, Debug, Default)]
pub(crate) struct Link(Option<SpanContext>);

impl Link {
    /// The current span, which is the request's span in a handler.
    pub(crate) fn to_current() -> Self {
        let context = Span::current().context();
        let current = context.span().span_context().clone();
        Self(current.is_valid().then_some(current))
    }

    /// Adds the link to `span`, a new root.
    pub(crate) fn add_to(&self, span: &Span) {
        if let Some(linked) = &self.0 {
            span.add_link(linked.clone());
        }
    }
}

#[cfg(test)]
mod tests;
