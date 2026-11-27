//! The Daemon's HTTP JSON-RPC listener (`POST /v1/rpc`, ADRs 0027, 0049,
//! 0067, 0099 and 0132), tested in-process over the in-memory network:
//! starts a Run that outlives the HTTP request, enforces Bearer token,
//! `Host`/`Origin` and `hello` header rules with 4xx JSON-RPC errors,
//! passes Trace context from `_meta` or HTTP `traceparent`/`tracestate`
//! headers, and respects E1 counted-work invariants.

use std::net::SocketAddr;

use goodfetch_protocol::catalog::{
    Event, EventClass, Operation, OperationClass, OperationKind, Product, Reach, Stability,
    schema_of,
};
use goodfetch_protocol::http_listener::{
    EVENTS_PATH, HEADER_FORM, HEADER_PRODUCT, HEADER_PROTOCOL, RPC_PATH,
};
use goodfetch_test_support::TestEnv;
use http_body_util::{BodyExt as _, Full};
use hyper::body::Bytes;
use hyper::client::conn::http1;
use hyper::{Method, Request};
use hyper_util::rt::TokioIo;
use opentelemetry::trace::{SpanId, SpanKind, TraceId};
use rstest::rstest;
use secrecy::SecretString;
use serde_json::{Value, json};

use super::daemon::{TOKEN, daemon as served, no_handover, options as daemon_options};
use super::trace::{PARENT_ID, TRACEPARENT, exported};
use super::{engine, engine_with, records_of};
use crate::call_error::CallError;
use crate::composition::{CallContext, Contribution, Handler};
use crate::daemon::DaemonOptions;
use crate::endpoint::{Authorities, Exposure, REFUSAL_TARGET};
use crate::http_listener::HttpListenerOptions;
use crate::transport::{self, InMemoryDialer, Network};

/// The loopback address the in-memory HTTP listener claims.
const HTTP_ADDRESS: &str = "127.0.0.1:7801";

/// A visibly fake test token dedicated to the HTTP listener (never a real credential).
const HTTP_LISTENER_TEST_TOKEN: &str = "http-listener-test-token-0000";

fn http_address() -> SocketAddr {
    HTTP_ADDRESS.parse().unwrap()
}

/// Builds a Daemon configured with an in-memory HTTP listener and returns both
/// its `DaemonOptions` and the `InMemoryDialer` for sending HTTP requests to it.
fn with_http_listener() -> (DaemonOptions, InMemoryDialer) {
    let (network, dialer) = transport::in_memory();
    let (listener, _) = goodfetch_test_support::runtime()
        .block_on(Network::InMemory(network).listen_at(http_address()))
        .unwrap();
    let options = DaemonOptions {
        http_listener: Some(HttpListenerOptions {
            listener,
            bound: http_address(),
            token: SecretString::from(HTTP_LISTENER_TEST_TOKEN.to_owned()),
            exposure: Exposure::from(Authorities::loopback(http_address())),
        }),
        ..daemon_options(true, &no_handover())
    };
    (options, dialer)
}

/// Sends one HTTP/1.1 request over `dialer` and returns `(status, www_authenticate, body_json)`.
async fn send_http(
    dialer: &InMemoryDialer,
    request: Request<Full<Bytes>>,
) -> (u16, Option<String>, Value) {
    let (reader, writer) = dialer.connect().await.unwrap().split();
    let stream = tokio::io::join(reader, writer);
    let (mut sender, connection) = http1::handshake(TokioIo::new(stream)).await.unwrap();
    let drive = tokio::spawn(async move {
        let _ = connection.await;
    });
    let response = sender.send_request(request).await.unwrap();
    let status = response.status().as_u16();
    let www_auth = response
        .headers()
        .get("www-authenticate")
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let body = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or_else(|error| {
            panic!(
                "non-JSON body ({status}): {error}: {:?}",
                String::from_utf8_lossy(&bytes)
            )
        })
    };
    drop(sender);
    let _ = drive.await;
    (status, www_auth, body)
}

fn valid_rpc_request(body: &Value) -> Request<Full<Bytes>> {
    Request::builder()
        .method(Method::POST)
        .uri(RPC_PATH)
        .header("Host", HTTP_ADDRESS)
        .header(
            "Authorization",
            format!("Bearer {HTTP_LISTENER_TEST_TOKEN}"),
        )
        .header(HEADER_PROTOCOL, "0.1")
        .header(HEADER_PRODUCT, "collie")
        .header(HEADER_FORM, "catalog")
        .header("Content-Type", "application/json")
        .body(Full::new(Bytes::from(body.to_string())))
        .unwrap()
}

/// A plain HTTP client starts a Run via `POST /v1/rpc`, the Run outlives the
/// HTTP request, `daemon.status` reports both the live Run and the HTTP
/// listener address, and `run.finish` ends it.
///
/// Covers: story:11-258, story:11-259
#[test]
fn post_v1_rpc_starts_a_run_that_outlives_the_http_request() {
    let (options, http_dialer) = with_http_listener();
    let (_, (start_status, run_id, live_runs, reported_http, finish_status)) = served(
        &engine(Product::Collie),
        &TestEnv::new(),
        options,
        |_socket_dialer| async move {
            let (start_status, _, start_body) = send_http(
                &http_dialer,
                valid_rpc_request(&json!({
                    "jsonrpc": "2.0",
                    "id": 1,
                    "method": "run.start",
                    "params": {}
                })),
            )
            .await;
            let run_id = start_body["result"]["run_id"]
                .as_str()
                .expect("run.start returned run_id")
                .to_owned();

            let (status_code, _, status_body) = send_http(
                &http_dialer,
                valid_rpc_request(&json!({
                    "jsonrpc": "2.0",
                    "id": 2,
                    "method": "daemon.status",
                    "params": {}
                })),
            )
            .await;
            assert_eq!(status_code, 200);
            let live_runs = status_body["result"]["live_runs"].as_u64().unwrap();
            let reported_http = status_body["result"]["http_listener"]
                .as_str()
                .map(str::to_owned);

            let (finish_status, _, finish_body) = send_http(
                &http_dialer,
                valid_rpc_request(&json!({
                    "jsonrpc": "2.0",
                    "id": 3,
                    "method": "run.finish",
                    "params": { "run_id": run_id }
                })),
            )
            .await;
            assert!(finish_body.get("result").is_some(), "{finish_body}");
            drop(http_dialer);
            (
                start_status,
                run_id,
                live_runs,
                reported_http,
                finish_status,
            )
        },
    );

    assert_eq!(start_status, 200);
    assert_ne!(run_id, "");
    assert_eq!(live_runs, 1);
    assert_eq!(reported_http.as_deref(), Some(HTTP_ADDRESS));
    assert_eq!(finish_status, 200);
}

/// Missing, wrong, daemon-scoped or query-string tokens are refused with `401`
/// carrying a JSON-RPC error, `WWW-Authenticate: Bearer`, and an Audit/log
/// refusal under `http_listener`.
///
/// Covers: story:11-259, story:11-261
#[rstest]
#[case::missing(None, RPC_PATH, "token_missing")]
#[case::wrong(Some("wrong-token"), RPC_PATH, "token_wrong")]
#[case::daemon_token_scoped_elsewhere(Some(TOKEN), RPC_PATH, "token_wrong")]
#[case::query_string_only(
    None,
    "/v1/rpc?access_token=http-listener-test-token-0000",
    "token_missing"
)]
#[case::query_string_with_bearer(
    Some(HTTP_LISTENER_TEST_TOKEN),
    "/v1/rpc?access_token=http-listener-test-token-0000",
    "token_wrong"
)]
fn token_refusals_return_401_with_jsonrpc_error_and_log(
    #[case] bearer: Option<&str>,
    #[case] uri: &str,
    #[case] expected_reason: &str,
) {
    let (options, http_dialer) = with_http_listener();
    let mut observed = None;
    let records = records_of(REFUSAL_TARGET, || {
        observed = Some(
            served(
                &engine(Product::Collie),
                &TestEnv::new(),
                options,
                |_socket_dialer| async move {
                    let mut builder = Request::builder()
                        .method(Method::POST)
                        .uri(uri)
                        .header("Host", HTTP_ADDRESS)
                        .header(HEADER_PROTOCOL, "0.1")
                        .header(HEADER_PRODUCT, "collie")
                        .header(HEADER_FORM, "catalog");
                    if let Some(token) = bearer {
                        builder = builder.header("Authorization", format!("Bearer {token}"));
                    }
                    let request = builder
                        .body(Full::new(Bytes::from(
                            json!({"jsonrpc": "2.0", "id": 1, "method": "engine.info", "params": {}})
                                .to_string(),
                        )))
                        .unwrap();
                    let answer = send_http(&http_dialer, request).await;
                    drop(http_dialer);
                    answer
                },
            )
            .1,
        );
    });
    let (status, www_auth, body) = observed.unwrap();

    assert_eq!(status, 401);
    assert_eq!(www_auth.as_deref(), Some("Bearer"));
    assert_eq!(body["jsonrpc"], json!("2.0"));
    assert!(body["error"]["data"]["code"].is_string(), "{body}");
    assert!(
        records.iter().any(|record| record.contains(&format!(
            "listener=\"http_listener\" reason=\"{expected_reason}\""
        ))),
        "expected refusal `{expected_reason}` in {records:#?}"
    );
}

/// Foreign `Host` or `Origin` headers are refused with `403` carrying a
/// JSON-RPC error and logged under `http_listener`, while same-origin
/// `Origin` is accepted.
///
/// Covers: story:11-259, story:11-261
#[rstest]
#[case::foreign_host("foreign.invalid", None, 403, Some("host_foreign"))]
#[case::foreign_origin(
    HTTP_ADDRESS,
    Some("https://foreign.invalid"),
    403,
    Some("origin_foreign")
)]
#[case::other_port_origin(
    HTTP_ADDRESS,
    Some("http://127.0.0.1:7899"),
    403,
    Some("origin_foreign")
)]
#[case::same_origin(HTTP_ADDRESS, Some("http://127.0.0.1:7801"), 200, None)]
#[case::localhost_host("localhost:7801", None, 200, None)]
fn host_and_origin_checks_refuse_foreign_with_403_and_allow_same_origin(
    #[case] host: &str,
    #[case] origin: Option<&str>,
    #[case] expected_status: u16,
    #[case] expected_reason: Option<&str>,
) {
    let (options, http_dialer) = with_http_listener();
    let mut observed = None;
    let records = records_of(REFUSAL_TARGET, || {
        observed = Some(
            served(
                &engine(Product::Collie),
                &TestEnv::new(),
                options,
                |_socket_dialer| async move {
                    let mut builder = Request::builder()
                        .method(Method::POST)
                        .uri(RPC_PATH)
                        .header("Host", host)
                        .header("Authorization", format!("Bearer {HTTP_LISTENER_TEST_TOKEN}"))
                        .header(HEADER_PROTOCOL, "0.1")
                        .header(HEADER_PRODUCT, "collie")
                        .header(HEADER_FORM, "catalog");
                    if let Some(origin) = origin {
                        builder = builder.header("Origin", origin);
                    }
                    let request = builder
                        .body(Full::new(Bytes::from(
                            json!({"jsonrpc": "2.0", "id": 1, "method": "engine.info", "params": {}})
                                .to_string(),
                        )))
                        .unwrap();
                    let answer = send_http(&http_dialer, request).await;
                    drop(http_dialer);
                    answer
                },
            )
            .1,
        );
    });
    let (status, _, body) = observed.unwrap();

    assert_eq!(status, expected_status, "{body}");
    if let Some(reason) = expected_reason {
        assert_eq!(body["jsonrpc"], json!("2.0"));
        assert!(body.get("error").is_some(), "{body}");
        assert!(
            records.iter().any(|record| record
                .contains(&format!("listener=\"http_listener\" reason=\"{reason}\""))),
            "expected refusal `{reason}` in {records:#?}"
        );
    } else {
        assert!(body.get("result").is_some(), "{body}");
    }
}

/// Refused `hello` headers (`protocol_version_unsupported`, `not_in_product`,
/// or invalid/missing headers) return `400` carrying the JSON-RPC error from
/// `rpc`'s handshake.
///
/// Covers: story:11-259
#[rstest]
#[case::unsupported_protocol(
    Some("9.0"),
    Some("collie"),
    Some("catalog"),
    "protocol_version_unsupported"
)]
#[case::not_in_product(Some("0.1"), Some("foxhound"), Some("catalog"), "not_in_product")]
#[case::missing_protocol(None, Some("collie"), Some("catalog"), "invalid_params")]
#[case::missing_product(Some("0.1"), None, Some("catalog"), "invalid_params")]
#[case::invalid_form(Some("0.1"), Some("collie"), Some("bogus"), "invalid_params")]
fn refused_hello_headers_return_400_carrying_the_jsonrpc_error(
    #[case] protocol: Option<&str>,
    #[case] product: Option<&str>,
    #[case] form: Option<&str>,
    #[case] expected_code: &str,
) {
    let (options, http_dialer) = with_http_listener();
    let (_, (status, _, body)) = served(
        &engine(Product::Collie),
        &TestEnv::new(),
        options,
        |_socket_dialer| async move {
            let mut builder = Request::builder()
                .method(Method::POST)
                .uri(RPC_PATH)
                .header("Host", HTTP_ADDRESS)
                .header(
                    "Authorization",
                    format!("Bearer {HTTP_LISTENER_TEST_TOKEN}"),
                );
            if let Some(protocol) = protocol {
                builder = builder.header(HEADER_PROTOCOL, protocol);
            }
            if let Some(product) = product {
                builder = builder.header(HEADER_PRODUCT, product);
            }
            if let Some(form) = form {
                builder = builder.header(HEADER_FORM, form);
            }
            let request = builder
                .body(Full::new(Bytes::from(
                    json!({"jsonrpc": "2.0", "id": 1, "method": "engine.info", "params": {}})
                        .to_string(),
                )))
                .unwrap();
            let answer = send_http(&http_dialer, request).await;
            drop(http_dialer);
            answer
        },
    );

    assert_eq!(status, 400, "{body}");
    assert_eq!(body["jsonrpc"], json!("2.0"));
    assert_eq!(
        body["error"]["data"]["code"],
        json!(expected_code),
        "{body}"
    );
}

/// `POST /v1/rpc` takes the Trace context from `_meta`, or from the HTTP
/// `traceparent` and `tracestate` headers when `_meta` has none; an over-long
/// or malformed HTTP value is dropped and never refused (ADR 0132).
///
/// Covers: story:11-259
#[test]
fn trace_context_prefers_meta_falls_back_to_http_headers_and_drops_malformed() {
    let meta_trace_id = "11111111111111111111111111111111";
    let meta_parent_id = "2222222222222222";
    let meta_traceparent = format!("00-{meta_trace_id}-{meta_parent_id}-01");
    let overlong = "a".repeat(300);

    let (options, http_dialer) = with_http_listener();
    let (statuses, spans) =
        exported(7, || {
            served(
                &engine(Product::Collie),
                &TestEnv::new(),
                options,
                |_socket_dialer| async move {
                    // 1. No `_meta`: takes HTTP `traceparent` and `tracestate`.
                    let req_http_trace = Request::builder()
                    .method(Method::POST)
                    .uri(RPC_PATH)
                    .header("Host", HTTP_ADDRESS)
                    .header("Authorization", format!("Bearer {HTTP_LISTENER_TEST_TOKEN}"))
                    .header(HEADER_PROTOCOL, "0.1")
                    .header(HEADER_PRODUCT, "collie")
                    .header("traceparent", TRACEPARENT)
                    .header("tracestate", "vendor=value")
                    .body(Full::new(Bytes::from(
                        json!({"jsonrpc": "2.0", "id": 1, "method": "engine.info", "params": {}})
                            .to_string(),
                    )))
                    .unwrap();
                    let (s1, _, b1) = send_http(&http_dialer, req_http_trace).await;
                    assert!(b1.get("result").is_some(), "{b1}");

                    // 2. Both `_meta` and HTTP header: `_meta` wins.
                    let req_meta_wins = Request::builder()
                        .method(Method::POST)
                        .uri(RPC_PATH)
                        .header("Host", HTTP_ADDRESS)
                        .header(
                            "Authorization",
                            format!("Bearer {HTTP_LISTENER_TEST_TOKEN}"),
                        )
                        .header(HEADER_PROTOCOL, "0.1")
                        .header(HEADER_PRODUCT, "collie")
                        .header("traceparent", TRACEPARENT)
                        .body(Full::new(Bytes::from(
                            json!({
                                "jsonrpc": "2.0",
                                "id": 2,
                                "method": "engine.info",
                                "params": { "_meta": { "traceparent": meta_traceparent } }
                            })
                            .to_string(),
                        )))
                        .unwrap();
                    let (s2, _, b2) = send_http(&http_dialer, req_meta_wins).await;
                    assert!(b2.get("result").is_some(), "{b2}");

                    // 3. Over-long / malformed HTTP trace headers: dropped, never refused.
                    let req_malformed = Request::builder()
                    .method(Method::POST)
                    .uri(RPC_PATH)
                    .header("Host", HTTP_ADDRESS)
                    .header("Authorization", format!("Bearer {HTTP_LISTENER_TEST_TOKEN}"))
                    .header(HEADER_PROTOCOL, "0.1")
                    .header(HEADER_PRODUCT, "collie")
                    .header("traceparent", overlong.as_str())
                    .header("tracestate", "not valid === state")
                    .body(Full::new(Bytes::from(
                        json!({"jsonrpc": "2.0", "id": 3, "method": "engine.info", "params": {}})
                            .to_string(),
                    )))
                    .unwrap();
                    let (s3, _, b3) = send_http(&http_dialer, req_malformed).await;
                    assert!(b3.get("result").is_some(), "{b3}");

                    drop(http_dialer);
                    (s1, s2, s3)
                },
            )
            .1
        });

    assert_eq!(statuses, (200, 200, 200));
    let server_spans: Vec<_> = spans
        .iter()
        .filter(|span| span.span_kind == SpanKind::Server && span.name == "engine.info")
        .collect();
    assert_eq!(server_spans.len(), 3, "expected 3 engine.info spans");

    // Span 1 inherited HTTP `traceparent`.
    assert_eq!(
        server_spans[0].parent_span_id,
        SpanId::from_hex(PARENT_ID).unwrap()
    );
    assert_eq!(
        server_spans[0].span_context.trace_id(),
        TraceId::from_hex("0af7651916cd43dd8448eb211c80319c").unwrap()
    );
    assert_eq!(
        server_spans[0].span_context.trace_state().get("vendor"),
        Some("value")
    );

    // Span 2 inherited `_meta`, overriding HTTP `traceparent`.
    assert_eq!(
        server_spans[1].parent_span_id,
        SpanId::from_hex(meta_parent_id).unwrap()
    );
    assert_eq!(
        server_spans[1].span_context.trace_id(),
        TraceId::from_hex(meta_trace_id).unwrap()
    );

    // Span 3 started a fresh root trace after dropping the over-long/malformed headers.
    assert_eq!(server_spans[2].parent_span_id, SpanId::INVALID);
}

/// One parsed Server-Sent Event frame (`id`, `event`, JSON `data`).
#[derive(Debug, PartialEq)]
struct SseEvent {
    id: u64,
    event: String,
    data: Value,
}

/// Parses a complete SSE frame (`id: ...\nevent: ...\ndata: ...\n\n`).
fn parse_sse_frame(frame: &str) -> SseEvent {
    let mut id = None;
    let mut event = None;
    let mut data = None;
    for line in frame.lines() {
        if let Some(value) = line.strip_prefix("id: ") {
            id = Some(value.parse::<u64>().unwrap());
        } else if let Some(value) = line.strip_prefix("event: ") {
            event = Some(value.to_owned());
        } else if let Some(value) = line.strip_prefix("data: ") {
            data = Some(serde_json::from_str::<Value>(value).unwrap());
        }
    }
    SseEvent {
        id: id.expect("SSE frame missing id"),
        event: event.expect("SSE frame missing event"),
        data: data.expect("SSE frame missing data"),
    }
}

/// Opens a streaming HTTP/1.1 connection for `GET /v1/events` and returns the
/// response head + incoming body alongside the connection driver task handle.
async fn open_sse(
    dialer: &InMemoryDialer,
    uri: &str,
    extra_headers: &[(&str, &str)],
) -> (
    u16,
    Option<String>,
    hyper::body::Incoming,
    tokio::task::JoinHandle<()>,
) {
    let (reader, writer) = dialer.connect().await.unwrap().split();
    let io = TokioIo::new(tokio::io::join(reader, writer));
    let (mut sender, connection) = http1::handshake(io).await.unwrap();
    let driver = tokio::spawn(async move {
        let _ = connection.await;
    });
    let mut builder = Request::builder()
        .method(Method::GET)
        .uri(uri)
        .header("Host", HTTP_ADDRESS)
        .header(
            "Authorization",
            format!("Bearer {HTTP_LISTENER_TEST_TOKEN}"),
        );
    for (name, value) in extra_headers {
        builder = builder.header(*name, *value);
    }
    let response = sender
        .send_request(builder.body(Full::new(Bytes::new())).unwrap())
        .await
        .unwrap();
    let status = response.status().as_u16();
    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    (status, content_type, response.into_body(), driver)
}

/// Reads `count` complete SSE frames from `body`.
async fn read_sse_events(body: &mut hyper::body::Incoming, count: usize) -> Vec<SseEvent> {
    let mut buffer = String::new();
    let mut events = Vec::with_capacity(count);
    while events.len() < count {
        let frame = body
            .frame()
            .await
            .expect("SSE stream ended before expected event")
            .unwrap();
        if let Some(chunk) = frame.data_ref() {
            buffer.push_str(std::str::from_utf8(chunk).unwrap());
            while let Some(pos) = buffer.find("\n\n") {
                let raw = buffer[..pos].to_owned();
                buffer.drain(..pos + 2);
                if !raw.is_empty() {
                    events.push(parse_sse_frame(&raw));
                }
            }
        }
    }
    events
}

async fn rpc_run_call(dialer: &InMemoryDialer, id: u64, method: &str, run_id: &str) -> u16 {
    send_http(
        dialer,
        valid_rpc_request(&json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": { "run_id": run_id }
        })),
    )
    .await
    .0
}

async fn read_one_sse(
    dialer: &InMemoryDialer,
    uri: &str,
    headers: &[(&str, &str)],
) -> (u16, Option<String>, SseEvent) {
    let (status, content_type, mut body, driver) = open_sse(dialer, uri, headers).await;
    let event = read_sse_events(&mut body, 1).await.pop().unwrap();
    drop(body);
    driver.abort();
    (status, content_type, event)
}

/// `GET /v1/events?run=<id>&after=<position>` streams a Run's events as SSE
/// with `id` = event position; dropping and reconnecting with `Last-Event-ID`
/// receives each subsequent event once; `after` and `Last-Event-ID` give the
/// exact same stream; a token in the query string is refused with `401`.
///
/// Covers: story:11-260
#[test]
fn get_events_streams_run_events_resumes_identically_and_refuses_query_token() {
    let (options, http_dialer) = with_http_listener();
    let (
        _,
        (
            first_status,
            content_type,
            first_event,
            resumed_by_header,
            resumed_by_after,
            query_token_status,
        ),
    ) = served(
        &engine(Product::Collie),
        &TestEnv::new(),
        options,
        |_socket_dialer| async move {
            assert_eq!(
                (
                    rpc_run_call(&http_dialer, 1, "run.start", "sse-a").await,
                    rpc_run_call(&http_dialer, 2, "run.start", "sse-b").await,
                ),
                (200, 200)
            );
            let (first_status, content_type, first) = read_one_sse(
                &http_dialer,
                &format!("{EVENTS_PATH}?run=sse-a&after=0"),
                &[],
            )
            .await;
            assert_eq!(
                (
                    rpc_run_call(&http_dialer, 3, "run.finish", "sse-a").await,
                    rpc_run_call(&http_dialer, 4, "run.finish", "sse-b").await,
                ),
                (200, 200)
            );
            let last_id = first.id.to_string();
            let (header_status, _, resumed_header) = read_one_sse(
                &http_dialer,
                &format!("{EVENTS_PATH}?run=sse-a"),
                &[("Last-Event-ID", &last_id)],
            )
            .await;
            let (query_status, _, resumed_query) = read_one_sse(
                &http_dialer,
                &format!("{EVENTS_PATH}?run=sse-a&after={last_id}"),
                &[],
            )
            .await;
            assert_eq!((header_status, query_status), (200, 200));

            let (q_status, _, _) = send_http(
                &http_dialer,
                Request::builder()
                    .method(Method::GET)
                    .uri(format!(
                        "{EVENTS_PATH}?run=sse-a&token={HTTP_LISTENER_TEST_TOKEN}"
                    ))
                    .header("Host", HTTP_ADDRESS)
                    .header(
                        "Authorization",
                        format!("Bearer {HTTP_LISTENER_TEST_TOKEN}"),
                    )
                    .body(Full::new(Bytes::new()))
                    .unwrap(),
            )
            .await;
            drop(http_dialer);
            (
                first_status,
                content_type,
                first,
                resumed_header,
                resumed_query,
                q_status,
            )
        },
    );

    assert_eq!(first_status, 200);
    assert_eq!(
        content_type.as_deref(),
        Some("text/event-stream; charset=utf-8")
    );
    assert_eq!(first_event.event, "run.axes_changed");
    assert_eq!(first_event.data["data"]["run_id"], json!("sse-a"));
    assert_eq!(first_event.data["data"]["lifecycle"], json!("running"));
    assert!(resumed_by_header.id > first_event.id);
    assert_eq!(resumed_by_header.data["data"]["run_id"], json!("sse-a"));
    assert_eq!(
        resumed_by_header.data["data"]["lifecycle"],
        json!("finished")
    );
    assert_eq!(resumed_by_header, resumed_by_after);
    assert_eq!(query_token_status, 401);
}

const SSE_TEST_TELEMETRY: Event = Event {
    name: "test.sse_progress",
    description: "A test telemetry event for SSE coalescing.",
    class: EventClass::Telemetry,
    payload: schema_of::<Value>,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: goodfetch_protocol::catalog::PROTOCOL_VERSION,
};

const PUBLISH_BURST: Operation = Operation {
    name: "test.publish_burst",
    description: "Publishes a burst of telemetry events for one Run.",
    class: OperationClass::Effect,
    kind: OperationKind::RoundTrip,
    reach: Reach::AllSurfaces,
    params: schema_of::<Value>,
    result: schema_of::<Value>,
    events: &[&SSE_TEST_TELEMETRY],
    cancellable: false,
    input: None,
    bound: None,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: goodfetch_protocol::catalog::PROTOCOL_VERSION,
};

async fn read_chunked_sse_until_step(
    reader: &mut tokio::io::BufReader<crate::transport::ConnectionReader>,
    final_step: u64,
) -> Vec<SseEvent> {
    use tokio::io::{AsyncBufReadExt as _, AsyncReadExt as _};

    let mut seen = Vec::new();
    let mut buffer = String::new();
    let mut done = false;
    while !done {
        let mut size_line = String::new();
        reader.read_line(&mut size_line).await.unwrap();
        let size = usize::from_str_radix(size_line.trim(), 16).unwrap();
        if size == 0 {
            break;
        }
        let mut chunk = vec![0_u8; size + 2];
        reader.read_exact(&mut chunk).await.unwrap();
        buffer.push_str(std::str::from_utf8(&chunk[..size]).unwrap());
        while let Some(pos) = buffer.find("\n\n") {
            let raw = buffer[..pos].to_owned();
            buffer.drain(..pos + 2);
            if !raw.is_empty() {
                let event = parse_sse_frame(&raw);
                if event.data["data"]["step"] == json!(final_step) {
                    done = true;
                }
                seen.push(event);
            }
        }
    }
    seen
}

/// Telemetry events coalesce with `dropped` when an SSE reader's credit window
/// is full, and closing the reader tears down the subscription so subsequent
/// work proceeds unimpeded.
///
/// Covers: story:11-260
#[test]
fn get_events_coalesces_telemetry_with_dropped_under_credits_and_unsubscribes_on_drop() {
    use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};

    let contribution = Contribution::new("test-sse-telemetry").with_handler(Handler::new(
        &PUBLISH_BURST,
        |params: Value, context: CallContext| async move {
            let run = params["run"].as_str().unwrap();
            let count = params["count"].as_u64().unwrap();
            let pad = "p".repeat(8192);
            for step in 0..count {
                context
                    .events()
                    .publish(
                        &SSE_TEST_TELEMETRY,
                        Some(run),
                        &json!({ "run_id": run, "step": step, "pad": pad }),
                    )
                    .await;
            }
            Ok::<Value, CallError>(json!({ "published": count }))
        },
    ));
    let test_engine = engine_with(Product::Collie, Some(contribution));
    let (options, http_dialer) = with_http_listener();
    let (_, (seen_events, post_drop_status)) = served(
        &test_engine,
        &TestEnv::new(),
        options,
        |_socket_dialer| async move {
            let (reader, mut writer) = http_dialer.connect().await.unwrap().split();
            let request = format!(
                "GET {EVENTS_PATH}?run=burst-run HTTP/1.1\r\n\
                 Host: {HTTP_ADDRESS}\r\n\
                 Authorization: Bearer {HTTP_LISTENER_TEST_TOKEN}\r\n\r\n"
            );
            writer.write_all(request.as_bytes()).await.unwrap();
            let mut reader = BufReader::new(reader);
            let mut status_line = String::new();
            reader.read_line(&mut status_line).await.unwrap();
            assert!(status_line.contains("200"), "{status_line}");
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).await.unwrap();
                if line == "\r\n" || line.is_empty() {
                    break;
                }
            }

            let (pub_status, _, _) = send_http(
                &http_dialer,
                valid_rpc_request(&json!({
                    "jsonrpc": "2.0",
                    "id": 1,
                    "method": "test.publish_burst",
                    "params": { "run": "burst-run", "count": 100 }
                })),
            )
            .await;
            assert_eq!(pub_status, 200);

            let seen = read_chunked_sse_until_step(&mut reader, 99).await;
            drop(reader);
            drop(writer);
            let s_start = rpc_run_call(&http_dialer, 2, "run.start", "after-drop").await;
            let s_finish = rpc_run_call(&http_dialer, 3, "run.finish", "after-drop").await;
            drop(http_dialer);
            (seen, (s_start, s_finish))
        },
    );

    assert_eq!(post_drop_status, (200, 200));
    assert!(
        seen_events.len() < 100,
        "telemetry events should coalesce under credits, got {} events",
        seen_events.len()
    );
    assert!(
        seen_events
            .iter()
            .any(|event| event.data["dropped"].as_u64().is_some_and(|n| n > 0)),
        "expected a coalesced telemetry event with `dropped > 0`, got {seen_events:#?}"
    );
}
