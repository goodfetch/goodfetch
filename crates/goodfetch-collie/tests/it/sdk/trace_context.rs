//! The program's Trace context in each request (ADR 0132), the
//! `trace-context` client conformance case: with the `opentelemetry`
//! feature, a call made inside a `tracing` span that `tracing-opentelemetry`
//! records sends that span's `traceparent`, and its `tracestate`, in its
//! `_meta`, a raw call's beside its raw mark, and never the Baggage the
//! program set; a call made outside any span sends no `_meta`. The
//! Scripted engine, reached as a Daemon, fails on params it does not
//! expect and on a step left unused, so its clean exit says each call
//! carried exactly that. The blocking API reads the calling thread's span,
//! though its call runs on the Connection's runtime.
//! Covers: story:11-138

use goodfetch_collie::{Collie, EngineChoice, blocking};
use goodfetch_test_support::{SCRIPTED_DAEMON_TOKEN, TestEnv, real_time_runtime};
use opentelemetry::baggage::BaggageExt as _;
use opentelemetry::trace::{
    SpanContext, SpanId, TraceContextExt as _, TraceFlags, TraceId, TraceState, TracerProvider as _,
};
use opentelemetry::{Context, KeyValue};
use opentelemetry_sdk::trace::{IdGenerator, SdkTracerProvider};
use serde_json::json;
use tracing::Instrument as _;
use tracing_opentelemetry::OpenTelemetrySpanExt as _;
use tracing_subscriber::layer::SubscriberExt as _;

use super::engine_choice::{hello, played_whole, scripted};
use super::options;

/// The case's script.
const SCRIPT: &str = "sdk-rust/connect/trace-context.json";

/// The program's span's own ID, which every span the test's tracer opens
/// takes, as the script expects it.
const SPAN_ID: u64 = 0x00f0_67aa_0ba9_02b7;

/// Gives every span [`SPAN_ID`], so that the `traceparent` sent is known.
#[derive(Debug)]
struct FixedIds;

impl IdGenerator for FixedIds {
    fn new_trace_id(&self) -> TraceId {
        TraceId::from(0x0af7_6519_16cd_43dd_8448_eb21_1c80_319c_u128)
    }

    fn new_span_id(&self) -> SpanId {
        SpanId::from(SPAN_ID)
    }
}

/// A subscriber that records every span through `tracing-opentelemetry`,
/// with the tracer that gives [`FixedIds`].
fn traced() -> impl tracing::Subscriber + Send + Sync {
    let provider = SdkTracerProvider::builder()
        .with_id_generator(FixedIds)
        .build();
    tracing_subscriber::registry()
        .with(tracing_opentelemetry::layer().with_tracer(provider.tracer("program")))
}

/// The program's span, whose parent is a remote span with a `tracestate`,
/// in a context holding Baggage, which no request carries.
fn program_span() -> tracing::Span {
    let parent = SpanContext::new(
        TraceId::from(0x0af7_6519_16cd_43dd_8448_eb21_1c80_319c_u128),
        SpanId::from(0xb7ad_6b71_6920_3331_u64),
        TraceFlags::SAMPLED,
        true,
        TraceState::from_key_value([("vendor", "value")]).unwrap(),
    );
    let span = tracing::info_span!("program");
    span.set_parent(
        Context::new()
            .with_remote_span_context(parent)
            .with_baggage([KeyValue::new("user", "secret")]),
    )
    .unwrap();
    span
}

/// The connection's options: the Daemon of `env`'s home, which the
/// Scripted engine is.
fn daemon(env: &TestEnv) -> goodfetch_collie::ConnectOptions {
    options(env).engine(EngineChoice::Daemon)
}

/// The case through the async API, its calls awaited in an instrumented
/// future, as a `#[tracing::instrument]` function awaits them.
/// Covers: story:11-138
#[test]
fn a_call_in_a_span_carries_its_trace_context_and_one_outside_none() {
    tracing::subscriber::with_default(traced(), || {
        let env = TestEnv::new();
        let (process, _line) = scripted(&env, SCRIPT, &hello(SCRIPTED_DAEMON_TOKEN), "socket");

        real_time_runtime().block_on(async {
            let collie = Collie::connect(daemon(&env)).await.unwrap();
            async {
                collie.explain_settings().await.unwrap();
                collie
                    .call("run.list")
                    .params(json!({ "lifecycle": "running" }))
                    .await
                    .unwrap();
            }
            .instrument(program_span())
            .await;
            collie.explain_settings().await.unwrap();
            collie.close().await.unwrap();
        });

        played_whole(&process);
    });
}

/// The same case through `blocking`, whose calls run on the Connection's
/// runtime but carry the span the calling thread is in.
/// Covers: story:11-138
#[test]
fn a_blocking_call_carries_its_threads_trace_context() {
    tracing::subscriber::with_default(traced(), || {
        let env = TestEnv::new();
        let (process, _line) = scripted(&env, SCRIPT, &hello(SCRIPTED_DAEMON_TOKEN), "socket");

        let collie = blocking::Collie::connect(daemon(&env)).unwrap();
        program_span().in_scope(|| {
            collie.explain_settings().send().unwrap();
            collie
                .call("run.list")
                .params(json!({ "lifecycle": "running" }))
                .send()
                .unwrap();
        });
        collie.explain_settings().send().unwrap();
        collie.close().unwrap();

        played_whole(&process);
    });
}
