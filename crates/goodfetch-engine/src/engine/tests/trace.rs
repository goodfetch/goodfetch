//! Each request's span and its Trace context, over the in-memory network
//! (spec #213's Seam 3), read through an in-process collector of the spans
//! the OpenTelemetry SDK exports (ADR 0132): the dispatcher's `SERVER`
//! span, the child of the caller's span its `_meta` names by W3C Trace
//! Context's rules; the `CLIENT` span of a request the engine sends, whose
//! Trace context its `_meta` carries; a Run's own root, linked to the
//! request that started it; and nothing of it while no span is exported.
//!
//! Covers: story:3-146
#![expect(
    clippy::disallowed_types,
    reason = "the collector, which the OpenTelemetry SDK calls synchronously, keeps the exported spans under a lock no loom model needs (ADR 0060)"
)]

use std::future::ready;
use std::sync::{Arc, Mutex};

use goodfetch_protocol::catalog::{
    HandlerRequest, Operation, OperationClass, OperationKind, PROTOCOL_VERSION, Product, Reach,
    Stability, schema_of,
};
use goodfetch_protocol::jsonrpc::ProtocolFailure;
use goodfetch_test_support::TestEnv;
use opentelemetry::trace::{SpanId, SpanKind, TraceId, TracerProvider as _};
use opentelemetry::{KeyValue, Value as Attribute};
use opentelemetry_sdk::error::OTelSdkResult;
use opentelemetry_sdk::trace::{SdkTracerProvider, SpanData, SpanExporter};
use rstest::rstest;
use serde_json::{Value, json};
use tracing_subscriber::Layer as _;
use tracing_subscriber::layer::SubscriberExt as _;

use super::run::call;
use super::{attached_on, engine, engine_with, hello};
use crate::call_error::CallError;
use crate::composition::{CallContext, Contribution, Handler};
use crate::entropy::Entropy;
use crate::trace::EntropyIds;

mod forwarded;

/// The caller's span, in W3C Trace Context's example `traceparent`.
const TRACE_ID: &str = "0af7651916cd43dd8448eb211c80319c";
const PARENT_ID: &str = "b7ad6b7169203331";
const TRACEPARENT: &str = "00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01";

/// The handler request `test.ask` sends its Client.
const ASKED: HandlerRequest = HandlerRequest {
    name: "test.asked",
    description: "Asks the Client a question.",
    params: schema_of::<Value>,
    result: schema_of::<Value>,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: PROTOCOL_VERSION,
};

/// A call that sends its Client [`ASKED`] and answers its answer.
const ASK: Operation = Operation {
    name: "test.ask",
    description: "Asks the Client a question, and answers its answer.",
    class: OperationClass::Read,
    kind: OperationKind::RoundTrip,
    reach: Reach::AllSurfaces,
    params: schema_of::<Value>,
    result: schema_of::<Value>,
    events: &[],
    cancellable: true,
    input: None,
    bound: None,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: PROTOCOL_VERSION,
};

fn asking() -> Contribution {
    Contribution::new("test_ask").with_handler(Handler::new(
        &ASK,
        |_: Value, context: CallContext| async move {
            let mut sent = context
                .client()
                .send::<Value>(&ASKED, &json!({ "question": "why" }))
                .await
                .map_err(|error| {
                    CallError::protocol(ProtocolFailure::InternalError, error.to_string())
                })?;
            let answer = sent.answer().await.map_err(|error| {
                CallError::protocol(ProtocolFailure::InternalError, error.to_string())
            })?;
            Ok::<Value, CallError>(answer)
        },
    ))
}

/// The spans the SDK exports, each as it ends.
#[derive(Clone, Debug, Default)]
struct Collector(Arc<Mutex<Vec<SpanData>>>);

impl SpanExporter for Collector {
    fn export(
        &self,
        batch: Vec<SpanData>,
    ) -> impl std::future::Future<Output = OTelSdkResult> + Send {
        self.0.lock().unwrap().extend(batch);
        ready(Ok(()))
    }
}

/// Runs `talk` with every span exported to a collector, its IDs drawn from
/// an `Entropy` seeded with `seed`, and answers what it answered and the
/// spans the collector received.
fn exported<T>(seed: u64, talk: impl FnOnce() -> T) -> (T, Vec<SpanData>) {
    let collector = Collector::default();
    let provider = SdkTracerProvider::builder()
        .with_simple_exporter(collector.clone())
        .with_id_generator(EntropyIds::new(Entropy::seeded(seed)))
        .build();
    let subscriber = tracing_subscriber::registry().with(
        tracing_opentelemetry::layer()
            .with_tracer(provider.tracer("goodfetch"))
            .with_location(false)
            .with_threads(false)
            .with_filter(tracing_subscriber::filter::filter_fn(|metadata| {
                metadata.is_span()
            })),
    );
    let answered = tracing::subscriber::with_default(subscriber, talk);
    provider.shutdown().unwrap();
    let spans = std::mem::take(&mut *collector.0.lock().unwrap());
    (answered, spans)
}

/// The reason of each event of the Trace context's target, as its
/// subscriber sees it.
#[derive(Clone, Default)]
struct Reasons(Arc<Mutex<Vec<String>>>);

impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for Reasons {
    fn on_event(&self, event: &tracing::Event<'_>, _: tracing_subscriber::layer::Context<'_, S>) {
        struct Reason(Option<String>);
        impl tracing::field::Visit for Reason {
            fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
                if field.name() == "reason" {
                    self.0 = Some(value.to_owned());
                }
            }
            fn record_debug(&mut self, _: &tracing::field::Field, _: &dyn std::fmt::Debug) {}
        }
        if event.metadata().target() == crate::trace::TRACE_TARGET {
            let mut reason = Reason(None);
            event.record(&mut reason);
            self.0.lock().unwrap().push(reason.0.unwrap_or_default());
        }
    }
}

/// The reasons of the `debug` events `engine.info` under `meta` leaves,
/// with the spans exported or not.
fn dropped(exporting: bool, meta: Value) -> Vec<String> {
    let reasons = Reasons::default();
    let provider = SdkTracerProvider::builder().build();
    let export = exporting.then(|| {
        tracing_opentelemetry::layer()
            .with_tracer(provider.tracer("goodfetch"))
            .with_filter(tracing_subscriber::filter::filter_fn(|metadata| {
                metadata.is_span()
            }))
    });
    let subscriber = tracing_subscriber::registry()
        .with(export)
        .with(reasons.clone());
    tracing::subscriber::with_default(subscriber, || {
        replies(vec![traced(1, "engine.info", json!({}), meta)])
    });
    provider.shutdown().unwrap();
    std::mem::take(&mut *reasons.0.lock().unwrap())
}

/// A value a receiver drops leaves one `debug` event each, and none while
/// no span is exported, when the Trace context is not read at all.
#[test]
fn each_value_a_receiver_drops_leaves_one_debug_event_while_spans_are_exported() {
    let meta = json!({ "traceparent": "00-0af7", "tracestate": "congo=t61rcWkgMzE" });

    let (exporting, not) = (dropped(true, meta.clone()), dropped(false, meta));

    assert_eq!(
        (exporting, not),
        (
            vec![
                "traceparent_unreadable".to_owned(),
                "tracestate_without_parent".to_owned()
            ],
            vec![]
        )
    );
}

/// The request `method` with `params` and the Trace context `meta`.
fn traced(id: i64, method: &str, mut params: Value, meta: Value) -> Value {
    params["_meta"] = meta;
    json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params })
}

/// Opens an Attached engine with `engine.info`'s operations, says `hello`,
/// sends `requests` one at a time and answers each reply.
fn replies(requests: Vec<Value>) -> Vec<Value> {
    attached_on(
        &engine(Product::Collie),
        &TestEnv::new(),
        |mut client| async move {
            client.send(&[hello("collie", "0.1")]).await;
            client.next().await;
            let mut replies = Vec::new();
            for request in requests {
                let id = request["id"].as_i64().unwrap();
                let method = request["method"].as_str().unwrap().to_owned();
                replies.push(call(&mut client, id, &method, request["params"].clone()).await);
            }
            replies
        },
    )
}

/// The one exported span of `kind` named `name`.
fn the_span<'a>(spans: &'a [SpanData], kind: &SpanKind, name: &str) -> &'a SpanData {
    let found: Vec<&SpanData> = spans
        .iter()
        .filter(|span| span.span_kind == *kind && span.name == name)
        .collect();
    assert_eq!(found.len(), 1, "{kind:?} spans named {name}: {spans:#?}");
    found[0]
}

/// The span's attribute `key`, as a string.
fn attribute(span: &SpanData, key: &str) -> Option<String> {
    span.attributes
        .iter()
        .find(|attribute| attribute.key.as_str() == key)
        .map(|KeyValue { value, .. }| match value {
            Attribute::String(text) => text.to_string(),
            other => other.to_string(),
        })
}

fn trace_id() -> TraceId {
    TraceId::from_hex(TRACE_ID).unwrap()
}

#[test]
fn a_request_with_a_valid_traceparent_gets_a_server_span_that_is_its_child() {
    let (replies, spans) = exported(1, || {
        replies(vec![traced(
            1,
            "engine.info",
            json!({}),
            json!({ "traceparent": TRACEPARENT, "tracestate": "congo=t61rcWkgMzE" }),
        )])
    });

    assert!(replies[0].get("result").is_some(), "{replies:?}");
    let span = the_span(&spans, &SpanKind::Server, "engine.info");
    assert_eq!(
        (
            span.span_context.trace_id(),
            span.parent_span_id,
            span.parent_span_is_remote,
            span.span_context.trace_state().header(),
        ),
        (
            trace_id(),
            SpanId::from_hex(PARENT_ID).unwrap(),
            true,
            "congo=t61rcWkgMzE".to_owned()
        )
    );
    assert_eq!(
        ["rpc.system.name", "rpc.method", "jsonrpc.request.id"].map(|key| attribute(span, key)),
        [
            Some("jsonrpc".to_owned()),
            Some("engine.info".to_owned()),
            Some("1".to_owned())
        ]
    );
}

#[rstest]
#[case::none(json!({}))]
#[case::a_bad_version(json!({ "traceparent": "ff-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01" }))]
#[case::an_all_zero_trace_id(json!({ "traceparent": "00-00000000000000000000000000000000-b7ad6b7169203331-01" }))]
#[case::an_all_zero_parent_id(json!({ "traceparent": "00-0af7651916cd43dd8448eb211c80319c-0000000000000000-01" }))]
#[case::a_wrong_length(json!({ "traceparent": "00-0af7651916cd43dd8448eb211c80319c-b7ad6b716920333-01" }))]
#[case::over_the_length_bound(json!({ "traceparent": format!("cc-{TRACE_ID}-{PARENT_ID}-01-{}", "a".repeat(201)) }))]
#[case::a_tracestate_without_a_valid_traceparent(json!({ "traceparent": "00-0af7", "tracestate": "congo=t61rcWkgMzE" }))]
fn a_request_whose_traceparent_cannot_be_used_gets_a_new_root_and_answers(#[case] meta: Value) {
    let (replies, spans) = exported(1, || {
        replies(vec![traced(1, "engine.info", json!({}), meta)])
    });

    assert!(replies[0].get("result").is_some(), "{replies:?}");
    let span = the_span(&spans, &SpanKind::Server, "engine.info");
    assert_eq!(
        (
            span.parent_span_id,
            span.span_context.trace_id() == trace_id(),
            span.span_context.trace_state().header(),
        ),
        (SpanId::INVALID, false, String::new())
    );
}

#[test]
fn a_failed_requests_span_holds_its_error_code_and_json_rpc_code() {
    let (replies, spans) = exported(1, || {
        replies(vec![traced(
            1,
            "run.finish",
            json!({ "run_id": "none-such" }),
            json!({}),
        )])
    });

    let span = the_span(&spans, &SpanKind::Server, "run.finish");
    assert_eq!(
        ["error.type", "rpc.response.status_code", "run_id"].map(|key| attribute(span, key)),
        [
            replies[0]["error"]["data"]["code"]
                .as_str()
                .map(str::to_owned),
            Some(replies[0]["error"]["code"].to_string()),
            Some("none-such".to_owned()),
        ]
    );
    assert_eq!(replies[0]["error"]["data"]["code"], json!("run_not_found"));
}

#[rstest]
#[case::baggage(json!({ "traceparent": TRACEPARENT, "baggage": "user=a" }))]
#[case::a_number(json!({ "traceparent": 1 }))]
#[case::no_object(json!("00-0af7"))]
#[case::an_array(json!([]))]
#[case::null(json!(null))]
fn a_meta_with_an_unknown_key_or_a_value_that_is_no_string_is_refused(#[case] meta: Value) {
    let replies = replies(vec![traced(1, "engine.info", json!({}), meta)]);

    assert_eq!(replies[0]["error"]["data"]["code"], json!("invalid_params"));
}

/// Opens an Attached engine with `test.ask`, says `hello`, calls `test.ask`
/// under `meta` and answers the handler request it received.
fn asked(meta: Value) -> Value {
    let engine = engine_with(Product::Collie, Some(asking()));
    attached_on(&engine, &TestEnv::new(), |mut client| async move {
        client.send(&[hello("collie", "0.1")]).await;
        client.next().await;
        client.send(&[traced(1, "test.ask", json!({}), meta)]).await;
        let asked = client.next().await.unwrap();
        client
            .send(&[json!({ "jsonrpc": "2.0", "id": asked["id"], "result": { "because": "so" } })])
            .await;
        let reply = client.next().await.unwrap();
        assert_eq!(reply["result"], json!({ "because": "so" }));
        asked
    })
}

#[test]
fn a_request_the_engine_sends_carries_its_client_spans_trace_context() {
    let (asked, spans) = exported(1, || asked(json!({ "traceparent": TRACEPARENT })));

    let server = the_span(&spans, &SpanKind::Server, "test.ask");
    let client = the_span(&spans, &SpanKind::Client, "test.asked");
    assert_eq!(
        asked["params"],
        json!({
            "question": "why",
            "_meta": {
                "traceparent": format!(
                    "00-{TRACE_ID}-{}-01",
                    client.span_context.span_id()
                ),
            },
        })
    );
    assert_eq!(
        (client.parent_span_id, client.span_context.trace_id()),
        (server.span_context.span_id(), trace_id())
    );
    assert_eq!(
        ["rpc.method", "jsonrpc.request.id"].map(|key| attribute(client, key)),
        [Some("test.asked".to_owned()), Some(asked["id"].to_string())]
    );
}

#[test]
fn with_no_span_exported_a_traceparent_received_is_ignored_and_none_is_sent_on() {
    let asked = asked(json!({ "traceparent": TRACEPARENT }));

    assert_eq!(asked["params"], json!({ "question": "why" }));
}

#[test]
fn a_run_is_its_own_root_linked_to_the_request_that_started_it() {
    let (_, spans) = exported(1, || {
        replies(vec![
            traced(
                1,
                "run.start",
                json!({ "run_id": "one" }),
                json!({ "traceparent": TRACEPARENT }),
            ),
            traced(2, "run.finish", json!({ "run_id": "one" }), json!({})),
        ])
    });

    let start = the_span(&spans, &SpanKind::Server, "run.start");
    let run = the_span(&spans, &SpanKind::Internal, "run");
    let links: Vec<(TraceId, SpanId)> = run
        .links
        .iter()
        .map(|link| (link.span_context.trace_id(), link.span_context.span_id()))
        .collect();
    assert_eq!(
        (
            run.parent_span_id,
            run.span_context.trace_id() == trace_id()
        ),
        (SpanId::INVALID, false)
    );
    assert_eq!(
        links,
        [(start.span_context.trace_id(), start.span_context.span_id())]
    );
}

#[test]
fn a_run_resumed_from_its_checkpoint_is_a_new_root_linked_to_the_resuming_request() {
    let resuming = "00-1af7651916cd43dd8448eb211c80319c-c7ad6b7169203331-01";
    let (replies, spans) = exported(1, || {
        replies(vec![
            traced(1, "run.start", json!({ "run_id": "one" }), json!({})),
            traced(2, "run.pause", json!({ "run_id": "one" }), json!({})),
            traced(
                3,
                "run.resume",
                json!({ "run_id": "one" }),
                json!({ "traceparent": resuming }),
            ),
            traced(4, "run.finish", json!({ "run_id": "one" }), json!({})),
        ])
    });

    assert_eq!(replies[2]["result"]["started_as"], json!("resumed"));
    let start = the_span(&spans, &SpanKind::Server, "run.start");
    let resume = the_span(&spans, &SpanKind::Server, "run.resume");
    let runs: Vec<(SpanId, Vec<SpanId>)> = spans
        .iter()
        .filter(|span| span.name == "run")
        .map(|run| {
            (
                run.parent_span_id,
                run.links
                    .iter()
                    .map(|link| link.span_context.span_id())
                    .collect(),
            )
        })
        .collect();
    assert_eq!(
        runs,
        [
            (SpanId::INVALID, vec![start.span_context.span_id()]),
            (SpanId::INVALID, vec![resume.span_context.span_id()]),
        ]
    );
    assert_eq!(
        resume.span_context.trace_id(),
        TraceId::from_hex("1af7651916cd43dd8448eb211c80319c").unwrap()
    );
}

/// A Secret canary a Client writes into its requests.
const CANARY: &str = "GFCANARY-hunter2";

/// The fields a span of each name may hold: IDs, the operation's name,
/// Error codes and values of bounded sets (ADR 0132; STYLE.md).
fn allowed_fields(name: &str, kind: &SpanKind) -> &'static [&'static str] {
    match (kind, name) {
        (SpanKind::Server | SpanKind::Client, _) => &[
            "rpc.system.name",
            "rpc.method",
            "jsonrpc.request.id",
            "run_id",
            "session_id",
            "error.type",
            "rpc.response.status_code",
            "target",
            "busy_ns",
            "idle_ns",
        ],
        (_, "run") => &["run_id", "target", "busy_ns", "idle_ns"],
        _ => &[],
    }
}

/// Every request a Client sends and the engine sends opens one span of the
/// dispatcher's or the RPC layer's, whose fields hold no params, no URL
/// and no value of an unbounded set, such as the text of a request's id.
#[test]
fn a_request_span_holds_only_bounded_values_and_the_dispatcher_opens_one_per_request() {
    let url = format!("https://example.test/{CANARY}/path?q={CANARY}");
    let (asked, spans) = exported(1, || {
        let engine = engine_with(Product::Collie, Some(asking()));
        attached_on(&engine, &TestEnv::new(), |mut client| async move {
            client.send(&[hello("collie", "0.1")]).await;
            client.next().await;
            let refused = call(
                &mut client,
                1,
                "run.start",
                json!({ "run_id": CANARY, "_meta": { "traceparent": TRACEPARENT } }),
            )
            .await;
            assert_eq!(refused["error"]["data"]["code"], json!("run_id_invalid"));
            client
                .send(&[json!({
                    "jsonrpc": "2.0",
                    "id": CANARY,
                    "method": "test.ask",
                    "params": { "url": url, "text": CANARY },
                })])
                .await;
            let asked = client.next().await.unwrap();
            client
                .send(&[json!({
                    "jsonrpc": "2.0",
                    "id": asked["id"],
                    "error": {
                        "code": -32000,
                        "message": CANARY,
                        "data": { "code": CANARY, "retryable": false, "url": url },
                    },
                })])
                .await;
            client.next().await.unwrap();
            asked
        })
    });

    assert!(asked["method"] == json!("test.asked"), "{asked}");
    for (kind, name) in [
        (SpanKind::Server, "hello"),
        (SpanKind::Server, "run.start"),
        (SpanKind::Server, "test.ask"),
        (SpanKind::Client, "test.asked"),
    ] {
        the_span(&spans, &kind, name);
    }
    for span in &spans {
        let allowed = allowed_fields(&span.name, &span.span_kind);
        for KeyValue { key, value, .. } in &span.attributes {
            assert!(
                allowed.contains(&key.as_str()),
                "{} holds `{key}`",
                span.name
            );
            let value = value.to_string();
            assert!(
                !value.contains("GFCANARY") && !value.contains("example.test"),
                "{} holds `{key}` = {value}",
                span.name
            );
        }
    }
}

#[test]
fn two_runs_of_one_seed_export_the_same_trace_and_span_ids() {
    let ids = || {
        let (_, spans) = exported(7, || {
            replies(vec![
                traced(1, "engine.info", json!({}), json!({})),
                traced(2, "engine.info", json!({}), json!({})),
            ])
        });
        spans
            .iter()
            .map(|span| (span.span_context.trace_id(), span.span_context.span_id()))
            .collect::<Vec<_>>()
    };

    let (first, second) = (ids(), ids());

    assert_eq!(first.len(), 3);
    assert_eq!(first, second);
}
