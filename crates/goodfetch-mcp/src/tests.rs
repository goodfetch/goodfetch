//! In-memory `rmcp` tests for `goodfetch-mcp` (ADR 0070, ticket #1359).
#![expect(
    clippy::disallowed_types,
    reason = "unit tests record notifications across client handler callbacks using a Mutex"
)]
#![expect(
    deprecated,
    reason = "tests verify the MCP server never sends sampling requests (SEP-2577)"
)]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::connection::ProductVersion;
use goodfetch_protocol::jsonrpc::{Message, RequestId};
use rmcp::handler::client::ClientHandler;
use rmcp::model::{
    CallToolRequest, CallToolRequestParams, ClientRequest, ContentBlock,
    CreateMessageRequestParams, CreateMessageResult, ElicitRequestParams, ElicitResult,
    ProgressNotificationParam, RequestMetaObject,
};
use rmcp::service::{
    NotificationContext, PeerRequestOptions, RequestContext, RoleClient, ServiceExt,
};
use serde_json::{Map, Value, json};
use tokio::io::{AsyncBufReadExt as _, AsyncWrite, AsyncWriteExt as _, BufReader};
use tokio::sync::mpsc;
use tokio_util::task::TaskTracker;

use crate::engine::{EngineLink, EngineLinkConfig, build_clean_link};
use crate::profiles::{ProfileName, ResolvedProfile};
use crate::server::{McpServer, ServerOptions};
use crate::transports;

fn test_runtime() -> tokio::runtime::Runtime {
    goodfetch_runtime::multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap()
}

#[derive(Clone, Default)]
struct RecordingClient {
    progress: Arc<Mutex<Vec<ProgressNotificationParam>>>,
    sampling_requests: Arc<Mutex<usize>>,
    elicitation_requests: Arc<Mutex<usize>>,
}

impl ClientHandler for RecordingClient {
    async fn on_progress(
        &self,
        params: ProgressNotificationParam,
        _context: NotificationContext<RoleClient>,
    ) {
        self.progress.lock().unwrap().push(params);
    }

    fn create_message(
        &self,
        _params: CreateMessageRequestParams,
        _context: RequestContext<RoleClient>,
    ) -> impl std::future::Future<Output = Result<CreateMessageResult, rmcp::ErrorData>> + Send + '_
    {
        *self.sampling_requests.lock().unwrap() += 1;
        std::future::ready(Err(rmcp::ErrorData::method_not_found::<
            rmcp::model::CreateMessageRequestMethod,
        >()))
    }

    fn create_elicitation(
        &self,
        _request: ElicitRequestParams,
        _context: RequestContext<RoleClient>,
    ) -> impl std::future::Future<Output = Result<ElicitResult, rmcp::ErrorData>> + Send + '_ {
        *self.elicitation_requests.lock().unwrap() += 1;
        std::future::ready(Err(rmcp::ErrorData::method_not_found::<
            rmcp::model::ElicitationCreateRequestMethod,
        >()))
    }
}

fn hello_ok(id: &RequestId) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": {
            "protocol_version": "0.1",
            "product": "collie",
            "product_version": "0.1.0",
            "engine_instance": "01testengineinstance000000"
        }
    })
}

async fn write_line<W: AsyncWrite + Unpin>(writer: &mut W, value: &Value) {
    let mut bytes = serde_json::to_vec(value).unwrap();
    bytes.push(b'\n');
    writer.write_all(&bytes).await.unwrap();
    writer.flush().await.unwrap();
}

async fn read_message<R: tokio::io::AsyncRead + Unpin>(
    lines: &mut tokio::io::Lines<BufReader<R>>,
) -> Message {
    let line = lines.next_line().await.unwrap().expect("expected message");
    serde_json::from_str(&line).expect("valid JSON-RPC message")
}

/// Reads messages from the MCP server until a non-subscription request arrives.
async fn read_tool_request<R: tokio::io::AsyncRead + Unpin>(
    lines: &mut tokio::io::Lines<BufReader<R>>,
) -> goodfetch_protocol::jsonrpc::Request {
    loop {
        match read_message(lines).await {
            Message::Request(req) if req.method == "events.subscribe" => {}
            Message::Notification(notif) if notif.method == "$/credit" => {}
            Message::Request(req) => return req,
            other => panic!("unexpected engine message: {other:?}"),
        }
    }
}

/// Spawns an in-memory MCP server connected to a custom engine task and
/// returns the `rmcp` client + client recorder + shutdown tracker.
async fn start_in_memory<F, Fut>(
    live_view_base_url: Option<String>,
    engine_fn: F,
) -> (
    rmcp::service::RunningService<RoleClient, RecordingClient>,
    RecordingClient,
    TaskTracker,
)
where
    F: FnOnce(tokio::io::DuplexStream) -> Fut + Send + 'static,
    Fut: std::future::Future<Output = ()> + Send + 'static,
{
    start_in_memory_with_profile(
        Product::Collie,
        ResolvedProfile::for_product(Product::Collie, ProfileName::Browser),
        live_view_base_url,
        engine_fn,
    )
    .await
}

/// Spawns an in-memory MCP server with a chosen [`Product`] and [`ResolvedProfile`].
async fn start_in_memory_with_profile<F, Fut>(
    product: Product,
    profile: ResolvedProfile,
    live_view_base_url: Option<String>,
    engine_fn: F,
) -> (
    rmcp::service::RunningService<RoleClient, RecordingClient>,
    RecordingClient,
    TaskTracker,
)
where
    F: FnOnce(tokio::io::DuplexStream) -> Fut + Send + 'static,
    Fut: std::future::Future<Output = ()> + Send + 'static,
{
    start_in_memory_with_options(
        product,
        profile,
        crate::resources::ResourceStore::default(),
        live_view_base_url,
        engine_fn,
    )
    .await
}

/// Spawns an in-memory MCP server with a chosen [`Product`], [`ResolvedProfile`]
/// and [`crate::resources::ResourceStore`].
async fn start_in_memory_with_options<F, Fut>(
    product: Product,
    profile: ResolvedProfile,
    resources: crate::resources::ResourceStore,
    live_view_base_url: Option<String>,
    engine_fn: F,
) -> (
    rmcp::service::RunningService<RoleClient, RecordingClient>,
    RecordingClient,
    TaskTracker,
)
where
    F: FnOnce(tokio::io::DuplexStream) -> Fut + Send + 'static,
    Fut: std::future::Future<Output = ()> + Send + 'static,
{
    let tasks = TaskTracker::new();
    let (mcp_engine_side, fake_engine_side) = tokio::io::duplex(64 * 1024);
    tasks.spawn(engine_fn(fake_engine_side));

    let config = EngineLinkConfig {
        product,
        product_version: ProductVersion::parse("0.1.0").unwrap(),
        token: None,
        live_view_base_url,
    };
    let (link, _hello) = EngineLink::connect(mcp_engine_side, config)
        .await
        .expect("engine link hello succeeded");

    let server = McpServer::new(
        ServerOptions {
            product,
            version: "0.1.0".to_owned(),
            profile,
            resources,
        },
        link,
    );

    let (client_io, server_io) = tokio::io::duplex(64 * 1024);
    let (server_read, server_write) = tokio::io::split(server_io);
    tasks.spawn(async move {
        let _ = transports::serve_io(server, server_read, server_write).await;
    });

    let recorder = RecordingClient::default();
    let (client_read, client_write) = tokio::io::split(client_io);
    let client = recorder
        .clone()
        .serve((client_read, client_write))
        .await
        .expect("rmcp client initialized");

    (client, recorder, tasks)
}

async fn hello_only_engine(stream: tokio::io::DuplexStream) {
    let (read, mut write) = tokio::io::split(stream);
    let mut lines = BufReader::new(read).lines();
    if let Message::Request(request) = read_message(&mut lines).await {
        write_line(&mut write, &hello_ok(&request.id)).await;
    }
    let _ = lines.next_line().await;
}

/// Covers: story:11-222, story:11-229
#[test]
fn initialize_names_collie_and_carries_instructions_without_sampling_or_elicitation() {
    let rt = test_runtime();
    rt.block_on(async {
        let (client, recorder, tasks) = start_in_memory(None, |engine_io| async move {
            let (r, mut w) = tokio::io::split(engine_io);
            let mut lines = BufReader::new(r).lines();
            let Message::Request(hello) = read_message(&mut lines).await else {
                panic!("expected hello");
            };
            assert_eq!(hello.method, "hello");
            let params = hello.params.unwrap();
            assert_eq!(params["surface"], "mcp");
            assert_eq!(params["form"], "tool");
            assert_eq!(params["needs"], "collie");
            write_line(&mut w, &hello_ok(&hello.id)).await;
            let _ = lines.next_line().await;
        })
        .await;

        let info = client.peer_info().expect("server info received");
        assert_eq!(
            info.server_info.as_ref().map(|s| s.name.as_str()),
            Some("goodfetch-collie")
        );
        let instructions = info.instructions.as_deref().expect("instructions present");
        assert!(
            instructions.contains("browser"),
            "instructions: {instructions}"
        );
        assert!(
            instructions.contains("goodfetch://docs/llms.txt"),
            "instructions: {instructions}"
        );
        assert!(
            instructions.contains("skills/list"),
            "instructions: {instructions}"
        );
        assert_eq!(*recorder.sampling_requests.lock().unwrap(), 0);
        assert_eq!(*recorder.elicitation_requests.lock().unwrap(), 0);

        let _ = client.cancel().await;
        tasks.close();
        tasks.wait().await;
    });
}

/// Covers: story:11-222, story:11-229
#[test]
fn tool_call_returns_structured_content_json_text_and_error_code_as_is_error_with_fix_first() {
    let rt = test_runtime();
    rt.block_on(async {
        let (client, recorder, tasks) = start_in_memory(None, |engine_io| async move {
            let (r, mut w) = tokio::io::split(engine_io);
            let mut lines = BufReader::new(r).lines();
            let Message::Request(hello) = read_message(&mut lines).await else {
                panic!("expected hello");
            };
            write_line(&mut w, &hello_ok(&hello.id)).await;

            // Call 1: run.start succeeds
            let req1 = read_tool_request(&mut lines).await;
            assert_eq!(req1.method, "run.start");
            write_line(
                &mut w,
                &json!({
                    "jsonrpc": "2.0",
                    "id": req1.id,
                    "result": {
                        "run_id": "run_test_123",
                        "started_as": "new"
                    }
                }),
            )
            .await;

            // Call 2: run.finish fails with a typed Error code carrying a fix
            let req2 = read_tool_request(&mut lines).await;
            assert_eq!(req2.method, "run.finish");
            write_line(
                &mut w,
                &json!({
                    "jsonrpc": "2.0",
                    "id": req2.id,
                    "error": {
                        "code": -32001,
                        "message": "No Run with id `missing` exists.",
                        "data": {
                            "code": "run_not_found",
                            "retryable": false,
                            "fix": "Start a Run with `run_start` first."
                        }
                    }
                }),
            )
            .await;

            let _ = lines.next_line().await;
        })
        .await;

        let ok_res = client
            .call_tool(CallToolRequestParams::new("run_start").with_arguments(Map::new()))
            .await
            .expect("run_start succeeded");
        assert_ne!(ok_res.is_error, Some(true));
        let expected_ok = json!({"run_id": "run_test_123", "started_as": "new"});
        assert_eq!(ok_res.structured_content, Some(expected_ok.clone()));
        let first_text = ok_res
            .content
            .first()
            .and_then(|block| block.as_text())
            .expect("JSON text block");
        let parsed_text: Value = serde_json::from_str(&first_text.text).unwrap();
        assert_eq!(parsed_text, expected_ok);

        let mut finish_args = Map::new();
        finish_args.insert("run_id".to_owned(), Value::String("missing".to_owned()));
        let err_res = client
            .call_tool(CallToolRequestParams::new("run_finish").with_arguments(finish_args))
            .await
            .expect("error returns as CallToolResult, not JSON-RPC error");
        assert_eq!(err_res.is_error, Some(true));
        let structured = err_res
            .structured_content
            .expect("error structuredContent present");
        assert_eq!(structured["code"], "run_not_found");
        assert_eq!(structured["group"], "program");
        assert_eq!(structured["retryable"], false);
        assert_eq!(structured["operation"], "run.finish");
        assert_eq!(structured["fix"], "Start a Run with `run_start` first.");
        let err_text = err_res
            .content
            .first()
            .and_then(|block| block.as_text())
            .expect("error text block");
        assert!(
            err_text
                .text
                .starts_with("Start a Run with `run_start` first."),
            "fix must come first in error text, got: {}",
            err_text.text
        );

        assert_eq!(*recorder.sampling_requests.lock().unwrap(), 0);
        assert_eq!(*recorder.elicitation_requests.lock().unwrap(), 0);

        let _ = client.cancel().await;
        tasks.close();
        tasks.wait().await;
    });
}

/// Covers: story:11-229
#[test]
fn cancellation_sends_dollar_cancel_to_engine() {
    let rt = test_runtime();
    rt.block_on(async {
        let (cancelled_tx, cancelled_rx) = tokio::sync::oneshot::channel::<i64>();
        let (started_tx, started_rx) = tokio::sync::oneshot::channel::<()>();

        let (client, _recorder, tasks) = start_in_memory(None, |engine_io| async move {
            let (r, mut w) = tokio::io::split(engine_io);
            let mut lines = BufReader::new(r).lines();
            let Message::Request(hello) = read_message(&mut lines).await else {
                panic!("expected hello");
            };
            write_line(&mut w, &hello_ok(&hello.id)).await;

            let req = read_tool_request(&mut lines).await;
            let _ = started_tx.send(());

            // Wait for $/cancel notification (skipping any $/credit)
            let cancel = loop {
                match read_message(&mut lines).await {
                    Message::Notification(n) if n.method == "$/credit" => {}
                    Message::Notification(n) if n.method == "$/cancel" => break n,
                    other => panic!("expected $/cancel notification, got {other:?}"),
                }
            };
            assert_eq!(cancel.method, "$/cancel");
            let cancelled_id = cancel.params.unwrap()["id"].as_i64().unwrap();
            assert_eq!(RequestId::Number(cancelled_id), req.id);
            let _ = cancelled_tx.send(cancelled_id);
        })
        .await;

        let mut args = Map::new();
        args.insert("run_id".to_owned(), Value::String("run_slow".to_owned()));
        let handle = client
            .send_cancellable_request(
                ClientRequest::CallToolRequest(CallToolRequest::new(
                    CallToolRequestParams::new("run_finish").with_arguments(args),
                )),
                PeerRequestOptions::no_options(),
            )
            .await
            .expect("request sent");

        tokio::time::timeout(Duration::from_secs(5), started_rx)
            .await
            .expect("engine saw tool call")
            .unwrap();

        handle
            .cancel(Some("user aborted".to_owned()))
            .await
            .expect("cancel notification sent");

        let cancelled_id = tokio::time::timeout(Duration::from_secs(5), cancelled_rx)
            .await
            .expect("engine received $/cancel")
            .unwrap();
        assert!(cancelled_id >= 1);

        let _ = client.cancel().await;
        tasks.close();
        tasks.wait().await;
    });
}

async fn play_progress_and_pause_engine(
    engine_io: tokio::io::DuplexStream,
    mut release_rx: mpsc::Receiver<()>,
) {
    let (r, mut w) = tokio::io::split(engine_io);
    let mut lines = BufReader::new(r).lines();
    let Message::Request(hello) = read_message(&mut lines).await else {
        panic!("expected hello");
    };
    write_line(&mut w, &hello_ok(&hello.id)).await;

    let mut sub_id: Option<RequestId> = None;
    let mut call_req = None;
    while call_req.is_none() {
        match read_message(&mut lines).await {
            Message::Request(req) if req.method == "events.subscribe" => sub_id = Some(req.id),
            Message::Notification(notif) if notif.method == "$/credit" => {}
            Message::Request(req) if req.method == "run.finish" => call_req = Some(req),
            other => panic!("unexpected engine message: {other:?}"),
        }
    }
    let req = call_req.unwrap();
    let event_sub_id = sub_id.expect("events.subscribe opened for progressToken");

    write_line(
        &mut w,
        &json!({
            "jsonrpc": "2.0",
            "method": "$/events",
            "params": {
                "id": event_sub_id,
                "events": [{
                    "position": 1,
                    "event": "run.progress",
                    "data": {
                        "run_id": "run_prog",
                        "position": 2,
                        "at": "2026-11-28T00:00:00Z",
                        "lifecycle": "running",
                        "writer": "live",
                        "epoch": 1,
                        "pauses": [{
                            "session": "sess_1",
                            "page": "page_1",
                            "reason": "Complete login",
                            "raised_by": { "by": "engine", "step": "login_recovery" },
                            "since": "2026-11-28T00:00:00Z"
                        }],
                        "message": "Waiting on login",
                        "last": false
                    }
                }]
            }
        }),
    )
    .await;

    write_line(
        &mut w,
        &json!({
            "jsonrpc": "2.0",
            "method": "$/log",
            "params": {
                "level": "warn",
                "target": "goodfetch::run",
                "message": "Storage is running low",
                "fields": {},
                "dropped": 0
            }
        }),
    )
    .await;

    let _ = release_rx.recv().await;
    write_line(
        &mut w,
        &json!({
            "jsonrpc": "2.0",
            "id": req.id,
            "result": { "run_id": "run_prog", "lifecycle": "finished" }
        }),
    )
    .await;
}

/// Covers: story:11-229
#[test]
fn progress_token_receives_progress_warnings_and_human_pause_link_without_credentials() {
    let rt = test_runtime();
    rt.block_on(async {
        let (release_tx, release_rx) = mpsc::channel::<()>(1);
        let (client, recorder, tasks) = start_in_memory(
            Some("https://admin:top-secret-password@live.local:8443/?token=leaked#frag".to_owned()),
            move |engine_io| play_progress_and_pause_engine(engine_io, release_rx),
        )
        .await;

        let mut args = Map::new();
        args.insert("run_id".to_owned(), Value::String("run_prog".to_owned()));
        let req =
            CallToolRequest::new(CallToolRequestParams::new("run_finish").with_arguments(args));

        let handle = client
            .send_cancellable_request(
                ClientRequest::CallToolRequest(req),
                PeerRequestOptions::no_options(),
            )
            .await
            .expect("sent call_tool with progressToken");
        let expected_token = handle.progress_token.clone();

        tokio::time::timeout(Duration::from_secs(5), async {
            while recorder.progress.lock().unwrap().len() < 3 {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("timed out waiting for progress notifications");

        release_tx.send(()).await.unwrap();
        let response = tokio::time::timeout(Duration::from_secs(5), handle.await_response())
            .await
            .expect("call completed after human pause resolved")
            .expect("valid response");
        assert!(matches!(
            response,
            rmcp::model::ServerResult::CallToolResult(_)
        ));

        let updates = recorder.progress.lock().unwrap().clone();
        assert!(
            updates.iter().all(|u| u.progress_token == expected_token),
            "every progress notification carries the call's progressToken"
        );
        let messages: Vec<&str> = updates
            .iter()
            .filter_map(|u| u.message.as_deref())
            .collect();

        let pause_msg = messages
            .iter()
            .find(|m| m.contains("Human pause"))
            .expect("Human pause reported in progress");
        assert!(
            pause_msg
                .contains("https://live.local:8443/runs/run_prog/sessions/sess_1/pages/page_1"),
            "pause link present: {pause_msg}"
        );
        assert!(
            !pause_msg.contains("top-secret-password")
                && !pause_msg.contains("admin")
                && !pause_msg.contains("leaked"),
            "pause link must carry no credentials: {pause_msg}"
        );

        assert!(
            messages.iter().any(|m| m.contains("Waiting on login")),
            "run.progress message forwarded: {messages:?}"
        );
        assert!(
            messages
                .iter()
                .any(|m| m.contains("warning:") && m.contains("Storage is running low")),
            "warning forwarded: {messages:?}"
        );

        let _ = client.cancel().await;
        tasks.close();
        tasks.wait().await;
    });
}

/// Covers: story:11-222
#[test]
fn traceparent_and_tracestate_propagate_to_engine_meta_without_baggage() {
    let rt = test_runtime();
    rt.block_on(async {
        let (seen_meta_tx, seen_meta_rx) = tokio::sync::oneshot::channel::<Value>();

        let (client, _recorder, tasks) = start_in_memory(None, |engine_io| async move {
            let (r, mut w) = tokio::io::split(engine_io);
            let mut lines = BufReader::new(r).lines();
            let Message::Request(hello) = read_message(&mut lines).await else {
                panic!("expected hello");
            };
            write_line(&mut w, &hello_ok(&hello.id)).await;

            let req = read_tool_request(&mut lines).await;
            let params = req.params.unwrap();
            let _ = seen_meta_tx.send(params.get("_meta").cloned().unwrap_or(Value::Null));
            write_line(
                &mut w,
                &json!({
                    "jsonrpc": "2.0",
                    "id": req.id,
                    "result": { "run_id": "run_trace", "started_as": "new" }
                }),
            )
            .await;
            let _ = lines.next_line().await;
        })
        .await;

        let tp = "00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01";
        let ts = "vendor=value";
        let mut meta = RequestMetaObject::new();
        meta.set_traceparent(tp);
        meta.set_tracestate(ts);
        meta.set_baggage("secret_key=do_not_forward");

        let req = CallToolRequest::new(
            CallToolRequestParams::new("run_start").with_arguments(Map::new()),
        );
        let mut options = PeerRequestOptions::no_options();
        options.meta = Some(meta);

        let handle = client
            .send_cancellable_request(ClientRequest::CallToolRequest(req), options)
            .await
            .expect("call_tool sent");
        let _ = handle.await_response().await.expect("call_tool succeeded");

        let engine_meta = seen_meta_rx.await.expect("engine received _meta");
        assert_eq!(engine_meta["traceparent"], tp);
        assert_eq!(engine_meta["tracestate"], ts);
        assert!(
            engine_meta.get("baggage").is_none(),
            "baggage must not be passed to engine: {engine_meta}"
        );

        let _ = client.cancel().await;
        tasks.close();
        tasks.wait().await;
    });
}

#[test]
fn screenshot_result_includes_image_content_block() {
    let result = crate::projection::success_tool_result(json!({
        "mime_type": "image/png",
        "data": "iVBORw0KGgo="
    }));
    assert_eq!(result.content.len(), 2);
    assert!(result.content[0].as_text().is_some());
    assert!(matches!(
        &result.content[1],
        ContentBlock::Image(img) if img.mime_type == "image/png" && img.data == "iVBORw0KGgo="
    ));
}

#[test]
fn clean_link_strips_credentials_query_and_fragment() {
    let link = build_clean_link(
        Some("https://user:pass@example.test:8443/old?token=abc#sec"),
        "/runs/r1/sessions/s1/pages/p1",
    );
    assert_eq!(
        link,
        "https://example.test:8443/runs/r1/sessions/s1/pages/p1"
    );
}

/// Covers: story:11-222
#[test]
fn first_profile_tools_list_is_a_snapshot() {
    let profile = ResolvedProfile::for_product(Product::Collie, ProfileName::Browser);
    let summary: Vec<Value> = profile
        .list_tools()
        .into_iter()
        .map(|tool| {
            let projected = profile.get(tool.name.as_ref()).expect("listed tool exists");
            json!({
                "name": tool.name,
                "operation": projected.operation,
                "annotations": tool.annotations,
                "input_schema_type": tool.input_schema.get("type"),
            })
        })
        .collect();
    insta::assert_snapshot!(
        "browser_profile_tools_list",
        serde_json::to_string_pretty(&summary).unwrap()
    );
}

fn empty_schema(_generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
    schemars::json_schema!({
        "type": "object",
        "properties": {}
    })
}

fn fixture_operation(
    name: &'static str,
    description: &'static str,
    class: goodfetch_protocol::catalog::OperationClass,
    deprecated: Option<goodfetch_protocol::catalog::Deprecation>,
) -> goodfetch_protocol::catalog::Operation {
    use goodfetch_protocol::catalog::{
        Operation, OperationKind, ProtocolVersion, Reach, Stability,
    };
    Operation {
        name,
        description,
        class,
        kind: OperationKind::RoundTrip,
        reach: Reach::AllSurfaces,
        params: empty_schema,
        result: empty_schema,
        events: &[],
        cancellable: false,
        input: None,
        bound: None,
        stability: Stability::Stable,
        deprecated,
        arrived_in: ProtocolVersion { major: 0, minor: 1 },
    }
}

/// Covers: story:11-183, story:11-226
#[test]
fn projection_rejects_invalid_names_long_descriptions_and_live_view_grants() {
    use goodfetch_protocol::catalog::{OperationClass, Product};
    use goodfetch_protocol::entries::operations_of;

    use crate::profiles::is_mcp_eligible;
    use crate::projection::{
        MAX_DESCRIPTION_CHARS, ProjectionViolation, description_for, validate_operation,
    };

    for product in [Product::Collie, Product::Foxhound] {
        for op in operations_of(product).filter(|op| is_mcp_eligible(op)) {
            assert_eq!(
                validate_operation(op),
                Ok(()),
                "eligible operation `{}` failed MCP projection validation",
                op.name
            );
            if op.deprecated.is_none() {
                assert_eq!(
                    description_for(op),
                    op.description.trim(),
                    "MCP description must equal the catalog/SDK docstring text for `{}`",
                    op.name
                );
            }
        }
    }

    let too_long_name = fixture_operation(
        "very_long_namespace.very_long_member_name_exceeding_39",
        "Valid description.",
        OperationClass::Read,
        None,
    );
    assert!(matches!(
        validate_operation(&too_long_name),
        Err(ProjectionViolation::InvalidToolName { .. })
    ));

    let bad_chars_name = fixture_operation(
        "page.Bad-Name",
        "Valid description.",
        OperationClass::Read,
        None,
    );
    assert!(matches!(
        validate_operation(&bad_chars_name),
        Err(ProjectionViolation::InvalidToolName { .. })
    ));

    let long_desc: &'static str = Box::leak("a".repeat(MAX_DESCRIPTION_CHARS + 1).into_boxed_str());
    let too_long_desc = fixture_operation("page.verbose", long_desc, OperationClass::Read, None);
    assert_eq!(
        validate_operation(&too_long_desc),
        Err(ProjectionViolation::DescriptionTooLong {
            operation: "page.verbose".to_owned(),
            chars: MAX_DESCRIPTION_CHARS + 1,
        })
    );

    // Issue #1014: Live view grant entries are refused on the MCP surface.
    let live_view_grant = fixture_operation(
        "live_view.grant",
        "Grants a Live view channel.",
        OperationClass::IdempotentWrite,
        None,
    );
    assert_eq!(
        validate_operation(&live_view_grant),
        Err(ProjectionViolation::LiveViewGrantNotAllowed {
            operation: "live_view.grant".to_owned(),
        })
    );
    assert!(
        !is_mcp_eligible(&live_view_grant),
        "live_view.grant must be excluded from every MCP profile"
    );
}

/// Covers: story:11-226, story:11-232
#[test]
fn annotations_follow_operation_class_and_open_world_rule() {
    use goodfetch_protocol::catalog::{OperationClass, Product};
    use goodfetch_protocol::entries::operations_of;

    use crate::profiles::is_mcp_eligible;
    use crate::projection::{annotations_for, is_open_world};

    for product in [Product::Collie, Product::Foxhound] {
        for op in operations_of(product).filter(|op| is_mcp_eligible(op)) {
            let ann = annotations_for(op);
            let expected_open_world = is_open_world(op);
            assert_eq!(
                ann.open_world_hint,
                Some(expected_open_world),
                "{}",
                op.name
            );
            match op.class {
                OperationClass::Read => {
                    assert_eq!(ann.read_only_hint, Some(true), "{}", op.name);
                    assert_eq!(ann.destructive_hint, None, "{}", op.name);
                    assert_eq!(ann.idempotent_hint, None, "{}", op.name);
                }
                OperationClass::IdempotentWrite => {
                    assert_eq!(ann.read_only_hint, Some(false), "{}", op.name);
                    assert_eq!(ann.destructive_hint, Some(false), "{}", op.name);
                    assert_eq!(ann.idempotent_hint, Some(true), "{}", op.name);
                }
                OperationClass::Effect => {
                    assert_eq!(ann.read_only_hint, Some(false), "{}", op.name);
                    assert_eq!(ann.destructive_hint, Some(true), "{}", op.name);
                    assert_eq!(ann.idempotent_hint, Some(false), "{}", op.name);
                }
            }
        }
    }

    // Verify wire delivery over in-memory rmcp client for all 3 classes + openWorldHint.
    test_runtime().block_on(async {
        let (client, _recorder, tasks) = start_in_memory(None, hello_only_engine).await;

        let list = client.list_tools(None).await.expect("list_tools");
        let by_name = |name: &str| {
            list.tools
                .iter()
                .find(|t| t.name == name)
                .unwrap_or_else(|| panic!("tool `{name}` missing from browser profile"))
        };

        let page_goto = by_name("page_goto")
            .annotations
            .as_ref()
            .expect("page_goto annotations");
        assert_eq!(page_goto.read_only_hint, Some(false));
        assert_eq!(page_goto.destructive_hint, Some(true));
        assert_eq!(page_goto.idempotent_hint, Some(false));
        assert_eq!(page_goto.open_world_hint, Some(true));

        let run_start = by_name("run_start")
            .annotations
            .as_ref()
            .expect("run_start annotations");
        assert_eq!(run_start.read_only_hint, Some(false));
        assert_eq!(run_start.destructive_hint, Some(false));
        assert_eq!(run_start.idempotent_hint, Some(true));
        assert_eq!(run_start.open_world_hint, Some(false));

        let page_read = by_name("page_read")
            .annotations
            .as_ref()
            .expect("page_read annotations");
        assert_eq!(page_read.read_only_hint, Some(true));
        assert_eq!(page_read.open_world_hint, Some(false));

        let _ = client.cancel().await;
        tasks.close();
        tasks.wait().await;
    });
}

/// Covers: story:11-226
#[test]
fn deprecated_fixture_entry_sets_prefix_and_schema_flag() {
    use goodfetch_protocol::catalog::{Deprecation, Operation, OperationClass, ProtocolVersion};

    static DEPRECATED_OP: Operation = Operation {
        name: "page.open_legacy",
        description: "Navigates a page to a URL.",
        class: OperationClass::Effect,
        kind: goodfetch_protocol::catalog::OperationKind::RoundTrip,
        reach: goodfetch_protocol::catalog::Reach::AllSurfaces,
        params: empty_schema,
        result: empty_schema,
        events: &[],
        cancellable: true,
        input: None,
        bound: None,
        stability: goodfetch_protocol::catalog::Stability::Stable,
        deprecated: Some(Deprecation {
            since: ProtocolVersion { major: 1, minor: 0 },
            replacement: Some("page.goto"),
            earliest_removal: ProtocolVersion { major: 2, minor: 0 },
            note: "Use page.goto instead.",
        }),
        arrived_in: ProtocolVersion { major: 0, minor: 1 },
    };

    test_runtime().block_on(async {
        let profile = ResolvedProfile::from_operations("fixture".to_owned(), vec![&DEPRECATED_OP]);
        let (client, _recorder, tasks) =
            start_in_memory_with_profile(Product::Collie, profile, None, hello_only_engine).await;

        let list = client.list_tools(None).await.expect("list_tools");
        assert_eq!(list.tools.len(), 1);
        let tool = &list.tools[0];
        assert_eq!(tool.name, "page_open_legacy");
        let desc = tool.description.as_deref().expect("description");
        assert!(
            desc.starts_with("Deprecated: use `page_goto`, removed in 2"),
            "unexpected deprecated description: {desc}"
        );
        assert_eq!(
            tool.input_schema.get("deprecated"),
            Some(&json!(true)),
            "deprecated fixture inputSchema must set deprecated: true"
        );

        let _ = client.cancel().await;
        tasks.close();
        tasks.wait().await;
    });
}

/// Covers: story:11-185, story:11-232
#[test]
fn per_profile_tools_list_bytes_and_named_workload_results_are_snapshots() {
    use crate::projection::tools_list_wire_bytes;

    let mut profile_sizes = Map::new();
    for (product_label, product) in [("collie", Product::Collie), ("foxhound", Product::Foxhound)] {
        let mut per_profile = Map::new();
        for profile_name in ProfileName::ALL {
            let resolved = ResolvedProfile::for_product(product, profile_name);
            let tools: Vec<_> = resolved.list_tools();
            let bytes = tools_list_wire_bytes(&tools);
            per_profile.insert(
                profile_name.as_str().to_owned(),
                json!({
                    "tool_count": tools.len(),
                    "tools_list_bytes": bytes,
                }),
            );
        }
        profile_sizes.insert(product_label.to_owned(), Value::Object(per_profile));
    }

    insta::assert_snapshot!(
        "profile_tools_list_bytes",
        serde_json::to_string_pretty(&Value::Object(profile_sizes)).unwrap()
    );

    // Named workloads from issue #1360 exercised end-to-end through `McpServer`
    // over in-memory `rmcp` so `tool_result_wire_bytes` measures real MCP tool
    // call responses rather than isolated JSON literals.
    let workload_sizes = measure_named_mcp_workloads();

    insta::assert_snapshot!(
        "workload_tool_result_bytes",
        serde_json::to_string_pretty(&Value::Object(workload_sizes)).unwrap()
    );
}

static OP_ARIA_SNAPSHOT: goodfetch_protocol::catalog::Operation =
    goodfetch_protocol::catalog::Operation {
        name: "page.aria_snapshot",
        description: "Returns the page's ARIA snapshot.",
        class: goodfetch_protocol::catalog::OperationClass::Read,
        kind: goodfetch_protocol::catalog::OperationKind::Query,
        reach: goodfetch_protocol::catalog::Reach::AllSurfaces,
        params: empty_schema,
        result: empty_schema,
        events: &[],
        cancellable: true,
        input: None,
        bound: None,
        stability: goodfetch_protocol::catalog::Stability::Stable,
        deprecated: None,
        arrived_in: goodfetch_protocol::catalog::ProtocolVersion { major: 0, minor: 1 },
    };

static OP_SUCCESS_CHECK: goodfetch_protocol::catalog::Operation =
    goodfetch_protocol::catalog::Operation {
        name: "success_check.evaluate",
        description: "Evaluates success checks for a Run.",
        class: goodfetch_protocol::catalog::OperationClass::Read,
        kind: goodfetch_protocol::catalog::OperationKind::Query,
        reach: goodfetch_protocol::catalog::Reach::AllSurfaces,
        params: empty_schema,
        result: empty_schema,
        events: &[],
        cancellable: true,
        input: None,
        bound: None,
        stability: goodfetch_protocol::catalog::Stability::Stable,
        deprecated: None,
        arrived_in: goodfetch_protocol::catalog::ProtocolVersion { major: 0, minor: 1 },
    };

static OP_CODE_MODE: goodfetch_protocol::catalog::Operation =
    goodfetch_protocol::catalog::Operation {
        name: "code_mode.run",
        description: "Executes a sandboxed Code mode program.",
        class: goodfetch_protocol::catalog::OperationClass::Effect,
        kind: goodfetch_protocol::catalog::OperationKind::RoundTrip,
        reach: goodfetch_protocol::catalog::Reach::AllSurfaces,
        params: empty_schema,
        result: empty_schema,
        events: &[],
        cancellable: true,
        input: None,
        bound: None,
        stability: goodfetch_protocol::catalog::Stability::Stable,
        deprecated: None,
        arrived_in: goodfetch_protocol::catalog::ProtocolVersion { major: 0, minor: 1 },
    };

#[expect(
    clippy::too_many_lines,
    reason = "end-to-end fixture executing all 6 named MCP workloads from issue #1360"
)]
fn measure_named_mcp_workloads() -> serde_json::Map<String, Value> {
    use crate::projection::tool_result_wire_bytes;
    use goodfetch_protocol::run::{Lifecycle, RunId, RunStarted, StartedAs};
    let run_started_payload = serde_json::to_value(RunStarted {
        run_id: RunId::parse("run-01j00000000000000000000001").unwrap(),
        started_as: StartedAs::New,
        lifecycle: Lifecycle::Running,
        needs: Product::Collie,
    })
    .unwrap();

    let mut ops = crate::profiles::profile_operations(Product::Collie, ProfileName::Browser);
    ops.push(&OP_ARIA_SNAPSHOT);
    ops.push(&OP_SUCCESS_CHECK);
    ops.push(&OP_CODE_MODE);
    let profile = ResolvedProfile::from_operations("workload-measure".to_owned(), ops);

    test_runtime().block_on(async {
        let run_started_for_engine = run_started_payload.clone();
        let (client, _recorder, tasks) = start_in_memory_with_profile(
            Product::Collie,
            profile,
            None,
            move |stream| async move {
                let (read, mut writer) = tokio::io::split(stream);
                let mut reader = BufReader::new(read);
                let mut line = String::new();
                let mut aria_calls: u32 = 0;
                while reader.read_line(&mut line).await.unwrap_or(0) > 0 {
                    let msg: Message = serde_json::from_str(line.trim()).unwrap();
                    line.clear();
                    if let Message::Request(req) = msg {
                        match req.method.as_str() {
                            "hello" => write_line(&mut writer, &hello_ok(&req.id)).await,
                            "events.subscribe" => {
                                write_line(
                                    &mut writer,
                                    &json!({"jsonrpc": "2.0", "id": req.id, "result": {}}),
                                )
                                .await;
                            }
                            "run.start" => {
                                write_line(
                                    &mut writer,
                                    &json!({"jsonrpc": "2.0", "id": req.id, "result": run_started_for_engine}),
                                )
                                .await;
                            }
                            "page.goto" => {
                                write_line(
                                    &mut writer,
                                    &json!({
                                        "jsonrpc": "2.0",
                                        "id": req.id,
                                        "result": {
                                            "url": "https://shop.test/products/1",
                                            "status": 200,
                                            "title": "Product 1 — Shop"
                                        }
                                    }),
                                )
                                .await;
                            }
                            "page.aria_snapshot" => {
                                aria_calls += 1;
                                let result = if aria_calls == 1 {
                                    json!({
                                        "mode": "ai",
                                        "generation": 1,
                                        "incremental": false,
                                        "tree": "- main:\n  - heading \"Product 1\" [ref=e1]\n  - button \"Add to cart\" [ref=e2]"
                                    })
                                } else {
                                    json!({
                                        "mode": "ai",
                                        "generation": 2,
                                        "incremental": true,
                                        "diff": "@@ e2 @@\n- button \"Add to cart\" [ref=e2]\n+ button \"Added\" [ref=e2]"
                                    })
                                };
                                write_line(
                                    &mut writer,
                                    &json!({"jsonrpc": "2.0", "id": req.id, "result": result}),
                                )
                                .await;
                            }
                            "success_check.evaluate" => {
                                write_line(
                                    &mut writer,
                                    &json!({
                                        "jsonrpc": "2.0",
                                        "id": req.id,
                                        "result": {
                                            "passed": true,
                                            "checks": [
                                                { "name": "cart_count", "passed": true, "observed": 1 }
                                            ]
                                        }
                                    }),
                                )
                                .await;
                            }
                            "code_mode.run" => {
                                write_line(
                                    &mut writer,
                                    &json!({
                                        "jsonrpc": "2.0",
                                        "id": req.id,
                                        "result": {
                                            "ok": true,
                                            "value": { "title": "Product 1 — Shop", "price": 1999 },
                                            "logs": ["navigated to /products/1"]
                                        }
                                    }),
                                )
                                .await;
                            }
                            other => panic!("unexpected method `{other}`"),
                        }
                    }
                }
            },
        )
        .await;

        let calls: [(&str, &str, Value); 6] = [
            ("run_start", "run_start", json!({})),
            (
                "page_goto",
                "page_goto",
                json!({ "page": "page-1", "url": "https://shop.test/products/1" }),
            ),
            (
                "aria_snapshot_ai_first",
                "page_aria_snapshot",
                json!({ "page": "page-1", "mode": "ai" }),
            ),
            (
                "aria_snapshot_ai_second",
                "page_aria_snapshot",
                json!({ "page": "page-1", "mode": "ai" }),
            ),
            (
                "success_check_evaluate",
                "success_check_evaluate",
                json!({ "run_id": "run-01j00000000000000000000001" }),
            ),
            (
                "code_mode_run",
                "code_mode_run",
                json!({ "program": "return { title: 'Product 1' };" }),
            ),
        ];

        let mut sizes = Map::new();
        for (label, tool_name, args) in calls {
            let result = client
                .call_tool(
                    CallToolRequestParams::new(tool_name)
                        .with_arguments(args.as_object().cloned().unwrap_or_default()),
                )
                .await
                .unwrap_or_else(|err| panic!("workload `{label}` failed: {err}"));
            sizes.insert(
                label.to_owned(),
                json!({
                    "wire_bytes": tool_result_wire_bytes(&result),
                    "is_error": result.is_error,
                    "content_blocks": result.content.len(),
                }),
            );
        }

        let _ = client.cancel().await;
        tasks.close();
        tasks.wait().await;
        sizes
    })
}

/// Covers: story:11-223, story:11-224, story:11-225, mcp-profile:browser, mcp-profile:agent, mcp-profile:run, mcp-profile:code, mcp-profile:full
#[test]
fn every_profile_projection_is_a_snapshot_and_identical_in_foxhound() {
    use crate::projection::tools_list_wire_bytes;

    for profile_name in ProfileName::ALL {
        let collie = ResolvedProfile::for_product(Product::Collie, profile_name);
        let foxhound = ResolvedProfile::for_product(Product::Foxhound, profile_name);

        let collie_tools: Vec<_> = collie.list_tools();
        let foxhound_tools: Vec<_> = foxhound.list_tools();

        let collie_bytes = tools_list_wire_bytes(&collie_tools);
        let snapshot_value = json!({
            "profile": profile_name.as_str(),
            "tools_list_bytes": collie_bytes,
            "tools": collie_tools.iter().map(|tool| {
                let p = collie.get(tool.name.as_ref()).expect("listed tool exists");
                json!({
                    "name": tool.name,
                    "operation": p.operation,
                    "description": tool.description,
                    "annotations": tool.annotations,
                    "input_schema_keys": tool.input_schema.get("properties")
                        .and_then(Value::as_object)
                        .map(|m| m.keys().cloned().collect::<Vec<_>>())
                        .unwrap_or_default(),
                })
            }).collect::<Vec<_>>(),
        });
        insta::assert_snapshot!(
            format!("profile_{}", profile_name.as_str()),
            serde_json::to_string_pretty(&snapshot_value).unwrap()
        );

        match profile_name {
            ProfileName::Browser | ProfileName::Agent | ProfileName::Run | ProfileName::Code => {
                // Collie's profiles are byte-for-byte identical in Foxhound's server (ADRs 0090, 0095).
                assert_eq!(
                    serde_json::to_vec(&collie_tools).unwrap(),
                    serde_json::to_vec(&foxhound_tools).unwrap(),
                    "profile `{}` must be identical in Collie and Foxhound",
                    profile_name.as_str()
                );
            }
            ProfileName::Full => {
                // Foxhound's `full` contains every Collie `full` tool identically (ADR 0094).
                for tool in &collie_tools {
                    let found = foxhound_tools
                        .iter()
                        .find(|t| t.name == tool.name)
                        .unwrap_or_else(|| {
                            panic!(
                                "Collie full tool `{}` missing from Foxhound full",
                                tool.name
                            )
                        });
                    assert_eq!(
                        serde_json::to_value(tool).unwrap(),
                        serde_json::to_value(found).unwrap(),
                        "Collie full tool `{}` must match identically in Foxhound full",
                        tool.name
                    );
                }
            }
        }
    }

    // `code` profile offers zero per-operation tools (until #1375 adds code_mode_run)
    // and serves cleanly over rmcp in-memory.
    test_runtime().block_on(async {
        let (client, _recorder, tasks) = start_in_memory_with_profile(
            Product::Collie,
            ResolvedProfile::for_product(Product::Collie, ProfileName::Code),
            None,
            hello_only_engine,
        )
        .await;
        let list = client.list_tools(None).await.expect("list_tools");
        assert!(
            list.tools.is_empty(),
            "code profile must offer no per-operation tools, got: {:?}",
            list.tools
                .iter()
                .map(|t| t.name.as_ref())
                .collect::<Vec<_>>()
        );
        let _ = client.cancel().await;
        tasks.close();
        tasks.wait().await;
    });
}

/// Covers: story:11-225, story:11-230, story:11-235
#[test]
fn no_kept_off_or_foxhound_operation_appears_and_classes_match_catalog() {
    use std::collections::{BTreeMap, BTreeSet};

    use goodfetch_protocol::catalog::{Reach, SDK_ONLY};
    use goodfetch_protocol::entries::operations_of;

    use crate::profiles::KEPT_OFF_OPERATIONS;
    use crate::projection::{annotations_for, is_live_view_grant};

    let collie_ops: BTreeSet<&str> = operations_of(Product::Collie).map(|o| o.name).collect();
    let all_ops: BTreeMap<&str, _> = operations_of(Product::Foxhound)
        .map(|o| (o.name, o))
        .collect();

    for product in [Product::Collie, Product::Foxhound] {
        for profile_name in ProfileName::ALL {
            let resolved = ResolvedProfile::for_product(product, profile_name);
            for tool in resolved.list_tools() {
                let projected = resolved
                    .get(tool.name.as_ref())
                    .expect("listed tool exists");
                let op = all_ops.get(projected.operation).unwrap_or_else(|| {
                    panic!("unknown catalog operation `{}`", projected.operation)
                });

                assert_eq!(
                    op.reach,
                    Reach::AllSurfaces,
                    "person-only or sdk-only `{}` leaked into `{}`",
                    op.name,
                    profile_name.as_str()
                );
                assert!(
                    !KEPT_OFF_OPERATIONS.contains(&op.name),
                    "kept-off `{}` leaked into `{}`",
                    op.name,
                    profile_name.as_str()
                );
                assert!(
                    !SDK_ONLY.contains(&op.name),
                    "SDK-only `{}` leaked into `{}`",
                    op.name,
                    profile_name.as_str()
                );
                assert!(
                    !is_live_view_grant(op.name),
                    "Live view grant `{}` leaked into `{}`",
                    op.name,
                    profile_name.as_str()
                );

                // No Foxhound-only entry on any Collie profile or on Foxhound's Collie profiles.
                if product == Product::Collie || profile_name != ProfileName::Full {
                    assert!(
                        collie_ops.contains(op.name),
                        "Foxhound-only `{}` leaked into {:?} `{}`",
                        op.name,
                        product,
                        profile_name.as_str()
                    );
                }

                // Granting parameter `accept_plugin_change` is stripped from `run_start` and `recipe_start`.
                if matches!(op.name, "run.start" | "recipe.start") {
                    let props = projected
                        .tool
                        .input_schema
                        .get("properties")
                        .and_then(Value::as_object);
                    assert!(
                        props.is_none_or(|m| !m.contains_key("accept_plugin_change")),
                        "`{}` must omit `accept_plugin_change` from inputSchema",
                        op.name
                    );
                }

                // Each surface's operations map to catalog operations of the same class.
                assert_eq!(
                    projected.class, op.class,
                    "class mismatch for `{}`",
                    op.name
                );
                assert_eq!(
                    projected.tool.annotations,
                    Some(annotations_for(op)),
                    "annotations mismatch for `{}`",
                    op.name
                );
            }
        }
    }
}

async fn read_text_resource(
    client: &rmcp::service::RunningService<RoleClient, RecordingClient>,
    uri: &str,
) -> (String, Option<String>) {
    let read = client
        .read_resource(rmcp::model::ReadResourceRequestParams::new(uri.to_owned()))
        .await
        .expect("read_resource");
    match &read.contents[0] {
        rmcp::model::ResourceContents::TextResourceContents {
            text, mime_type, ..
        } => (text.clone(), mime_type.clone()),
        other => panic!("expected text resource for `{uri}`, got {other:?}"),
    }
}

/// Covers: story:11-234
#[test]
fn resources_and_skills_served_over_mcp_with_verified_digests() {
    use rmcp::model::{GetSkillRequestParams, SkillResources};

    use crate::resources::{
        EmbeddedDoc, EmbeddedSkillFile, LLMS_TXT_URI, ResourceStore, SKILL_TREE_DIGEST_META_KEY,
        skill_tree_digest,
    };

    let skill_md = b"---\nname: goodfetch-collie\ndescription: \"Drive a real browser with goodfetch-collie.\"\n---\n# goodfetch-collie\n\nStart a Run.\n";
    let expected_tree_digest = skill_tree_digest([("SKILL.md", skill_md.as_slice())].into_iter());
    let store = ResourceStore::from_embedded(
        Product::Collie,
        "0.1.0",
        vec![EmbeddedDoc {
            page: "runs.md".to_owned(),
            title: "Runs".to_owned(),
            markdown: "# Runs\n\nEvery action runs inside a Run.\n".to_owned(),
        }],
        vec![EmbeddedSkillFile {
            path: "goodfetch-collie/SKILL.md".to_owned(),
            contents: skill_md.to_vec(),
        }],
    );

    test_runtime().block_on(async {
        let (client, _recorder, tasks) = start_in_memory_with_options(
            Product::Collie,
            ResolvedProfile::for_product(Product::Collie, ProfileName::Code),
            store,
            None,
            hello_only_engine,
        )
        .await;

        let tools = client.list_tools(None).await.expect("list_tools");
        assert_eq!(tools.tools.len(), 0);

        let resources = client.list_resources(None).await.expect("list_resources");
        assert!(
            resources
                .resources
                .iter()
                .any(|r| r.uri == LLMS_TXT_URI && r.mime_type.as_deref() == Some("text/markdown"))
        );
        assert!(resources.resources.iter().any(|r| {
            r.uri == "goodfetch://docs/runs.md" && r.mime_type.as_deref() == Some("text/markdown")
        }));

        let (llms_text, _) = read_text_resource(&client, LLMS_TXT_URI).await;
        assert!(llms_text.contains("0.1.0") && llms_text.contains("goodfetch://docs/runs.md"));

        let (runs_text, runs_mime) = read_text_resource(&client, "goodfetch://docs/runs.md").await;
        assert_eq!(runs_mime.as_deref(), Some("text/markdown"));
        assert!(runs_text.contains("Every action runs inside a Run."));

        let skills = client.list_skills(None).await.expect("list_skills");
        assert_eq!(skills.skills.len(), 1);
        let listed = &skills.skills[0];
        listed.validate().expect("SEP-2640 skill validation");
        assert_eq!(listed.name(), "goodfetch-collie");
        assert_eq!(
            listed
                .meta
                .as_ref()
                .and_then(|m| m.get(SKILL_TREE_DIGEST_META_KEY))
                .and_then(Value::as_str),
            Some(expected_tree_digest.as_str())
        );

        let fetched = client
            .get_skill(GetSkillRequestParams::new(listed.uri.clone()))
            .await
            .expect("get_skill");
        assert_eq!(fetched.skill, *listed);

        let SkillResources::Files(ref manifest_files) = fetched.skill.resources else {
            panic!("expected static file manifest with digests");
        };
        assert_ne!(manifest_files.len(), 0);
        for entry in manifest_files {
            assert!(entry.digest.starts_with("sha256:"));
            let (text, _) = read_text_resource(&client, &entry.uri).await;
            entry
                .verify(text.as_bytes())
                .expect("skill file bytes must match manifest digest");
        }

        let _ = client.cancel().await;
        tasks.close();
        tasks.wait().await;
    });
}

#[derive(Clone, Default)]
struct RecordingRefusals {
    reasons: Arc<Mutex<Vec<goodfetch_protocol::audit::RefusalReason>>>,
}

impl transports::HttpRefusalRecorder for RecordingRefusals {
    fn record(
        &self,
        reason: goodfetch_protocol::audit::RefusalReason,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + '_>> {
        self.reasons.lock().unwrap().push(reason);
        Box::pin(std::future::ready(()))
    }
}

struct ScriptedSessionConnector {
    tasks: TaskTracker,
    next_session: Arc<std::sync::atomic::AtomicUsize>,
    per_session_limit: usize,
    hold_method: Option<&'static str>,
    cancelled_sessions: Arc<Mutex<Vec<usize>>>,
    release_held: Arc<tokio::sync::Notify>,
}

async fn run_scripted_session_actor(
    engine_stream: tokio::io::DuplexStream,
    session_num: usize,
    limit: usize,
    hold_method: Option<&'static str>,
    cancelled_sessions: Arc<Mutex<Vec<usize>>>,
    release_held: Arc<tokio::sync::Notify>,
) {
    let (read_half, mut write_half) = tokio::io::split(engine_stream);
    let mut lines = BufReader::new(read_half).lines();

    let Message::Request(hello_req) = read_message(&mut lines).await else {
        return;
    };
    write_line(&mut write_half, &hello_ok(&hello_req.id)).await;

    let mut calls = 0usize;
    while let Ok(Some(line)) = lines.next_line().await {
        if line.trim().is_empty() {
            continue;
        }
        let Ok(message) = serde_json::from_str::<Message>(&line) else {
            continue;
        };
        match message {
            Message::Request(req) if req.method == "events.subscribe" => {}
            Message::Notification(notif) if notif.method == "$/cancel" => {
                cancelled_sessions.lock().unwrap().push(session_num);
            }
            Message::Request(req) if hold_method == Some(req.method.as_str()) => {
                let cancel_fut = async {
                    while let Ok(Some(next_line)) = lines.next_line().await {
                        if let Ok(Message::Notification(notif)) =
                            serde_json::from_str::<Message>(&next_line)
                            && notif.method == "$/cancel"
                        {
                            cancelled_sessions.lock().unwrap().push(session_num);
                            return true;
                        }
                    }
                    false
                };
                tokio::select! {
                    biased;
                    cancelled = cancel_fut => {
                        if cancelled {
                            break;
                        }
                    }
                    () = release_held.notified() => {
                        write_line(
                            &mut write_half,
                            &json!({
                                "jsonrpc": "2.0",
                                "id": req.id,
                                "result": {
                                    "run_id": format!("held-session-{session_num}"),
                                    "lifecycle": "finished"
                                }
                            }),
                        )
                        .await;
                    }
                }
            }
            Message::Request(req) => {
                calls += 1;
                let reply = if calls > limit {
                    json!({
                        "jsonrpc": "2.0",
                        "id": req.id,
                        "error": {
                            "code": -32000,
                            "message": "Per-connection limit reached",
                            "data": {
                                "code": "limit_exceeded",
                                "retryable": false,
                                "details": {
                                    "fix": "Raise the limit with `goodfetch-collie usage raise`."
                                }
                            }
                        }
                    })
                } else {
                    json!({
                        "jsonrpc": "2.0",
                        "id": req.id,
                        "result": {
                            "run_id": format!("run-session-{session_num}-call-{calls}")
                        }
                    })
                };
                write_line(&mut write_half, &reply).await;
            }
            _ => {}
        }
    }
}

impl crate::engine::EngineConnector for ScriptedSessionConnector {
    fn connect(
        &self,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = Result<EngineLink, crate::engine::EngineLinkError>>
                + Send
                + '_,
        >,
    > {
        let session_num = self
            .next_session
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let (client_stream, engine_stream) = tokio::io::duplex(64 * 1024);

        self.tasks.spawn(run_scripted_session_actor(
            engine_stream,
            session_num,
            self.per_session_limit,
            self.hold_method,
            Arc::clone(&self.cancelled_sessions),
            Arc::clone(&self.release_held),
        ));

        Box::pin(async move {
            let (link, _) = EngineLink::connect(
                client_stream,
                EngineLinkConfig {
                    product: Product::Collie,
                    product_version: ProductVersion::parse("0.1.0").unwrap(),
                    token: None,
                    live_view_base_url: None,
                },
            )
            .await?;
            Ok(link)
        })
    }
}

/// Covers: story:11-220
#[test]
#[expect(
    clippy::too_many_lines,
    reason = "end-to-end Streamable HTTP refusal matrix verifies missing, wrong, query-string tokens, foreign Host/Origin, and rmcp client access"
)]
fn streamable_http_refuses_missing_wrong_query_token_and_foreign_host_or_origin() {
    use goodfetch_protocol::audit::RefusalReason;
    use rmcp::transport::StreamableHttpClientTransport;
    use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
    use secrecy::SecretString;
    use tokio_util::sync::CancellationToken;

    let rt = test_runtime();
    rt.block_on(async {
        let tasks = TaskTracker::new();
        let token = transports::new_launch_token().expect("OS random token");
        assert_eq!(token.len(), 64);

        let refusals = Arc::new(RecordingRefusals::default());
        let cancellation = CancellationToken::new();
        let connector = crate::engine::SharedEngineConnector::new(ScriptedSessionConnector {
            tasks: tasks.clone(),
            next_session: Arc::new(std::sync::atomic::AtomicUsize::new(1)),
            per_session_limit: 10,
            hold_method: None,
            cancelled_sessions: Arc::new(Mutex::new(Vec::new())),
            release_held: Arc::new(tokio::sync::Notify::new()),
        });
        let options = ServerOptions {
            product: Product::Collie,
            version: "0.1.0".to_owned(),
            profile: ResolvedProfile::for_product(Product::Collie, ProfileName::Browser),
            resources: crate::resources::ResourceStore::default(),
        };
        let bound = transports::BoundHttpServer::bind(
            transports::HttpServerConfig {
                bind: transports::DEFAULT_HTTP_BIND.parse().unwrap(),
                auth: transports::HttpAuth::LaunchToken(SecretString::from(token.clone())),
                tls: transports::HttpTls::Plain,
                refusals: Some(transports::SharedRefusalRecorder::new(RecordingRefusals {
                    reasons: Arc::clone(&refusals.reasons),
                })),
                cancellation: cancellation.clone(),
            },
            options,
            connector,
        )
        .await
        .expect("bind loopback MCP HTTP");
        assert!(bound.local_addr().ip().is_loopback());
        let url = bound.url().to_owned();
        assert!(url.starts_with("http://127.0.0.1:") && url.ends_with("/mcp"));

        tasks.spawn(async move {
            let _ = bound.serve().await;
        });

        let http = test_http_client(&[]);
        let init_body = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-11-25",
                "capabilities": {},
                "clientInfo": { "name": "http-refusal-test", "version": "1.0.0" }
            }
        });

        // 1. Missing token -> 401 + WWW-Authenticate: Bearer
        let resp = http
            .post(&url)
            .header("Accept", "application/json, text/event-stream")
            .json(&init_body)
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 401);
        assert_eq!(resp.headers().get("www-authenticate").unwrap(), "Bearer");

        // 2. Wrong token -> 401
        let resp = http
            .post(&url)
            .header("Authorization", "Bearer deadbeef")
            .header("Accept", "application/json, text/event-stream")
            .json(&init_body)
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 401);

        // 3. Token in query string only -> 401
        let resp = http
            .post(format!("{url}?access_token={token}"))
            .header("Accept", "application/json, text/event-stream")
            .json(&init_body)
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 401);

        // 4. Token in query string even with valid Authorization header -> 401
        let resp = http
            .post(format!("{url}?token={token}"))
            .header("Authorization", format!("Bearer {token}"))
            .header("Accept", "application/json, text/event-stream")
            .json(&init_body)
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 401);

        // 5. Foreign Host -> 403
        let resp = http
            .post(&url)
            .header("Host", "foreign.invalid")
            .header("Authorization", format!("Bearer {token}"))
            .header("Accept", "application/json, text/event-stream")
            .json(&init_body)
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 403);

        // 6. Foreign Origin -> 403
        let resp = http
            .post(&url)
            .header("Origin", "https://foreign.invalid")
            .header("Authorization", format!("Bearer {token}"))
            .header("Accept", "application/json, text/event-stream")
            .json(&init_body)
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 403);

        let recorded = refusals.reasons.lock().unwrap().clone();
        assert_eq!(
            recorded,
            vec![
                RefusalReason::TokenMissing,
                RefusalReason::TokenWrong,
                RefusalReason::TokenMissing,
                RefusalReason::TokenWrong,
                RefusalReason::HostForeign,
                RefusalReason::OriginForeign,
            ]
        );

        // 7. Valid rmcp Streamable HTTP client connects, lists tools, and calls a tool.
        let transport = StreamableHttpClientTransport::with_client(
            http,
            StreamableHttpClientTransportConfig::with_uri(url).auth_header(token),
        );
        let client = ().serve(transport).await.expect("rmcp http initialized");
        let tools = client.list_all_tools().await.expect("list_all_tools");
        assert!(tools.iter().any(|t| t.name == "run_start"));
        let result = client
            .call_tool(CallToolRequestParams::new("run_start").with_arguments(Map::new()))
            .await
            .expect("call run_start");
        assert_ne!(result.is_error, Some(true));
        assert_eq!(
            result.structured_content.as_ref().unwrap()["run_id"],
            "run-session-1-call-1"
        );

        let _ = client.cancel().await;
        cancellation.cancel();
        tasks.close();
        tasks.wait().await;
    });
}

/// Covers: story:11-231
#[test]
#[expect(
    clippy::too_many_lines,
    reason = "verifies per-session Limit isolation and independent cancellation across two concurrent Streamable HTTP sessions"
)]
fn streamable_http_two_concurrent_sessions_keep_independent_limits_and_cancellation() {
    use rmcp::transport::StreamableHttpClientTransport;
    use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
    use secrecy::SecretString;
    use tokio_util::sync::CancellationToken;

    let rt = test_runtime();
    rt.block_on(async {
        let tasks = TaskTracker::new();
        let token = transports::new_launch_token().expect("OS random token");
        let cancellation = CancellationToken::new();
        let cancelled_sessions = Arc::new(Mutex::new(Vec::new()));
        let release_held = Arc::new(tokio::sync::Notify::new());
        let connector = crate::engine::SharedEngineConnector::new(ScriptedSessionConnector {
            tasks: tasks.clone(),
            next_session: Arc::new(std::sync::atomic::AtomicUsize::new(1)),
            per_session_limit: 1,
            hold_method: Some("run.finish"),
            cancelled_sessions: Arc::clone(&cancelled_sessions),
            release_held: Arc::clone(&release_held),
        });
        let options = ServerOptions {
            product: Product::Collie,
            version: "0.1.0".to_owned(),
            profile: ResolvedProfile::for_product(Product::Collie, ProfileName::Browser),
            resources: crate::resources::ResourceStore::default(),
        };
        let bound = transports::BoundHttpServer::bind(
            transports::HttpServerConfig {
                bind: transports::DEFAULT_HTTP_BIND.parse().unwrap(),
                auth: transports::HttpAuth::LaunchToken(SecretString::from(token.clone())),
                tls: transports::HttpTls::Plain,
                refusals: None,
                cancellation: cancellation.clone(),
            },
            options,
            connector,
        )
        .await
        .expect("bind loopback MCP HTTP");
        let url = bound.url().to_owned();

        tasks.spawn(async move {
            let _ = bound.serve().await;
        });

        let client_a = ()
            .serve(StreamableHttpClientTransport::with_client(
                test_http_client(&[]),
                StreamableHttpClientTransportConfig::with_uri(url.clone())
                    .auth_header(token.clone()),
            ))
            .await
            .expect("client A initialized");

        let client_b = ()
            .serve(StreamableHttpClientTransport::with_client(
                test_http_client(&[]),
                StreamableHttpClientTransportConfig::with_uri(url).auth_header(token),
            ))
            .await
            .expect("client B initialized");

        // 1. Session A spends its allowance (1 call), then hits `limit_exceeded`.
        let a_ok = client_a
            .call_tool(CallToolRequestParams::new("run_start").with_arguments(Map::new()))
            .await
            .expect("session A call 1");
        assert_ne!(a_ok.is_error, Some(true));
        let a_limit = client_a
            .call_tool(CallToolRequestParams::new("run_start").with_arguments(Map::new()))
            .await
            .expect("session A call 2 returns isError");
        assert_eq!(a_limit.is_error, Some(true));
        assert_eq!(
            a_limit.structured_content.as_ref().unwrap()["code"],
            "limit_exceeded"
        );

        // 2. Session B keeps its own independent allowance: its first call succeeds,
        // and only its second call hits its own limit.
        let b_ok = client_b
            .call_tool(CallToolRequestParams::new("run_start").with_arguments(Map::new()))
            .await
            .expect("session B call 1");
        assert_ne!(b_ok.is_error, Some(true));
        assert_ne!(
            a_ok.structured_content.as_ref().unwrap()["run_id"],
            b_ok.structured_content.as_ref().unwrap()["run_id"],
            "each session must use its own engine Connection"
        );
        let b_limit = client_b
            .call_tool(CallToolRequestParams::new("run_start").with_arguments(Map::new()))
            .await
            .expect("session B call 2 returns isError");
        assert_eq!(b_limit.is_error, Some(true));
        assert_eq!(
            b_limit.structured_content.as_ref().unwrap()["code"],
            "limit_exceeded"
        );

        // 3. Independent cancellation: both sessions start a held `run_finish` call;
        // session A cancels via PeerRequestOptions timeout while session B completes.
        let mut args_a = Map::new();
        args_a.insert("run_id".to_owned(), Value::String("r-a".to_owned()));
        let mut args_b = Map::new();
        args_b.insert("run_id".to_owned(), Value::String("r-b".to_owned()));

        let peer_a = client_a.peer().clone();
        let peer_b = client_b.peer().clone();
        let handle_a = peer_a
            .send_cancellable_request(
                ClientRequest::CallToolRequest(CallToolRequest::new(
                    CallToolRequestParams::new("run_finish").with_arguments(args_a),
                )),
                PeerRequestOptions::no_options(),
            )
            .await
            .expect("send request A");
        let handle_b = peer_b
            .send_cancellable_request(
                ClientRequest::CallToolRequest(CallToolRequest::new(
                    CallToolRequestParams::new("run_finish").with_arguments(args_b),
                )),
                PeerRequestOptions::no_options(),
            )
            .await
            .expect("send request B");

        // Wait briefly so both calls reach their respective engine connections, then cancel A only.
        tokio::time::sleep(Duration::from_millis(100)).await;
        handle_a.cancel(None).await.expect("cancel request A");

        // Poll until $/cancel reaches session A's engine actor.
        let mut saw_cancel = false;
        for _ in 0..40 {
            if !cancelled_sessions.lock().unwrap().is_empty() {
                saw_cancel = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        assert!(
            saw_cancel,
            "session A's engine Connection must receive $/cancel"
        );
        assert_eq!(
            cancelled_sessions.lock().unwrap().as_slice(),
            &[1],
            "only session A's engine Connection should receive $/cancel"
        );
        release_held.notify_waiters();

        let res_b = handle_b
            .await_response()
            .await
            .expect("session B call completes unaffected");
        let rmcp::model::ServerResult::CallToolResult(tool_b) = res_b else {
            panic!("expected CallToolResult");
        };
        assert_ne!(tool_b.is_error, Some(true));
        assert_eq!(
            tool_b.structured_content.as_ref().unwrap()["run_id"],
            "held-session-2"
        );

        let _ = client_a.cancel().await;
        let _ = client_b.cancel().await;
        cancellation.cancel();
        tasks.close();
        tasks.wait().await;
    });
}

#[expect(
    clippy::disallowed_methods,
    reason = "unit test builds a reqwest client with explicit ring rustls config"
)]
fn test_http_client(extra_roots: &[rustls::pki_types::CertificateDer<'static>]) -> reqwest::Client {
    let mut roots = rustls::RootCertStore::empty();
    for cert in extra_roots {
        roots.add(cert.clone()).unwrap();
    }
    let tls = rustls::ClientConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .unwrap()
    .with_root_certificates(roots)
    .with_no_client_auth();
    reqwest::Client::builder()
        .tls_backend_preconfigured(tls)
        .no_proxy()
        .build()
        .unwrap()
}

/// Local OAuth 2.1 authorization server stub serving RFC 8414 metadata and a
/// JWKS with both Ed25519 (`EdDSA`) and P-256 (`ES256`) signing keys.
struct LocalAuthServerStub {
    issuer: String,
    ed_key: ed25519_dalek::SigningKey,
    ed_kid: String,
    ec_key: p256::ecdsa::SigningKey,
    ec_kid: String,
    cancellation: tokio_util::sync::CancellationToken,
}

impl LocalAuthServerStub {
    #[expect(
        clippy::disallowed_types,
        reason = "test authorization server stub opens a loopback TCP listener"
    )]
    async fn start(tasks: &TaskTracker) -> Self {
        use bytes::Bytes;
        use data_encoding::BASE64URL_NOPAD;
        use http::{Request, Response, StatusCode, header};
        use http_body_util::Full;
        use hyper::body::Incoming;
        use hyper_util::rt::TokioIo;
        use std::convert::Infallible;

        let ed_key = ed25519_dalek::SigningKey::from_bytes(&[0x42u8; 32]);
        let ed_x = BASE64URL_NOPAD.encode(ed_key.verifying_key().as_bytes());
        let ed_kid = "stub-ed25519-1".to_owned();

        let p256_key = p256::ecdsa::SigningKey::from_slice(&[0x24u8; 32]).unwrap();
        let p256_point = p256_key.verifying_key().to_sec1_point(false);
        let p256_x = BASE64URL_NOPAD.encode(p256_point.x().unwrap().as_slice());
        let p256_y = BASE64URL_NOPAD.encode(p256_point.y().unwrap().as_slice());
        let p256_kid = "stub-es256-1".to_owned();

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let issuer = format!("http://{addr}");
        let metadata = Arc::new(
            serde_json::to_vec(&json!({
                "issuer": issuer,
                "jwks_uri": format!("{issuer}/jwks.json"),
            }))
            .unwrap(),
        );
        let jwks = Arc::new(
            serde_json::to_vec(&json!({
                "keys": [
                    { "kty": "OKP", "crv": "Ed25519", "kid": ed_kid, "use": "sig", "alg": "EdDSA", "x": ed_x },
                    { "kty": "EC", "crv": "P-256", "kid": p256_kid, "use": "sig", "alg": "ES256", "x": p256_x, "y": p256_y }
                ]
            }))
            .unwrap(),
        );
        let cancellation = tokio_util::sync::CancellationToken::new();
        let stop = cancellation.clone();
        let conn_tasks = tasks.clone();
        tasks.spawn(async move {
            loop {
                let accepted = tokio::select! {
                    () = stop.cancelled() => break,
                    res = listener.accept() => res,
                };
                let Ok((stream, _)) = accepted else { break };
                let (metadata, jwks) = (Arc::clone(&metadata), Arc::clone(&jwks));
                conn_tasks.spawn(async move {
                    let _ = hyper::server::conn::http1::Builder::new()
                        .serve_connection(
                            TokioIo::new(stream),
                            hyper::service::service_fn(move |req: Request<Incoming>| {
                                let (metadata, jwks) = (Arc::clone(&metadata), Arc::clone(&jwks));
                                async move {
                                    let (status, body) = match req.uri().path() {
                                        "/.well-known/oauth-authorization-server" => {
                                            (StatusCode::OK, (*metadata).clone())
                                        }
                                        "/jwks.json" => (StatusCode::OK, (*jwks).clone()),
                                        _ => (StatusCode::NOT_FOUND, b"{}".to_vec()),
                                    };
                                    let mut resp = Response::new(Full::new(Bytes::from(body)));
                                    *resp.status_mut() = status;
                                    resp.headers_mut().insert(
                                        header::CONTENT_TYPE,
                                        http::HeaderValue::from_static("application/json"),
                                    );
                                    Ok::<_, Infallible>(resp)
                                }
                            }),
                        )
                        .await;
                });
            }
        });

        Self {
            issuer,
            ed_key,
            ed_kid,
            ec_key: p256_key,
            ec_kid: p256_kid,
            cancellation,
        }
    }

    fn sign_eddsa(&self, claims: &Value) -> String {
        use data_encoding::BASE64URL_NOPAD;
        use ed25519_dalek::Signer as _;
        let header = json!({ "alg": "EdDSA", "typ": "JWT", "kid": self.ed_kid });
        let input = format!(
            "{}.{}",
            BASE64URL_NOPAD.encode(header.to_string().as_bytes()),
            BASE64URL_NOPAD.encode(claims.to_string().as_bytes())
        );
        let sig = self.ed_key.sign(input.as_bytes());
        format!("{input}.{}", BASE64URL_NOPAD.encode(&sig.to_bytes()))
    }

    fn sign_es256(&self, claims: &Value) -> String {
        use data_encoding::BASE64URL_NOPAD;
        use p256::ecdsa::signature::Signer as _;
        let header = json!({ "alg": "ES256", "typ": "JWT", "kid": self.ec_kid });
        let input = format!(
            "{}.{}",
            BASE64URL_NOPAD.encode(header.to_string().as_bytes()),
            BASE64URL_NOPAD.encode(claims.to_string().as_bytes())
        );
        let sig: p256::ecdsa::Signature = self.ec_key.sign(input.as_bytes());
        format!("{input}.{}", BASE64URL_NOPAD.encode(&sig.to_bytes()))
    }
}

struct RecordingEngineConnector {
    tasks: TaskTracker,
    recorded_wire: Arc<Mutex<Vec<String>>>,
}

impl crate::engine::EngineConnector for RecordingEngineConnector {
    fn connect(
        &self,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<
                    Output = Result<crate::engine::EngineLink, crate::engine::EngineLinkError>,
                > + Send
                + '_,
        >,
    > {
        let tasks = self.tasks.clone();
        let recorded = Arc::clone(&self.recorded_wire);
        Box::pin(async move {
            let (client_io, engine_io) = tokio::io::duplex(64 * 1024);
            let (engine_read, mut engine_write) = tokio::io::split(engine_io);
            let (client_read, client_write) = tokio::io::split(client_io);

            tasks.spawn(async move {
                use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};
                let mut lines = BufReader::new(engine_read).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    recorded.lock().unwrap().push(line.clone());
                    let msg: Value = serde_json::from_str(&line).unwrap();
                    let method = msg.get("method").and_then(Value::as_str).unwrap_or("");
                    let Some(id) = msg.get("id").cloned() else {
                        continue;
                    };
                    let reply = match method {
                        "hello" => json!({
                            "jsonrpc": "2.0",
                            "id": id,
                            "result": {
                                "protocol_version": goodfetch_protocol::catalog::PROTOCOL_VERSION.to_string(),
                                "product": "collie",
                                "product_version": "0.1.0",
                                "engine_instance": "attached",
                                "max_calls_in_flight": 16,
                                "timeouts": []
                            }
                        }),
                        "run.start" => json!({
                            "jsonrpc": "2.0",
                            "id": id,
                            "result": { "run_id": "run-oauth-verified" }
                        }),
                        other => json!({
                            "jsonrpc": "2.0",
                            "id": id,
                            "error": {
                                "code": -32001,
                                "message": format!("unexpected method {other}"),
                                "data": {
                                    "code": "method_not_found",
                                    "group": "program",
                                    "retryable": false
                                }
                            }
                        }),
                    };
                    let mut out = serde_json::to_vec(&reply).unwrap();
                    out.push(b'\n');
                    if engine_write.write_all(&out).await.is_err() {
                        break;
                    }
                    let _ = engine_write.flush().await;
                }
            });

            let (link, _hello) = crate::engine::EngineLink::connect_split(
                client_read,
                client_write,
                crate::engine::EngineLinkConfig {
                    product: Product::Collie,
                    product_version: goodfetch_protocol::connection::ProductVersion::parse("0.1.0")
                        .unwrap(),
                    token: Some("internal-engine-only-token".to_owned()),
                    live_view_base_url: None,
                },
            )
            .await?;
            Ok(link)
        })
    }
}

/// Covers: story:11-221, setting:mcp.oauth.issuer, setting:mcp.oauth.audience
#[test]
fn oauth_config_validates_issuer_and_audience_and_builds_rfc9728_metadata() {
    use crate::oauth::OAuthConfig;

    let bad_issuer = OAuthConfig::new("http://auth.example.com", "https://mcp.example.com/mcp")
        .expect_err("non-loopback HTTP issuer must be refused");
    assert_eq!(bad_issuer.key, "mcp.oauth.issuer");

    let empty_issuer = OAuthConfig::new("", "https://mcp.example.com/mcp").unwrap_err();
    assert_eq!(empty_issuer.key, "mcp.oauth.issuer");

    let fragment_aud = OAuthConfig::new(
        "https://auth.example.com",
        "https://mcp.example.com/mcp#frag",
    )
    .unwrap_err();
    assert_eq!(fragment_aud.key, "mcp.oauth.audience");

    let config = OAuthConfig::new("https://auth.example.com", "https://mcp.example.com/mcp")
        .expect("valid HTTPS OAuth config");
    let metadata = config.protected_resource_metadata();
    assert_eq!(metadata.resource, "https://mcp.example.com/mcp");
    assert_eq!(
        metadata.authorization_servers,
        vec!["https://auth.example.com"]
    );
    assert_eq!(metadata.bearer_methods_supported, vec!["header"]);
    assert_eq!(
        config.resource_metadata_url(),
        "https://mcp.example.com/.well-known/oauth-protected-resource/mcp"
    );
    assert!(config.matches_metadata_path("/.well-known/oauth-protected-resource"));
    assert!(config.matches_metadata_path("/.well-known/oauth-protected-resource/mcp"));
}

/// Covers: story:11-221, setting:mcp.oauth.issuer, setting:mcp.oauth.audience
#[test]
#[expect(
    clippy::disallowed_types,
    reason = "integration test opens one raw plain TCP socket to prove the TLS listener refuses plaintext with tls_required"
)]
#[expect(
    clippy::too_many_lines,
    reason = "verifies TLS enforcement, RFC 9728 metadata, issuer/audience/exp/missing/signature rejections, rmcp HTTPS client, and no token passthrough"
)]
fn remote_mcp_tls_and_oauth_enforces_metadata_audience_issuer_exp_and_no_passthrough() {
    use crate::oauth::{OAuthConfig, OAuthResourceServer};
    use goodfetch_protocol::audit::RefusalReason;
    use rmcp::transport::StreamableHttpClientTransport;
    use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
    use tokio_util::sync::CancellationToken;

    let rt = test_runtime();
    rt.block_on(async {
        let tasks = TaskTracker::new();
        let stub = LocalAuthServerStub::start(&tasks).await;

        // Generate a self-signed TLS certificate for `localhost` / `127.0.0.1`.
        let key_pair = rcgen::KeyPair::generate().unwrap();
        let cert = rcgen::CertificateParams::new(vec![
            "localhost".to_owned(),
            "127.0.0.1".to_owned(),
        ])
        .unwrap()
        .self_signed(&key_pair)
        .unwrap();
        let cert_der = cert.der().clone();
        let key_der = rustls::pki_types::PrivateKeyDer::Pkcs8(
            rustls::pki_types::PrivatePkcs8KeyDer::from(key_pair.serialize_der()),
        );
        let server_tls = rustls::ServerConfig::builder_with_provider(Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(vec![cert_der.clone()], key_der)
        .unwrap();
        let tls_acceptor = transports::SharedTlsAcceptor::new(transports::StaticTlsAcceptor(
            tokio_rustls::TlsAcceptor::from(Arc::new(server_tls)),
        ));

        let fixed_now: u64 = 1_800_000_000;
        let audience = "https://mcp.remote.test/mcp";
        let oauth_config = OAuthConfig::new(&stub.issuer, audience).unwrap();
        let fetcher =
            crate::oauth::SharedOAuthMetadataFetcher::new(transports::HttpOAuthFetcher::new(&[]));
        let oauth = Arc::new(OAuthResourceServer::new(
            oauth_config,
            fetcher,
            crate::oauth::EpochSecondsClock::new(move || fixed_now),
        ));

        let refusals = Arc::new(RecordingRefusals::default());
        let recorded_wire = Arc::new(Mutex::new(Vec::new()));
        let connector = crate::engine::SharedEngineConnector::new(RecordingEngineConnector {
            tasks: tasks.clone(),
            recorded_wire: Arc::clone(&recorded_wire),
        });
        let options = ServerOptions {
            product: Product::Collie,
            version: "0.1.0".to_owned(),
            profile: ResolvedProfile::for_product(Product::Collie, ProfileName::Browser),
            resources: crate::resources::ResourceStore::default(),
        };
        let cancellation = CancellationToken::new();
        let bound = transports::BoundHttpServer::bind(
            transports::HttpServerConfig {
                bind: "0.0.0.0:0".parse().unwrap(),
                auth: transports::HttpAuth::OAuth(Arc::clone(&oauth)),
                tls: transports::HttpTls::Direct(tls_acceptor),
                refusals: Some(transports::SharedRefusalRecorder::new(RecordingRefusals {
                    reasons: Arc::clone(&refusals.reasons),
                })),
                cancellation: cancellation.clone(),
            },
            options,
            connector,
        )
        .await
        .expect("bind remote TLS + OAuth MCP server");
        let port = bound.local_addr().port();
        assert!(bound.url().starts_with("https://"));

        tasks.spawn(async move {
            let _ = bound.serve().await;
        });

        // 1. Plaintext HTTP attempt on the TLS listener is closed unanswered with `tls_required`.
        let mut plain = tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .unwrap();
        plain
            .write_all(b"GET /mcp HTTP/1.1\r\nHost: localhost\r\n\r\n")
            .await
            .unwrap();
        let mut buf = [0u8; 16];
        match plain.read(&mut buf).await {
            Ok(0) => {}
            Err(err) if err.kind() == std::io::ErrorKind::ConnectionReset => {}
            other => panic!("TLS listener must close plaintext connection unanswered, got {other:?}"),
        }

        let https = test_http_client(&[cert_der]);
        let base = format!("https://localhost:{port}");
        let mcp_url = format!("{base}/mcp");

        // 2. RFC 9728 Protected Resource Metadata is served without auth and names issuer + audience.
        for path in [
            "/.well-known/oauth-protected-resource",
            "/.well-known/oauth-protected-resource/mcp",
        ] {
            let resp = https.get(format!("{base}{path}")).send().await.unwrap();
            assert_eq!(resp.status(), 200, "metadata path {path}");
            let meta: Value = resp.json().await.unwrap();
            assert_eq!(meta["resource"], audience);
            assert_eq!(meta["authorization_servers"], json!([stub.issuer]));
            assert_eq!(meta["bearer_methods_supported"], json!(["header"]));
        }

        let init_body = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-11-25",
                "capabilities": {},
                "clientInfo": { "name": "oauth-test", "version": "1.0.0" }
            }
        });

        // 3. Missing token -> 401 with RFC 9728 `resource_metadata` in WWW-Authenticate.
        let resp = https
            .post(&mcp_url)
            .header("Accept", "application/json, text/event-stream")
            .json(&init_body)
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 401);
        let www_auth = resp
            .headers()
            .get("www-authenticate")
            .unwrap()
            .to_str()
            .unwrap();
        assert!(
            www_auth.contains("resource_metadata=\"https://mcp.remote.test/.well-known/oauth-protected-resource/mcp\""),
            "unexpected WWW-Authenticate: {www_auth}"
        );

        // 4. Token from another issuer -> 401.
        let wrong_iss = stub.sign_eddsa(&json!({
            "iss": "https://other-issuer.invalid",
            "aud": audience,
            "exp": fixed_now + 600,
            "sub": "user-1"
        }));
        let resp = https
            .post(&mcp_url)
            .header("Authorization", format!("Bearer {wrong_iss}"))
            .header("Accept", "application/json, text/event-stream")
            .json(&init_body)
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 401);

        // 5. Token of another audience -> 401.
        let wrong_aud = stub.sign_eddsa(&json!({
            "iss": stub.issuer,
            "aud": "https://other-service.invalid/mcp",
            "exp": fixed_now + 600,
            "sub": "user-1"
        }));
        let resp = https
            .post(&mcp_url)
            .header("Authorization", format!("Bearer {wrong_aud}"))
            .header("Accept", "application/json, text/event-stream")
            .json(&init_body)
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 401);

        // 6. Expired token -> 401.
        let expired = stub.sign_eddsa(&json!({
            "iss": stub.issuer,
            "aud": audience,
            "exp": fixed_now - 1,
            "sub": "user-1"
        }));
        let resp = https
            .post(&mcp_url)
            .header("Authorization", format!("Bearer {expired}"))
            .header("Accept", "application/json, text/event-stream")
            .json(&init_body)
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 401);

        // 7. `alg: none` token -> 401.
        let alg_none = format!(
            "{}.{}.sig",
            data_encoding::BASE64URL_NOPAD.encode(b"{\"alg\":\"none\",\"typ\":\"JWT\"}"),
            data_encoding::BASE64URL_NOPAD.encode(
                json!({ "iss": stub.issuer, "aud": audience, "exp": fixed_now + 600 })
                    .to_string()
                    .as_bytes()
            )
        );
        let resp = https
            .post(&mcp_url)
            .header("Authorization", format!("Bearer {alg_none}"))
            .header("Accept", "application/json, text/event-stream")
            .json(&init_body)
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 401);

        // 8. Valid EdDSA token -> rmcp Streamable HTTP client succeeds over HTTPS.
        let valid_eddsa = stub.sign_eddsa(&json!({
            "iss": stub.issuer,
            "aud": audience,
            "exp": fixed_now + 600,
            "sub": "agent-client"
        }));
        let client_ed = ()
            .serve(StreamableHttpClientTransport::with_client(
                https.clone(),
                StreamableHttpClientTransportConfig::with_uri(mcp_url.clone())
                    .auth_header(valid_eddsa.clone()),
            ))
            .await
            .expect("rmcp HTTPS client with valid EdDSA token");
        let started = client_ed
            .call_tool(CallToolRequestParams::new("run_start").with_arguments(Map::new()))
            .await
            .expect("run_start with EdDSA token");
        assert_ne!(started.is_error, Some(true));
        let _ = client_ed.cancel().await;

        // 9. Valid ES256 token with array `aud` -> rmcp HTTPS client succeeds.
        let valid_es256 = stub.sign_es256(&json!({
            "iss": stub.issuer,
            "aud": ["https://other.example.test", audience],
            "exp": fixed_now + 600,
            "sub": "agent-client-ec"
        }));
        let p256_client = ()
            .serve(StreamableHttpClientTransport::with_client(
                https,
                StreamableHttpClientTransportConfig::with_uri(mcp_url)
                    .auth_header(valid_es256.clone()),
            ))
            .await
            .expect("rmcp HTTPS client with valid ES256 token");
        let tools = p256_client.list_all_tools().await.expect("list_all_tools");
        assert!(tools.iter().any(|t| t.name == "run_start"));
        let _ = p256_client.cancel().await;

        // 10. Prove NO OAuth bearer token (or any segment of it) ever reached the engine.
        let wire_messages = recorded_wire.lock().unwrap().clone();
        assert!(
            wire_messages.iter().any(|line| line.contains("\"hello\"")),
            "engine must have received hello"
        );
        assert!(
            wire_messages.iter().any(|line| line.contains("\"run.start\"")),
            "engine must have received run.start"
        );
        for token in [&valid_eddsa, &valid_es256] {
            for segment in std::iter::once(token.as_str()).chain(token.split('.')) {
                assert!(
                    wire_messages.iter().all(|line| !line.contains(segment)),
                    "OAuth token segment leaked into engine wire message: {wire_messages:?}"
                );
            }
        }

        // Verify Audit refusals recorded TlsRequired + TokenMissing + TokenWrong.
        let reasons = refusals.reasons.lock().unwrap().clone();
        assert!(reasons.contains(&RefusalReason::TlsRequired));
        assert!(reasons.contains(&RefusalReason::TokenMissing));
        assert!(reasons.contains(&RefusalReason::TokenWrong));

        stub.cancellation.cancel();
        cancellation.cancel();
        tasks.close();
        tasks.wait().await;
    });
}

/// Regression test for S5: `EngineLinkConfig`'s `Debug` redacts `token`.
#[test]
fn engine_link_config_debug_redacts_token() {
    let secret = "gf_super_secret_daemon_token_canary_987654321";
    let config = EngineLinkConfig {
        product: Product::Collie,
        product_version: ProductVersion::parse("0.1.0").unwrap(),
        token: Some(secret.to_owned()),
        live_view_base_url: Some("http://127.0.0.1:7601".to_owned()),
    };
    let rendered = format!("{config:?}");
    assert!(
        !rendered.contains(secret),
        "EngineLinkConfig Debug leaked secret token: {rendered}"
    );
    assert!(
        rendered.contains("[redacted]"),
        "EngineLinkConfig Debug should show redaction marker: {rendered}"
    );
}

/// Regression test for S8: `connection_lost` maps to `ErrorGroup::Unavailable` (`-32006`),
/// not `InternalError` (`-32603`).
#[test]
fn dropped_engine_connection_returns_unavailable_connection_lost_code() {
    use goodfetch_protocol::catalog::ErrorGroup;

    let rt = test_runtime();
    rt.block_on(async {
        let (client, _recorder, tasks) = start_in_memory(None, |stream| async move {
            let (read, mut writer) = tokio::io::split(stream);
            let mut reader = BufReader::new(read);
            let mut line = String::new();
            while reader.read_line(&mut line).await.unwrap_or(0) > 0 {
                let msg: Message = serde_json::from_str(line.trim()).unwrap();
                line.clear();
                if let Message::Request(req) = msg {
                    match req.method.as_str() {
                        "hello" => write_line(&mut writer, &hello_ok(&req.id)).await,
                        "events.subscribe" => {
                            write_line(
                                &mut writer,
                                &json!({"jsonrpc": "2.0", "id": req.id, "result": {}}),
                            )
                            .await;
                        }
                        "run.start" => {
                            // Drop connection abruptly while call is in flight.
                            return;
                        }
                        _ => {}
                    }
                }
            }
        })
        .await;

        let result = client
            .call_tool(CallToolRequestParams::new("run_start").with_arguments(Map::new()))
            .await
            .expect("tool error returns CallToolResult");
        assert_eq!(result.is_error, Some(true));
        let structured = result
            .structured_content
            .as_ref()
            .expect("structuredContent");
        assert_eq!(structured["code"], "connection_lost");
        assert_eq!(structured["group"], ErrorGroup::Unavailable.as_str());
        assert_eq!(structured["retryable"], true);

        let _ = client.cancel().await;
        tasks.close();
        tasks.wait().await;
    });
}

/// Regression test for S3: `HttpOAuthFetcher` reads >32 KiB multi-chunk HTTP/1.1
/// responses without deadlocking `conn` against `into_body().collect()`.
#[test]
fn http_oauth_fetcher_reads_large_chunked_jwks_without_deadlock() {
    use crate::oauth::OAuthMetadataFetcher as _;

    let rt = test_runtime();
    rt.block_on(async {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        // Build a ~40 KiB JSON JWKS payload split across multiple HTTP/1.1 chunks.
        let mut keys = Vec::new();
        for i in 0..80 {
            keys.push(json!({
                "kty": "OKP",
                "crv": "Ed25519",
                "kid": format!("large-key-{i:03}"),
                "x": "A".repeat(440),
            }));
        }
        let body = serde_json::to_vec(&json!({ "keys": keys })).unwrap();
        assert!(body.len() > 32 * 1024, "payload must exceed 32 KiB: {}", body.len());

        let tasks = TaskTracker::new();
        tasks.spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buf = [0u8; 2048];
            let _ = tokio::io::AsyncReadExt::read(&mut socket, &mut buf).await;
            let header = "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n";
            socket.write_all(header.as_bytes()).await.unwrap();
            for chunk in body.chunks(2048) {
                let prefix = format!("{:x}\r\n", chunk.len());
                socket.write_all(prefix.as_bytes()).await.unwrap();
                socket.write_all(chunk).await.unwrap();
                socket.write_all(b"\r\n").await.unwrap();
                socket.flush().await.unwrap();
            }
            socket.write_all(b"0\r\n\r\n").await.unwrap();
            socket.flush().await.unwrap();
        });

        let fetcher = transports::HttpOAuthFetcher::new(&[]);
        let jwks_doc = tokio::time::timeout(
            Duration::from_secs(5),
            fetcher.fetch_json(format!("http://{addr}/jwks.json")),
        )
        .await
        .expect("large chunked JWKS fetch must not deadlock")
        .expect("large chunked JWKS fetch must succeed");
        assert_eq!(jwks_doc["keys"].as_array().unwrap().len(), 80);
        tasks.close();
        tasks.wait().await;
    });
}

struct StaticJwksFetcher {
    issuer: String,
    jwks: Value,
}

impl crate::oauth::OAuthMetadataFetcher for StaticJwksFetcher {
    fn fetch_json(
        &self,
        url: String,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Value, String>> + Send + '_>>
    {
        Box::pin(async move {
            if url.contains("jwks") {
                Ok(self.jwks.clone())
            } else {
                Ok(json!({
                    "issuer": self.issuer,
                    "jwks_uri": format!("{}/jwks.json", self.issuer),
                }))
            }
        })
    }
}

fn sign_rs256_jwt(pkcs1_b64: &str, kid: &str, claims: &Value) -> String {
    use data_encoding::{BASE64, BASE64URL_NOPAD};
    use rustls::SignatureScheme;
    use rustls::pki_types::PrivateKeyDer;

    let der = BASE64.decode(pkcs1_b64.as_bytes()).unwrap();
    let provider = rustls::crypto::ring::default_provider();
    let key = provider
        .key_provider
        .load_private_key(PrivateKeyDer::Pkcs1(der.into()))
        .unwrap();
    let signer = key
        .choose_scheme(&[SignatureScheme::RSA_PKCS1_SHA256])
        .expect("RS256 signer");
    let header = BASE64URL_NOPAD.encode(
        serde_json::to_vec(&json!({ "alg": "RS256", "typ": "JWT", "kid": kid }))
            .unwrap()
            .as_slice(),
    );
    let payload = BASE64URL_NOPAD.encode(serde_json::to_vec(claims).unwrap().as_slice());
    let signing_input = format!("{header}.{payload}");
    let sig = signer.sign(signing_input.as_bytes()).unwrap();
    format!("{signing_input}.{}", BASE64URL_NOPAD.encode(&sig))
}

/// Coverage for S12: `verify_rs256` and `encode_rsa_public_key_der` with real
/// RSA-2048 keys (valid token, wrong signature, tampered payload, wrong key/kid).
#[test]
fn oauth_verifies_rs256_and_rejects_tampered_or_wrong_key() {
    use crate::oauth::{
        EpochSecondsClock, OAuthConfig, OAuthResourceServer, SharedOAuthMetadataFetcher,
        TokenRejection,
    };
    use data_encoding::BASE64URL_NOPAD;

    const RSA_K1_PKCS8_B64: &str = "MIIEowIBAAKCAQEAkre1N8y5cpV2303DY7heCvsRoTcSxMD1HXNljS7MD3LFV8l0UN24t/poMYLBo66iO77c/jgnN7LBSmi4F+nObG/lMQa5z4LdthPOpWnmSP18uHfNO9FhI0iis/5w4sMD5L+db1lu9NHJlxcD/MC25h4dQleRRaW6nKW5n3kYl/kTFXVTGj9g9UQBXqX2O3iPfApHT7hetoy0TUo/zeml3cw39qj4UnPjkZ1u3oL+R3hrj60m1v9Hg4anhmMvibSs67C6y0jJBrpjm70i3ckGMbLWFhrs+/+xrLQ738KMIvWcJ0ngG3gkxOM2/8nlxgap8qV5Rpkhjdurs6JOVFA9QwIDAQABAoIBAB7a2JcMDGIPEclDathxYQGxLFN5qA5NwB658nze/xYunlbobHhHML4sQLZM19lK9B5ZdNNXztOVWbYC0NhimMFQRORrFC+SR3svrjE2VLI78PtAYp52WZXYhDMTSv09Yve73mmY5s9AwULvmayRgMVjRub5UHlTsauY5LEnf+cRrn7r5tbUstidcvsS93JQMz1CIuf/Sw4LisGticSxkqa4luWperTcUBFec2Se8M/llka3Ff+FYEFsrjj1PqIO9EHoGz19HzXAxJUBk0vyxny8qrraFiT6/6r6b6cD/1641jWclGiUbTxnrhPIqsApraiw8fxfDVp1qwNMPAs8g00CgYEAyeIlolJu5olaN8bCBW5tBOq2BxnjByK61Mpo9+0Hzyy9gqcdmKL6qbCh69JXMCPjbTMUf5Yv1g1Id+soQ4+twnerEWCG9hZV5mxSPmU3hCUAYxeXGJ+W9UBixdmstOMbNVgmAZLEKMfIx0Cp7Cz1nLAcDieTIkQj36zPwpnKkfUCgYEAugvqykwZhK2RXOMNXGMkH5SjPCF45Z4xFqnaR35NWXVYkRFQSDqmdw8jowM2TQqeu4tecAVDdlnCrG6nCn0Tgzqvk5mJ/GrvzvZygvcAjjcsb7cqT5Tc7Ld3wj6FPf+N7ryWnIvAlySMHIZzQuxMnRB8iK0gD+JEow/Ma1OJN1cCgYB7KAalW3xkDu6GkC3UemTvVu/srRIeTEWFgmU+uc7FyBlkFuarMuGPmz1sS0j9CSe4eizLGKfeAQRwg3hwnrn1pGMrmqM6hK2qf/I8NisD5j6yNLXsQ48n1jtrTztYriLvC/NDinE4H4iQZRZECblb+ONkjyfnA4KO9TFZGdDF8QKBgQCWCghXD1pBEw05IO9kg5vdUCz40HHElHg4M9ESeTx+GdHMZG8fYRVgzTfLPkrrhNYQn8wQgV4vsyH5l99k6Qv0eD6ixV9vzvoJKd2d2v96sepHSit4NLCawzd00qD4KAHZ+ANqypYODm5kjtWjshfzTxlvzFcgGDWMX7BXGe5+TwKBgFCM83YaC9Z/Pif+2QGBFqiDPcIUS/rG/Fn6WF1Fvv9091MVSi83FO0pqSmHyNhbZf+IixvnQyjIiFs29r/0LniNDi4tk1kEjHoWfxlAT/Vv/u/zIxuJUcExTPcR4NxNhaHPwluev5ZglUHes18odjqmjiU68Sx25rzwNJy8xWkb";
    const RSA_K1_N: &str = "kre1N8y5cpV2303DY7heCvsRoTcSxMD1HXNljS7MD3LFV8l0UN24t_poMYLBo66iO77c_jgnN7LBSmi4F-nObG_lMQa5z4LdthPOpWnmSP18uHfNO9FhI0iis_5w4sMD5L-db1lu9NHJlxcD_MC25h4dQleRRaW6nKW5n3kYl_kTFXVTGj9g9UQBXqX2O3iPfApHT7hetoy0TUo_zeml3cw39qj4UnPjkZ1u3oL-R3hrj60m1v9Hg4anhmMvibSs67C6y0jJBrpjm70i3ckGMbLWFhrs-_-xrLQ738KMIvWcJ0ngG3gkxOM2_8nlxgap8qV5Rpkhjdurs6JOVFA9Qw";
    const RSA_K1_E: &str = "AQAB";
    const RSA_K2_PKCS8_B64: &str = "MIIEowIBAAKCAQEA1OBc45zsAF8uHgnpiY1oNyEpENMnzztC83CB+s7HVXLY+mRnqzbONbRBAs2ig7AXnzgGCekws0IclzNkwtM7Uu6PsC3ds2zemEe2mvdrvLRgfr/VBBV/acIEilhiQZiCjwhNBQ5mMUEy7xGLBGO7AyFH+YX1jHxnzndrsVh78qHCr1sKW/s1OohoFvvjdyVxLsUOe8D3FBtvEsOJdjajSKGQb/gqaR1mYEGJDbfFYXGRI+sZbj0x6PjXFiaMQzmFVgj9qbtrEG955P04ZY2XXFFhkcQ5L2/+mmGY2jIcl94f0RzRg6Dnz9CpeP799n68mWm2RC0gQBCvg0PjqETPfQIDAQABAoIBAAng6IaafVwuDoyPzrHWqU+nnwl8FwSXni2AWtPJ/TeSpw2Fk8b++dZFqV+pLcG5LftyRNt4Yg1rg7qjhKlC1qWsUugs1LUzF+hOqaNq2GnPIAw5uDeijMFaYs4NuG+NVX+wNazGpIjkGuo7sO0Q+Lh2NqoWZJTyn/mVkp6N+eCCOgJfJU9wPALhaxh9d+McuU+NQcwuJNP4fqbZsZg18im7eLp81gw2wmqcE6/nlbAcCYtv6/Qf2vNtYkwsmfRFw2zpX86OAHgYzLn31H3rVa7US6jRTwJTBDR4PnScB25IlGMEyTNCvcaCf3nWRy+DLhfazD0FZ9booGfvhkG8NrkCgYEA8R1OFbwQQvyrekw8i2njisw9gp45w5q+zx0igLVoyfE+dU4hNijqIM2L9eIOzo17+APsxR8R32zpiFHq4Y7D+HcObk4Z4ovtcPpVfy8hWDd5e394W5Cn8rRabotIffFYq+tqajoOQfd49OQhRF6kZ9O28nLTzlTCtAYGh8LGW9kCgYEA4gTE8uRZWOJuzsfmr3uU52SQpmg2fyjcyxzY04/fmYVgwJq7XMsFyOYF3/DG8a1IWePU4cNTuGXZ69V6AmJ+4IxVuaQqJhYsO3valxRN2b1/iTdF94kFElFw1VhSDr8Zi8C6p8MgtnLd10afz8qJvq8Fpjlg0sl9Y5aqqo7bvkUCgYEAzsNamrjTGR6jzhRiNwtu3S+X79LPpjWlOua5F7Z5TfNapC2DvXYIkGizhP99wxMBJAX5nHCwWvCCsyvMR/CtjPxLm7vN5sAntaJ2fsJjzcqEmqsNAzhhTXfrtLAsUhQM/iC6+K0gXOFjEF4xSPf/9ClR0Uc26tE0xjwEjZcxksECgYBjlOxdhCSll+Zwbv0BtjZuGQ3uc54HOq7FQul5lTtUPcLaOZJxmPXshQRznXdaBhoJmOszqUbayO1SDhWatWDneI1QpLb5Xu2QK8qspAKE/AwUHelS3ekBICbRqP9GhgYwX/MddUiVelipv/ErsTYtcJnRmhhB5wq69Sguy5Cf7QKBgGEtup9HaIETss8DObfRZtyCy9o8PAu8J68GccPzRP5jlHNEs6yJ6l9W4SIwIOaU7uaqL6vNRDNF6Q0AXPSAxoySQtYtt/IewiEVJvubllHO0P1DggJO3JcrE5HNwwkSN5PpCEaQ8vQeU1DoJq5wao9krzwT9+ZZ49e88FnEzP7N";
    let sign_rs256 = sign_rs256_jwt;

    let issuer = "https://rs256.issuer.test";
    let audience = "https://mcp.remote.test/mcp";
    let fixed_now: u64 = 1_800_000_000;
    let config = OAuthConfig::new(issuer, audience).unwrap();
    let fetcher = SharedOAuthMetadataFetcher::new(StaticJwksFetcher {
        issuer: issuer.to_owned(),
        jwks: json!({
            "keys": [{
                "kty": "RSA",
                "kid": "rsa-key-1",
                "alg": "RS256",
                "use": "sig",
                "n": RSA_K1_N,
                "e": RSA_K1_E,
            }]
        }),
    });
    let server =
        OAuthResourceServer::new(config, fetcher, EpochSecondsClock::new(move || fixed_now));

    let claims = json!({
        "iss": issuer,
        "aud": audience,
        "exp": fixed_now + 600,
        "sub": "rsa-user"
    });
    let valid_token = sign_rs256(RSA_K1_PKCS8_B64, "rsa-key-1", &claims);

    test_runtime().block_on(async {
        // 1. Valid RS256 token passes.
        let verified = server
            .verify_bearer(&valid_token)
            .await
            .expect("valid RS256 token must verify");
        assert_eq!(verified.subject.as_deref(), Some("rsa-user"));

        // 2. Tampered payload fails signature verification.
        let parts: Vec<&str> = valid_token.split('.').collect();
        let tampered_payload = BASE64URL_NOPAD.encode(
            serde_json::to_vec(&json!({
                "iss": issuer,
                "aud": audience,
                "exp": fixed_now + 600,
                "sub": "attacker"
            }))
            .unwrap()
            .as_slice(),
        );
        let tampered = format!("{}.{tampered_payload}.{}", parts[0], parts[2]);
        assert_eq!(
            server.verify_bearer(&tampered).await,
            Err(TokenRejection::InvalidSignature)
        );

        // 3. Corrupted signature fails.
        let mut sig_bytes = BASE64URL_NOPAD.decode(parts[2].as_bytes()).unwrap();
        sig_bytes[0] ^= 0xff;
        let wrong_sig = format!(
            "{}.{}.{}",
            parts[0],
            parts[1],
            BASE64URL_NOPAD.encode(&sig_bytes)
        );
        assert_eq!(
            server.verify_bearer(&wrong_sig).await,
            Err(TokenRejection::InvalidSignature)
        );

        // 4. Token signed by a different RSA key (same kid) fails.
        let wrong_key = sign_rs256(RSA_K2_PKCS8_B64, "rsa-key-1", &claims);
        assert_eq!(
            server.verify_bearer(&wrong_key).await,
            Err(TokenRejection::InvalidSignature)
        );

        // 5. Token with an unknown kid fails.
        let unknown_kid = sign_rs256(RSA_K1_PKCS8_B64, "unknown-kid", &claims);
        assert_eq!(
            server.verify_bearer(&unknown_kid).await,
            Err(TokenRejection::InvalidSignature)
        );
    });
}

/// Coverage for P14: `HttpTls::Upstream` / `trusts_proxy` / `with_trusted_forwarded`
/// admits `X-Forwarded-Proto: https` + `X-Forwarded-Host` from a trusted proxy
/// and refuses untrusted peers or plain `X-Forwarded-Proto: http`.
#[test]
fn streamable_http_upstream_tls_honors_trusted_proxy_headers_and_refuses_untrusted() {
    use goodfetch_protocol::daemon::TrustedProxy;
    use secrecy::SecretString;
    use tokio_util::sync::CancellationToken;

    let rt = test_runtime();
    rt.block_on(async {
        let tasks = TaskTracker::new();
        let token = transports::new_launch_token().expect("OS random token");
        let cancellation = CancellationToken::new();
        let connector = crate::engine::SharedEngineConnector::new(ScriptedSessionConnector {
            tasks: tasks.clone(),
            next_session: Arc::new(std::sync::atomic::AtomicUsize::new(1)),
            per_session_limit: 5,
            hold_method: None,
            cancelled_sessions: Arc::new(Mutex::new(Vec::new())),
            release_held: Arc::new(tokio::sync::Notify::new()),
        });
        let options = ServerOptions {
            product: Product::Collie,
            version: "0.1.0".to_owned(),
            profile: ResolvedProfile::for_product(Product::Collie, ProfileName::Browser),
            resources: crate::resources::ResourceStore::default(),
        };

        // 1. Server trusting loopback (127.0.0.1/32) as its upstream TLS-terminating proxy.
        let trusted_proxy = TrustedProxy::parse("127.0.0.1/32").unwrap();
        let bound_trusted = transports::BoundHttpServer::bind(
            transports::HttpServerConfig {
                bind: "127.0.0.1:0".parse().unwrap(),
                auth: transports::HttpAuth::LaunchToken(SecretString::from(token.clone())),
                tls: transports::HttpTls::Upstream(vec![trusted_proxy]),
                refusals: None,
                cancellation: cancellation.clone(),
            },
            options.clone(),
            connector.clone(),
        )
        .await
        .expect("bind upstream TLS MCP server (trusted)");
        let trusted_url = bound_trusted.url().to_owned();
        tasks.spawn(async move {
            let _ = bound_trusted.serve().await;
        });

        // 2. Server trusting only 10.0.0.1/32 (so our 127.0.0.1 peer is NOT trusted).
        let other_proxy = TrustedProxy::parse("10.0.0.1/32").unwrap();
        let bound_untrusted = transports::BoundHttpServer::bind(
            transports::HttpServerConfig {
                bind: "127.0.0.1:0".parse().unwrap(),
                auth: transports::HttpAuth::LaunchToken(SecretString::from(token.clone())),
                tls: transports::HttpTls::Upstream(vec![other_proxy]),
                refusals: None,
                cancellation: cancellation.clone(),
            },
            options,
            connector,
        )
        .await
        .expect("bind upstream TLS MCP server (untrusted)");
        let untrusted_url = bound_untrusted.url().to_owned();
        tasks.spawn(async move {
            let _ = bound_untrusted.serve().await;
        });

        let _ = rustls::crypto::ring::default_provider().install_default();
        let http = reqwest::Client::new();

        assert_eq!(
            post_forwarded_mcp_init(&http, &trusted_url, &token, "https").await,
            200,
            "trusted upstream proxy forwarded Host/Origin must be admitted"
        );
        assert_eq!(
            post_forwarded_mcp_init(&http, &trusted_url, &token, "http").await,
            403,
            "X-Forwarded-Proto: http must not admit forwarded Host"
        );
        assert_eq!(
            post_forwarded_mcp_init(&http, &untrusted_url, &token, "https").await,
            403,
            "untrusted peer must not have forwarded headers trusted"
        );

        cancellation.cancel();
        tasks.close();
        tasks.wait().await;
    });
}

async fn post_forwarded_mcp_init(
    http: &reqwest::Client,
    url: &str,
    token: &str,
    forwarded_proto: &str,
) -> reqwest::StatusCode {
    let init_body = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": "2025-11-25",
            "capabilities": {},
            "clientInfo": { "name": "upstream-proxy-test", "version": "1.0.0" }
        }
    });
    http.post(url)
        .header("Host", "mcp.edge.example")
        .header("Origin", "https://mcp.edge.example")
        .header("X-Forwarded-Proto", forwarded_proto)
        .header("X-Forwarded-Host", "mcp.edge.example")
        .header("Authorization", format!("Bearer {token}"))
        .header("Accept", "application/json, text/event-stream")
        .json(&init_body)
        .send()
        .await
        .unwrap()
        .status()
}
