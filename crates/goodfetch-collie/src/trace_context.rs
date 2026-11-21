//! The Trace context each request of the Rust SDK carries (ADR 0132): with
//! the `opentelemetry` feature, the OpenTelemetry context of the program's
//! current `tracing` span, which `tracing-opentelemetry` gives, read on the
//! thread that makes the call; without the feature, none. Only
//! `traceparent` and `tracestate` are read, never Baggage.

use std::sync::Arc;

use goodfetch_collie_client::{TraceContext, TraceContextSource};

/// What reads the Trace context of each connection the SDK opens, Foxhound's
/// included: the current `tracing` span's with the `opentelemetry` feature,
/// and nothing without it.
pub(crate) fn source() -> Option<Arc<dyn TraceContextSource>> {
    cfg!(feature = "opentelemetry").then(|| Arc::new(CurrentSpan) as Arc<dyn TraceContextSource>)
}

/// Reads the OpenTelemetry context of the calling thread's current
/// `tracing` span.
#[derive(Debug)]
struct CurrentSpan;

impl TraceContextSource for CurrentSpan {
    /// The current span's `traceparent`, and its `tracestate` when it has
    /// one, as the engine writes them for the requests it sends; none
    /// outside a span that `tracing-opentelemetry` records.
    fn current(&self) -> Option<TraceContext> {
        goodfetch_engine::trace::sent(&tracing::Span::current())
    }
}
