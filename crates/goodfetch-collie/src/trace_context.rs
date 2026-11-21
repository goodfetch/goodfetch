//! The Trace context each request of the Rust SDK carries (ADR 0132): with
//! the `opentelemetry` feature, the OpenTelemetry context of the program's
//! current `tracing` span, which `tracing-opentelemetry` gives, read on the
//! thread that makes the call; without the feature, none. Only
//! `traceparent` and `tracestate` are read, never Baggage.

use std::sync::Arc;

use goodfetch_collie_client::TraceContextSource;

/// What reads the Trace context of each connection the SDK opens, Foxhound's
/// included: the current `tracing` span's with the `opentelemetry` feature,
/// and nothing without it.
pub(crate) fn source() -> Option<Arc<dyn TraceContextSource>> {
    #[cfg(feature = "opentelemetry")]
    {
        Some(Arc::new(CurrentSpan))
    }
    #[cfg(not(feature = "opentelemetry"))]
    {
        None
    }
}

/// Reads the OpenTelemetry context of the calling thread's current
/// `tracing` span.
#[cfg(feature = "opentelemetry")]
#[derive(Debug)]
struct CurrentSpan;

#[cfg(feature = "opentelemetry")]
impl TraceContextSource for CurrentSpan {
    /// The current span's `traceparent`, and its `tracestate` when it has
    /// one, as W3C Trace Context writes them; none outside a span that
    /// `tracing-opentelemetry` records.
    fn current(&self) -> Option<goodfetch_collie_client::TraceContext> {
        use opentelemetry::trace::TraceContextExt as _;
        use tracing_opentelemetry::OpenTelemetrySpanExt as _;

        let context = tracing::Span::current().context();
        let span = context.span();
        let sent = span.span_context();
        sent.is_valid().then(|| {
            let tracestate = sent.trace_state().header();
            goodfetch_collie_client::TraceContext {
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
}
