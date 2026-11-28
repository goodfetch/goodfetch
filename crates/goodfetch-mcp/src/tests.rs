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
    let tasks = TaskTracker::new();
    let (mcp_engine_side, fake_engine_side) = tokio::io::duplex(64 * 1024);
    tasks.spawn(engine_fn(fake_engine_side));

    let config = EngineLinkConfig {
        product: Product::Collie,
        product_version: ProductVersion::parse("0.1.0").unwrap(),
        token: None,
        live_view_base_url,
    };
    let (link, _hello) = EngineLink::connect(mcp_engine_side, config)
        .await
        .expect("engine link hello succeeded");

    let server = McpServer::new(
        ServerOptions {
            product: Product::Collie,
            version: "0.1.0".to_owned(),
            profile: ResolvedProfile::for_product(Product::Collie, ProfileName::Browser),
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
        .tools
        .iter()
        .map(|projected| {
            json!({
                "name": projected.tool.name,
                "operation": projected.operation,
                "annotations": projected.tool.annotations,
                "input_schema_type": projected.tool.input_schema.get("type"),
            })
        })
        .collect();
    insta::assert_snapshot!(
        "browser_profile_tools_list",
        serde_json::to_string_pretty(&summary).unwrap()
    );
}
