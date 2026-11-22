//! The program's Trace context without the `opentelemetry` feature (ADR
//! 0132): a call made in a `tracing` span that `tracing-opentelemetry`
//! records sends no `_meta`, so the linked engine's span of it is a root,
//! in a trace of its own. Every other build of the crate's tests takes the
//! feature, so this module is built only by `just feature-combinations`,
//! which builds them with `--cfg goodfetch_without_opentelemetry` and runs
//! it. `trace_context.rs`'s `linked` test, with the feature, is its twin.
//! It declares no Feature: no suite whose results decide one runs it
//! (TESTING.md, Feature coverage), and the `trace-context` case's tests
//! declare story 11-138.

use opentelemetry::trace::{SpanId, SpanKind};
use tracing::Instrument as _;

use super::exported_spans::{linked_spans, the_span};

/// The engine's span of a call made in the program's span is a root.
#[test]
fn a_call_in_a_span_sends_no_trace_context_so_the_linked_engines_span_is_a_root() {
    let spans = linked_spans(async |collie| {
        async { collie.explain_settings().await.unwrap() }
            .instrument(tracing::info_span!("program"))
            .await;
    });

    let program = the_span(&spans, "program").span_context.clone();
    let call = the_span(&spans, "config.explain");
    assert_eq!(
        (call.span_kind.clone(), call.parent_span_id),
        (SpanKind::Server, SpanId::INVALID),
    );
    assert_ne!(call.span_context.trace_id(), program.trace_id());
}
