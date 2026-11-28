//! Driving a Session through a Browser host beside its browser (ADRs 0022,
//! 0095, 0096, 0129 and 0132; issue 753).
//!
//! When `browser.host` is configured on a Run (in user configuration, project
//! configuration, Operator policy, connection settings or Run settings), the
//! engine routes each Session's operations over one JSON-RPC link per Session
//! to that Browser host using `goodfetch-engine`'s `framing` and `wire`
//! layers, carrying the engine's `CLIENT` span Trace context in `params._meta`
//! while OTLP export is on. The Browser host runs `goodfetch-page`'s driver,
//! guard and interception and `goodfetch-cdp`'s connection unchanged beside
//! its browser, so each action crosses the link in one network round trip.
//!
//! Size: one concern, the engine's Browser host transport link, remote Session
//! routing table and single-Session listener guard, kept together so framing,
//! arrival-ordered enqueue, event bridging, reverse handler dispatch and
//! teardown share one link type.

#![expect(
    clippy::disallowed_types,
    reason = "AtomicI64 and AtomicUsize generate local JSON-RPC request IDs and track the single-session cap without a lock (ADR 0060)"
)]

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};

use goodfetch_engine::client::Client;
use goodfetch_engine::events::Events;
use goodfetch_engine::framing::{Frame, Framer, Incoming, MAX_MESSAGE_BYTES, Received, decode};
use goodfetch_engine::local_addresses::Unavailable;
use goodfetch_engine::run_trace::{RUN_TRACE_NOTIFICATION, RunTrace};
use goodfetch_engine::runs::RunParts;
use goodfetch_engine::secrets::Purpose;
use goodfetch_engine::settings::RunSettings;
use goodfetch_engine::tasks::{TaskOwner, TaskStarter};
use goodfetch_engine::transport::{self, Connection};
use goodfetch_engine::{Answered, CallContext, CallError, Guard, trace, wire};
use goodfetch_protocol::catalog::{Operation, OperationKind, Product, SettingScope};
use goodfetch_protocol::connection::{
    CANCEL, CLOSED, CancelParams, HELLO, HelloParams, HelloResult, ProductVersion, Surface,
};
use goodfetch_protocol::daemon::DaemonEndpoint;
use goodfetch_protocol::egress::{EGRESS_LOCAL_ADDRESSES, LocalAddressFailure};
use goodfetch_protocol::events::{
    CREDIT, CreditParams, EVENTS, EVENTS_SUBSCRIBE, EventsSubscribeParams,
};
use goodfetch_protocol::jsonrpc::{
    ErrorObject, ErrorResponse, META, ProtocolFailure, RequestId, RequestMeta, TraceContext,
    Version,
};
use goodfetch_protocol::page::browser_host::{
    BROWSER_HOST, BrowserHostAddress, BrowserHostSetting,
};
use goodfetch_protocol::page::dialog::{DIALOG_HANDLE, PAGE_ON_DIALOG};
use goodfetch_protocol::page::route::{PAGE_ROUTE, ROUTE_HANDLE, SESSION_ROUTE};
use goodfetch_protocol::page::session::{
    RUN_NEW_SESSION, SESSION_CLOSE, SESSION_NEW_PAGE, SessionClosed,
};
use goodfetch_protocol::page::session_pages::SESSION_PAGES;
use goodfetch_protocol::page::{BROWSER_HOST_UNREACHABLE, PageId, RUN_NEW_PAGE, SessionId};
use goodfetch_protocol::run::{RUN_FINISH, RUN_START, RunId};
use goodfetch_protocol::secret_reference::SecretReference;
use goodfetch_protocol::settings::{
    CAPABILITY_UNAVAILABLE, CapabilityUnavailableDetails, SettingsLayer,
};
use secrecy::ExposeSecret as _;
use serde::Serialize;
use serde_json::{Map, Value, json};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::sync::{Mutex, mpsc, oneshot};
use tokio_util::sync::CancellationToken;
use tracing::Instrument as _;

/// Initial and refill credit batch for the Browser host's `events.subscribe`.
const EVENT_CREDITS: u32 = 64;

/// Byte chunk size read from a Browser host connection.
const READ_CHUNK: usize = 8 * 1024;

type ReplySender = oneshot::Sender<Result<Received, ErrorObject>>;
type ReplyReceiver = oneshot::Receiver<Result<Received, ErrorObject>>;
type PendingMap = Arc<Mutex<BTreeMap<RequestId, ReplySender>>>;
type StreamMap = Arc<Mutex<BTreeMap<RequestId, mpsc::Sender<Vec<Value>>>>>;
type HandlerMap = Arc<Mutex<BTreeMap<RequestId, (RequestId, Client)>>>;

/// One message queued synchronously in strict arrival order onto a Browser host
/// link's writer task.
enum Outbound {
    Bytes(Vec<u8>),
    Request {
        id: RequestId,
        bytes: Vec<u8>,
        reply: ReplySender,
        handler: Option<(RequestId, Client)>,
        stream: Option<mpsc::Sender<Vec<Value>>>,
    },
}

/// Handle returned by synchronously enqueueing one outbound JSON-RPC call.
struct EnqueuedCall {
    id: RequestId,
    span: tracing::Span,
    reply_rx: ReplyReceiver,
    has_handler: bool,
    stream_rx: Option<mpsc::Receiver<Vec<Value>>>,
}

/// One active JSON-RPC link from the engine to a Browser host process.
pub(crate) struct BrowserHostLink {
    run_id: RunId,
    outgoing: mpsc::UnboundedSender<Outbound>,
    next_id: AtomicI64,
    streams: StreamMap,
    /// Active `page.route` / `session.route` / `page.on_dialog` calls on this
    /// link: maps `remote_call_id` -> `(local_call_id, local_client)`.
    handler_calls: HandlerMap,
    /// Cancelled only when `shutdown()` tears down the link (after `run.finish`
    /// has been sent), independent of the parent Run's cancellation token.
    shutdown: CancellationToken,
    /// Owner of this link's reader/writer/event tasks.
    owner: TaskOwner,
}

impl BrowserHostLink {
    /// Connects to `connection`, exchanges `hello` announcing `Product::Collie`
    /// (ADR 0095) with `token`, starts a remote Run with `run_settings`, and
    /// subscribes to remote page events so downloads, popups and page events
    /// flow into `events`.
    #[expect(
        clippy::too_many_arguments,
        reason = "opening a Browser host link takes the socket, auth, Run identity/settings and engine handles"
    )]
    async fn open(
        connection: Connection,
        token: String,
        product_version: ProductVersion,
        run_id: RunId,
        run_settings: Map<String, Value>,
        parent_owner: &TaskStarter,
        events: Events,
        run_trace: RunTrace,
        remote: Arc<RemoteSessions>,
    ) -> Result<Arc<Self>, CallError> {
        let owner = parent_owner.child();
        let shutdown = CancellationToken::new();
        let (reader, writer) = connection.split();
        let (outgoing, outgoing_rx) = mpsc::unbounded_channel::<Outbound>();
        let pending: PendingMap = Arc::new(Mutex::new(BTreeMap::new()));
        let streams: StreamMap = Arc::new(Mutex::new(BTreeMap::new()));
        let handler_calls: HandlerMap = Arc::new(Mutex::new(BTreeMap::new()));

        spawn_writer(
            &owner,
            writer,
            outgoing_rx,
            shutdown.clone(),
            Arc::clone(&pending),
            Arc::clone(&streams),
            Arc::clone(&handler_calls),
        );

        let link = Arc::new(Self {
            run_id: run_id.clone(),
            outgoing: outgoing.clone(),
            next_id: AtomicI64::new(1),
            streams: Arc::clone(&streams),
            handler_calls: Arc::clone(&handler_calls),
            shutdown: shutdown.clone(),
            owner,
        });

        spawn_reader(
            reader,
            shutdown,
            ReaderState {
                pending,
                streams,
                handlers: handler_calls,
                outgoing,
                remote: Arc::clone(&remote),
                link: Arc::downgrade(&link),
                tasks: link.owner.clone(),
                run_trace,
            },
        );

        // Exchange `hello`, announcing `Product::Collie` whatever our engine's
        // Product is (ADR 0095).
        let hello_params = HelloParams {
            token: Some(token),
            needs: Some(Product::Collie),
            ..HelloParams::new(Surface::Rust, product_version)
        };
        let hello_value =
            serde_json::to_value(hello_params).expect("invariant: HelloParams encodes as JSON");
        let hello_reply = link
            .call_raw(HELLO, hello_value, CancellationToken::new(), false)
            .await?;
        let hello_result: HelloResult = serde_json::from_value(hello_reply).map_err(|error| {
            unreachable_error(format!("unreadable Browser host `hello` reply: {error}"))
        })?;
        if let Some((message, _)) =
            goodfetch_engine::rpc::version_refusal(hello_result.protocol_version, true)
        {
            link.shutdown().await;
            return Err(unreachable_error(message));
        }

        let start_params = json!({
            "run_id": run_id,
            "settings": run_settings,
        });
        link.call_raw(RUN_START.name, start_params, CancellationToken::new(), true)
            .await?;

        link.start_event_bridge(events, remote);

        Ok(link)
    }

    /// Subscribes to `events.subscribe` on the Browser host and re-publishes
    /// every page/Session event (`page.download`, `page.popup`, `session.page`,
    /// `page.close`, `page.request`, `page.response`, etc.) onto the engine's
    /// `Events`, registering any newly announced `page_id` first.
    fn start_event_bridge(self: &Arc<Self>, events: Events, remote: Arc<RemoteSessions>) {
        let id = RequestId::Number(self.next_id.fetch_add(1, Ordering::Relaxed));
        let (tx, mut rx) = mpsc::channel::<Vec<Value>>(64);
        let (reply_tx, _reply_rx) = oneshot::channel();
        let subscribe_params = EventsSubscribeParams::default();
        if let Ok(line) = wire::request_line(&id, EVENTS_SUBSCRIBE.name, &subscribe_params) {
            let _ = self.outgoing.send(Outbound::Request {
                id: id.clone(),
                bytes: line,
                reply: reply_tx,
                handler: None,
                stream: Some(tx),
            });
            let credit = wire::notification_line(
                CREDIT,
                &CreditParams {
                    id: id.clone(),
                    credits: EVENT_CREDITS,
                },
            );
            let _ = self.outgoing.send(Outbound::Bytes(credit));
        }
        let outgoing = self.outgoing.clone();
        let shutdown = self.shutdown.clone();
        let link = Arc::clone(self);
        let run_id = self.run_id.clone();
        self.owner.spawn(async move {
            loop {
                let batch = tokio::select! {
                    biased;
                    () = shutdown.cancelled() => break,
                    item = rx.recv() => match item {
                        Some(batch) => batch,
                        None => break,
                    },
                };
                let count = u32::try_from(batch.len()).unwrap_or(u32::MAX);
                for envelope in batch {
                    let Some(name) = envelope.get("event").and_then(Value::as_str) else {
                        continue;
                    };
                    let Some(data) = envelope.get("data") else {
                        continue;
                    };
                    // Register any newly announced page_id (popups, session pages)
                    // BEFORE publishing the event to subscribers.
                    register_announced_pages(data, &link, &remote).await;
                    if let Some(event) = goodfetch_protocol::page::ENTRIES
                        .events
                        .iter()
                        .find(|event| event.name == name)
                    {
                        let scope = data
                            .get("run_id")
                            .and_then(Value::as_str)
                            .or(Some(run_id.as_str()));
                        events.publish(event, scope, data).await;
                    }
                }
                if count > 0 {
                    let refill = wire::notification_line(
                        CREDIT,
                        &CreditParams {
                            id: id.clone(),
                            credits: count,
                        },
                    );
                    if outgoing.send(Outbound::Bytes(refill)).is_err() {
                        break;
                    }
                }
            }
        });
    }

    /// Synchronously assigns an ID, attaches `_meta` Trace context, and queues
    /// one outbound JSON-RPC call in strict arrival order before returning.
    fn enqueue_call(
        &self,
        method: &'static str,
        mut params: Value,
        with_trace: bool,
        handler: Option<(RequestId, Client)>,
        streaming: bool,
    ) -> Result<EnqueuedCall, CallError> {
        let span = if with_trace {
            trace::client_span(method)
        } else {
            tracing::Span::none()
        };
        let id = RequestId::Number(self.next_id.fetch_add(1, Ordering::Relaxed));
        if with_trace {
            trace::record_id(&span, &id);
            inject_meta(&span, &mut params);
        }
        let bytes = wire::request_line(&id, method, &params).map_err(|error| {
            CallError::protocol(ProtocolFailure::InternalError, error.to_string())
        })?;
        let (reply_tx, reply_rx) = oneshot::channel();
        let (stream_tx, stream_rx) = if streaming {
            let (tx, rx) = mpsc::channel(32);
            (Some(tx), Some(rx))
        } else {
            (None, None)
        };
        let has_handler = handler.is_some();
        self.outgoing
            .send(Outbound::Request {
                id: id.clone(),
                bytes,
                reply: reply_tx,
                handler,
                stream: stream_tx,
            })
            .map_err(|_| {
                unreachable_error("the Browser host connection closed before the request was sent")
            })?;
        Ok(EnqueuedCall {
            id,
            span,
            reply_rx,
            has_handler,
            stream_rx,
        })
    }

    /// Awaits an already-enqueued non-streaming request, cancelling remotely on
    /// `cancel`.
    async fn await_call(
        &self,
        enqueued: EnqueuedCall,
        cancel: CancellationToken,
        with_trace: bool,
    ) -> Result<Value, CallError> {
        let EnqueuedCall {
            id,
            span,
            mut reply_rx,
            has_handler,
            ..
        } = enqueued;
        let answer = async {
            tokio::select! {
                biased;
                reply = &mut reply_rx => reply,
                () = cancel.cancelled() => {
                    let cancel_line = wire::notification_line(CANCEL, &CancelParams { id: id.clone() });
                    let _ = self.outgoing.send(Outbound::Bytes(cancel_line));
                    reply_rx.await
                }
            }
        }
        .instrument(span.clone())
        .await;

        if has_handler {
            self.handler_calls.lock().await.remove(&id);
        }
        decode_reply(
            answer,
            &span,
            with_trace,
            || "the Browser host closed the connection before answering",
        )
    }

    /// Awaits an already-enqueued streaming call, bridging credits and
    /// `$/events` batches.
    async fn await_streaming(
        self: &Arc<Self>,
        enqueued: EnqueuedCall,
        context: &CallContext,
        remote: &Arc<RemoteSessions>,
    ) -> Result<Value, CallError> {
        let EnqueuedCall {
            id,
            span,
            mut reply_rx,
            stream_rx,
            ..
        } = enqueued;
        let mut stream_rx = stream_rx.expect("invariant: streaming call created stream channel");
        let stream = context.stream();
        let local_call_id = context.id();
        let cancel = context.cancellation();
        let mut cancel_sent = false;

        let answer = async {
            loop {
                tokio::select! {
                    biased;
                    reply = &mut reply_rx => break reply,
                    () = cancel.cancelled(), if !cancel_sent => {
                        cancel_sent = true;
                        let cancel_line = wire::notification_line(CANCEL, &CancelParams { id: id.clone() });
                        let _ = self.outgoing.send(Outbound::Bytes(cancel_line));
                    }
                    Some(batch) = stream_rx.recv() => {
                        forward_event_batch(batch, self, remote, local_call_id, stream).await;
                    }
                    granted = stream.spend(256) => {
                        let count = u32::try_from(granted).unwrap_or(u32::MAX);
                        let msg = wire::notification_line(
                            CREDIT,
                            &CreditParams {
                                id: id.clone(),
                                credits: count,
                            },
                        );
                        let _ = self.outgoing.send(Outbound::Bytes(msg));
                    }
                }
            }
        }
        .instrument(span.clone())
        .await;

        self.streams.lock().await.remove(&id);
        while let Ok(batch) = stream_rx.try_recv() {
            forward_event_batch(batch, self, remote, local_call_id, stream).await;
        }

        decode_reply(
            answer,
            &span,
            true,
            || "the Browser host closed the connection while streaming",
        )
    }

    /// Sends one JSON-RPC request to the Browser host under a `CLIENT` span
    /// (ADR 0132), injecting `params._meta` while the engine exports spans.
    pub(crate) async fn call(
        &self,
        method: &'static str,
        params: Value,
        cancel: CancellationToken,
    ) -> Result<Value, CallError> {
        self.call_raw(method, params, cancel, true).await
    }

    async fn call_raw(
        &self,
        method: &'static str,
        params: Value,
        cancel: CancellationToken,
        with_trace: bool,
    ) -> Result<Value, CallError> {
        let enqueued = self.enqueue_call(method, params, with_trace, None, false)?;
        self.await_call(enqueued, cancel, with_trace).await
    }

    /// Finishes the remote Run and closes the connection so a listening
    /// Browser host wipes its profile and exits.
    pub(crate) async fn finish_and_close(&self) {
        let _ = self
            .call_raw(
                RUN_FINISH.name,
                json!({ "run_id": self.run_id }),
                CancellationToken::new(),
                false,
            )
            .await;
        self.shutdown().await;
    }

    async fn shutdown(&self) {
        self.shutdown.cancel();
        self.owner.stop().await;
    }
}

async fn register_announced_pages(
    data: &Value,
    link: &Arc<BrowserHostLink>,
    remote: &Arc<RemoteSessions>,
) {
    if let Some(page_id) = data.get("page_id").and_then(Value::as_str) {
        remote
            .register_page(PageId(page_id.to_owned()), Arc::clone(link))
            .await;
    }
    if let Some(popup_id) = data
        .get("popup")
        .and_then(|popup| popup.get("page_id"))
        .and_then(Value::as_str)
    {
        remote
            .register_page(PageId(popup_id.to_owned()), Arc::clone(link))
            .await;
    }
}

fn inject_meta(span: &tracing::Span, params: &mut Value) {
    let sent = trace::sent(span);
    let raw = trace::in_raw_call();
    if (raw || sent.is_some())
        && let Value::Object(members) = params
    {
        let TraceContext {
            traceparent,
            tracestate,
        } = sent.unwrap_or_default();
        let meta = RequestMeta {
            traceparent,
            tracestate,
            raw,
        };
        members.insert(
            META.to_owned(),
            serde_json::to_value(meta).expect("invariant: RequestMeta encodes as JSON"),
        );
    }
}

fn decode_reply(
    answer: Result<Result<Received, ErrorObject>, oneshot::error::RecvError>,
    span: &tracing::Span,
    with_trace: bool,
    closed_message: impl FnOnce() -> &'static str,
) -> Result<Value, CallError> {
    match answer {
        Ok(Ok(received)) => received.read::<Value>().map_err(|error| {
            CallError::protocol(ProtocolFailure::InternalError, error.to_string())
        }),
        Ok(Err(error)) => {
            if with_trace {
                trace::record_failure(span, &error.data.code, error.code);
            }
            Err(CallError::from_wire(error))
        }
        Err(_) => Err(unreachable_error(closed_message())),
    }
}

async fn forward_event_batch(
    batch: Vec<Value>,
    link: &Arc<BrowserHostLink>,
    remote: &Arc<RemoteSessions>,
    local_call_id: &RequestId,
    stream: &goodfetch_engine::CallStream,
) {
    for event in &batch {
        if let Some(data) = event.get("data") {
            register_announced_pages(data, link, remote).await;
        }
    }
    let payload = json!({
        "id": local_call_id,
        "events": batch,
    });
    let _ = stream.notify(EVENTS, &payload).await;
}

struct ReaderState {
    pending: PendingMap,
    streams: StreamMap,
    handlers: HandlerMap,
    outgoing: mpsc::UnboundedSender<Outbound>,
    remote: Arc<RemoteSessions>,
    link: std::sync::Weak<BrowserHostLink>,
    tasks: TaskStarter,
    run_trace: RunTrace,
}

fn spawn_writer(
    owner: &TaskOwner,
    mut writer: goodfetch_engine::transport::ConnectionWriter,
    mut outgoing_rx: mpsc::UnboundedReceiver<Outbound>,
    shutdown: CancellationToken,
    pending: PendingMap,
    streams: StreamMap,
    handlers: HandlerMap,
) {
    owner.spawn(async move {
        loop {
            let item = tokio::select! {
                biased;
                () = shutdown.cancelled() => break,
                item = outgoing_rx.recv() => match item {
                    Some(item) => item,
                    None => break,
                },
            };
            let bytes = match item {
                Outbound::Bytes(bytes) => bytes,
                Outbound::Request {
                    id,
                    bytes,
                    reply,
                    handler,
                    stream,
                } => {
                    pending.lock().await.insert(id.clone(), reply);
                    if let Some(entry) = handler {
                        handlers.lock().await.insert(id.clone(), entry);
                    }
                    if let Some(tx) = stream {
                        streams.lock().await.insert(id, tx);
                    }
                    bytes
                }
            };
            if writer.write_all(&bytes).await.is_err() || writer.flush().await.is_err() {
                break;
            }
        }
        let _ = writer.shutdown().await;
    });
}

fn spawn_reader(
    mut reader: goodfetch_engine::transport::ConnectionReader,
    shutdown: CancellationToken,
    state: ReaderState,
) {
    let tasks = state.tasks.clone();
    tasks.spawn(async move {
        let mut framer = Framer::new(MAX_MESSAGE_BYTES);
        let mut buf = vec![0_u8; READ_CHUNK];
        let mut decoded = Vec::new();
        loop {
            let outcome = tokio::select! {
                biased;
                () = shutdown.cancelled() => break,
                read = reader.read(&mut buf) => read,
            };
            match outcome {
                Ok(0) | Err(_) => {
                    decoded.extend(framer.finish());
                    for frame in decoded.drain(..) {
                        if let Frame::Line(bytes) = frame {
                            dispatch_incoming(bytes, &state).await;
                        }
                    }
                    break;
                }
                Ok(read) => {
                    framer.feed(&buf[..read], &mut decoded);
                    for frame in decoded.drain(..) {
                        if let Frame::Line(bytes) = frame {
                            dispatch_incoming(bytes, &state).await;
                        }
                    }
                }
            }
        }
        state.pending.lock().await.clear();
        state.streams.lock().await.clear();
    });
}

async fn dispatch_incoming(bytes: Vec<u8>, state: &ReaderState) {
    let Ok(incoming) = decode(bytes) else {
        return;
    };
    match incoming {
        Incoming::Reply { id, answer } => {
            if let Some(tx) = state.pending.lock().await.remove(&id) {
                let _ = tx.send(answer);
            }
        }
        Incoming::Notification(notification) => {
            if notification.method() == EVENTS
                && let Some(raw) = notification.params()
                && let Ok(batch) = serde_json::from_str::<Value>(raw)
                && let Some(id_val) = batch.get("id")
                && let Ok(id) = serde_json::from_value::<RequestId>(id_val.clone())
                && let Some(events) = batch.get("events").and_then(Value::as_array)
            {
                let sender = state.streams.lock().await.get(&id).cloned();
                if let Some(sender) = sender {
                    let _ = sender.send(events.clone()).await;
                }
            } else if notification.method() == RUN_TRACE_NOTIFICATION
                && let Some(raw) = notification.params()
                && let Ok(entry) = serde_json::from_str::<Value>(raw)
                && let Some(file) = entry.get("file").and_then(Value::as_str)
                && let Some(line) = entry.get("line").and_then(Value::as_str)
            {
                let _ = state
                    .run_trace
                    .append_mirrored(file, line.as_bytes().to_vec())
                    .await;
            }
        }
        Incoming::Request { id, call } => {
            dispatch_handler_request(id, call, state).await;
        }
    }
}

async fn dispatch_handler_request(
    id: RequestId,
    call: goodfetch_engine::framing::Call,
    state: &ReaderState,
) {
    let method = call.method().to_owned();
    let handler_req = if method == ROUTE_HANDLE.name {
        Some(&ROUTE_HANDLE)
    } else if method == DIALOG_HANDLE.name {
        Some(&DIALOG_HANDLE)
    } else {
        None
    };
    let Some(handler_req) = handler_req else {
        return;
    };
    let mut params: Value = call
        .params()
        .and_then(|raw| serde_json::from_str(raw).ok())
        .unwrap_or_else(|| json!({}));
    if let Value::Object(obj) = &mut params {
        obj.remove(META);
    }
    let Some(remote_call_id) = params
        .get("id")
        .cloned()
        .and_then(|v| serde_json::from_value::<RequestId>(v).ok())
    else {
        return;
    };
    let Some((local_call_id, client)) = state.handlers.lock().await.get(&remote_call_id).cloned()
    else {
        return;
    };
    if let Some(link) = state.link.upgrade() {
        if let Some(route_id) = params.get("route_id").and_then(Value::as_str) {
            state
                .remote
                .register_route(route_id.to_owned(), Arc::clone(&link))
                .await;
        }
        if let Some(dialog_id) = params.get("dialog_id").and_then(Value::as_str) {
            state
                .remote
                .register_dialog(dialog_id.to_owned(), link)
                .await;
        }
    }
    if let Value::Object(obj) = &mut params {
        obj.insert(
            "id".to_owned(),
            serde_json::to_value(&local_call_id).expect("invariant: RequestId encodes as JSON"),
        );
    }
    let reply_outgoing = state.outgoing.clone();
    state.tasks.spawn(async move {
        let response_line = match client.send::<Value>(handler_req, &params).await {
            Ok(mut sent) => match sent.answer().await {
                Ok(result) => wire::success_line(&id, &result).ok(),
                Err(goodfetch_engine::client::ClientRequestError::Failed {
                    code,
                    retryable,
                    message,
                    details,
                }) => Some(wire::line(&ErrorResponse {
                    jsonrpc: Version,
                    id: Some(id),
                    error: ErrorObject {
                        code: goodfetch_protocol::catalog::ErrorGroup::Program.json_rpc_class(),
                        message,
                        data: goodfetch_protocol::jsonrpc::ErrorData {
                            code,
                            retryable,
                            details,
                        },
                    },
                })),
                Err(_) => Some(wire::line(&ErrorResponse {
                    jsonrpc: Version,
                    id: Some(id),
                    error: ErrorObject {
                        code: ProtocolFailure::InternalError.code(),
                        message: "the program connection ended before answering the handler"
                            .to_owned(),
                        data: goodfetch_protocol::jsonrpc::ErrorData {
                            code: ProtocolFailure::InternalError.name().to_owned(),
                            retryable: false,
                            details: Map::new(),
                        },
                    },
                })),
            },
            Err(_) => None,
        };
        if let Some(bytes) = response_line {
            let _ = reply_outgoing.send(Outbound::Bytes(bytes));
        }
    });
}

/// Routing table of Sessions currently driven through Browser hosts on this
/// engine.
#[derive(Default)]
pub(crate) struct RemoteSessions {
    state: Mutex<RemoteState>,
}

#[derive(Default)]
struct RemoteState {
    /// Default Session link per Run (for `run.new_page`).
    default_sessions: BTreeMap<RunId, Arc<BrowserHostLink>>,
    /// All open remote links per Run, so Run teardown closes every link.
    run_links: BTreeMap<RunId, Vec<Arc<BrowserHostLink>>>,
    /// Open remote Session ID -> link.
    sessions: BTreeMap<SessionId, Arc<BrowserHostLink>>,
    /// Closed remote Session IDs (for idempotent `session.close` and `closed` on reuse).
    closed_sessions: BTreeSet<SessionId>,
    /// Open remote Page ID -> link.
    pages: BTreeMap<PageId, Arc<BrowserHostLink>>,
    /// Active route ID -> link.
    routes: BTreeMap<String, Arc<BrowserHostLink>>,
    /// Active dialog ID -> link.
    dialogs: BTreeMap<String, Arc<BrowserHostLink>>,
    /// Active CDP session ID -> link.
    cdp_sessions: BTreeMap<String, Arc<BrowserHostLink>>,
    /// Active Lent endpoint ID -> link.
    lends: BTreeMap<String, Arc<BrowserHostLink>>,
}

impl RemoteSessions {
    pub(crate) async fn register_page(&self, page_id: PageId, link: Arc<BrowserHostLink>) {
        self.state.lock().await.pages.insert(page_id, link);
    }

    pub(crate) async fn register_route(&self, route_id: String, link: Arc<BrowserHostLink>) {
        self.state.lock().await.routes.insert(route_id, link);
    }

    pub(crate) async fn register_dialog(&self, dialog_id: String, link: Arc<BrowserHostLink>) {
        self.state.lock().await.dialogs.insert(dialog_id, link);
    }

    /// The `RunId` that owns a remote `page_id`, if any (for `RunOfCall`).
    pub(crate) async fn page_run(&self, page_id: &PageId) -> Option<RunId> {
        self.state
            .lock()
            .await
            .pages
            .get(page_id)
            .map(|link| link.run_id.clone())
    }

    /// Returns a `Guard` that routes any page/Session operation targeting a
    /// Browser host Session over its `BrowserHostLink` as one JSON-RPC message.
    ///
    /// Outbound calls are enqueued synchronously in strict JSON-RPC arrival
    /// order before returning the future so concurrent calls (`page.route`
    /// followed by `page.goto`, or repeated `route.*` answers) reach the
    /// Browser host in the exact order the program sent them.
    pub(crate) fn guard(self: &Arc<Self>) -> Guard {
        let remote = Arc::clone(self);
        Arc::new(move |operation, params, _read, context| {
            let state = remote.state.try_lock().ok()?;
            if state.is_empty() {
                return None;
            }
            let parsed: Value = serde_json::from_str(params.json()).ok()?;
            let key = RouteKey::extract(&parsed)?;
            if !state.matches(&key) {
                return None;
            }
            if let RouteKey::Session(ref sid) = key
                && state.closed_sessions.contains(sid)
            {
                let sid = sid.clone();
                let context = context.clone();
                return Some(Box::pin(async move {
                    if operation.name == SESSION_CLOSE.name {
                        return Answered::encode(&context, &SessionClosed {});
                    }
                    Err(CallError::new(
                        &CLOSED,
                        format!("Session `{}` is closed.", sid.0),
                        &json!({ "session_id": sid.0 }),
                    ))
                }));
            }
            let link = state.lookup(&key)?;
            drop(state);

            let is_streaming = operation.kind == OperationKind::Event;
            let handler = if operation.name == PAGE_ROUTE.name
                || operation.name == SESSION_ROUTE.name
                || operation.name == PAGE_ON_DIALOG.name
            {
                Some((context.id().clone(), context.client().clone()))
            } else {
                None
            };
            let enqueued = link.enqueue_call(operation.name, parsed, true, handler, is_streaming);
            let remote = Arc::clone(&remote);
            let context = context.clone();
            Some(Box::pin(async move {
                let enqueued = enqueued?;
                remote
                    .finish_forwarded(operation, key, link, enqueued, is_streaming, context)
                    .await
            }))
        })
    }

    async fn connect_for_run(
        self: &Arc<Self>,
        run: &RunParts,
        config: &BrowserHostSetting,
        context: &CallContext,
    ) -> Result<Arc<BrowserHostLink>, CallError> {
        let parsed_addr = BrowserHostAddress::parse(&config.address).map_err(unreachable_error)?;
        if parsed_addr.tls {
            return Err(CallError::new(
                &CAPABILITY_UNAVAILABLE,
                "TLS client connections to a remote Browser host (`wss://`) are not built yet; use loopback until area 4 brings TLS",
                &CapabilityUnavailableDetails {
                    key: BROWSER_HOST.name.to_owned(),
                    layer: SettingsLayer::Run,
                    kind: "tls".to_owned(),
                    fix: "Run the Browser host on loopback (e.g. `127.0.0.1:0`) or tunnel over loopback.".to_owned(),
                },
            ));
        }
        let socket_addr = parsed_addr
            .loopback_socket_addr()
            .ok_or_else(|| unreachable_error("Browser host address must resolve to loopback"))?;
        let token = resolve_token(run, config).await?;
        let connection = transport::connect_daemon(&DaemonEndpoint::Tcp(socket_addr))
            .await
            .map_err(|error| {
                unreachable_error(format!(
                    "could not connect to Browser host at `{}`: {error}",
                    config.address
                ))
            })?;
        let remote_settings = forwardable_settings(run.identity.settings());
        let run_id = run.identity.id().clone();
        let link = BrowserHostLink::open(
            connection,
            token,
            context.identity().product_version.clone(),
            run_id.clone(),
            remote_settings,
            run.identity.owner(),
            context.events().clone(),
            run.trace.clone(),
            Arc::clone(self),
        )
        .await?;

        let first_for_run = {
            let mut state = self.state.lock().await;
            let entry = state.run_links.entry(run_id.clone()).or_default();
            let first = entry.is_empty();
            entry.push(Arc::clone(&link));
            first
        };
        if first_for_run {
            let remote = Arc::clone(self);
            let run_owner = run.identity.owner().clone();
            let cleanup_run_id = run_id;
            run.identity.owner().spawn(async move {
                run_owner.cancelled().await;
                remote.close_run(&cleanup_run_id).await;
            });
        }
        Ok(link)
    }

    async fn close_run(&self, run_id: &RunId) {
        let links = {
            let mut state = self.state.lock().await;
            state.default_sessions.remove(run_id);
            let links = state.run_links.remove(run_id).unwrap_or_default();
            state.sessions.retain(|_, l| &l.run_id != run_id);
            state.pages.retain(|_, l| &l.run_id != run_id);
            state.routes.retain(|_, l| &l.run_id != run_id);
            state.dialogs.retain(|_, l| &l.run_id != run_id);
            state.cdp_sessions.retain(|_, l| &l.run_id != run_id);
            state.lends.retain(|_, l| &l.run_id != run_id);
            links
        };
        for link in links {
            link.finish_and_close().await;
        }
    }

    async fn finish_forwarded(
        self: Arc<Self>,
        operation: &'static Operation,
        key: RouteKey,
        link: Arc<BrowserHostLink>,
        enqueued: EnqueuedCall,
        is_streaming: bool,
        context: CallContext,
    ) -> Result<Answered, CallError> {
        let result = if is_streaming {
            link.await_streaming(enqueued, &context, &self).await?
        } else {
            link.await_call(enqueued, context.cancellation().clone(), true)
                .await?
        };

        {
            let mut state = self.state.lock().await;
            if operation.name == SESSION_NEW_PAGE.name
                && let Some(pid) = result.get("page_id").and_then(Value::as_str)
            {
                state
                    .pages
                    .insert(PageId(pid.to_owned()), Arc::clone(&link));
            } else if operation.name == SESSION_PAGES.name
                && let Some(pages) = result.get("pages").and_then(Value::as_array)
            {
                for page in pages {
                    if let Some(pid) = page.get("page_id").and_then(Value::as_str) {
                        state
                            .pages
                            .insert(PageId(pid.to_owned()), Arc::clone(&link));
                    }
                }
            } else if let Some(cid) = result.get("cdp_session_id").and_then(Value::as_str) {
                state.cdp_sessions.insert(cid.to_owned(), Arc::clone(&link));
            } else if let Some(eid) = result.get("endpoint_id").and_then(Value::as_str) {
                state.lends.insert(eid.to_owned(), Arc::clone(&link));
            }
        }

        if operation.name == SESSION_CLOSE.name
            && let RouteKey::Session(ref sid) = key
        {
            {
                let mut state = self.state.lock().await;
                state.sessions.remove(sid);
                state.closed_sessions.insert(sid.clone());
                state.pages.retain(|_, l| !Arc::ptr_eq(l, &link));
                state.routes.retain(|_, l| !Arc::ptr_eq(l, &link));
                state.dialogs.retain(|_, l| !Arc::ptr_eq(l, &link));
                state.cdp_sessions.retain(|_, l| !Arc::ptr_eq(l, &link));
                state.lends.retain(|_, l| !Arc::ptr_eq(l, &link));
                if let Some(default_link) = state.default_sessions.get(&link.run_id)
                    && Arc::ptr_eq(default_link, &link)
                {
                    state.default_sessions.remove(&link.run_id);
                }
            }
            link.finish_and_close().await;
        }

        Answered::encode(&context, &result)
    }
}

/// How an incoming page operation identifies its target Session/link.
enum RouteKey {
    Session(SessionId),
    Page(PageId),
    Route(String),
    Dialog(String),
    CdpSession(String),
    Lend(String),
}

impl RouteKey {
    fn extract(params: &Value) -> Option<Self> {
        if let Some(sid) = params.get("session_id").and_then(Value::as_str) {
            return Some(Self::Session(SessionId(sid.to_owned())));
        }
        if let Some(pid) = params.get("page_id").and_then(Value::as_str).or_else(|| {
            params
                .get("locator")
                .and_then(|l| l.get("page_id"))
                .and_then(Value::as_str)
        }) {
            return Some(Self::Page(PageId(pid.to_owned())));
        }
        if let Some(rid) = params.get("route_id").and_then(Value::as_str) {
            return Some(Self::Route(rid.to_owned()));
        }
        if let Some(did) = params.get("dialog_id").and_then(Value::as_str) {
            return Some(Self::Dialog(did.to_owned()));
        }
        if let Some(cid) = params.get("cdp_session_id").and_then(Value::as_str) {
            return Some(Self::CdpSession(cid.to_owned()));
        }
        if let Some(eid) = params.get("endpoint_id").and_then(Value::as_str) {
            return Some(Self::Lend(eid.to_owned()));
        }
        None
    }
}

impl RemoteState {
    fn is_empty(&self) -> bool {
        self.sessions.is_empty()
            && self.closed_sessions.is_empty()
            && self.pages.is_empty()
            && self.routes.is_empty()
            && self.dialogs.is_empty()
            && self.cdp_sessions.is_empty()
            && self.lends.is_empty()
    }

    fn matches(&self, key: &RouteKey) -> bool {
        match key {
            RouteKey::Session(sid) => {
                self.sessions.contains_key(sid) || self.closed_sessions.contains(sid)
            }
            RouteKey::Page(pid) => self.pages.contains_key(pid),
            RouteKey::Route(rid) => self.routes.contains_key(rid),
            RouteKey::Dialog(did) => self.dialogs.contains_key(did),
            RouteKey::CdpSession(cid) => self.cdp_sessions.contains_key(cid),
            RouteKey::Lend(eid) => self.lends.contains_key(eid),
        }
    }

    fn lookup(&self, key: &RouteKey) -> Option<Arc<BrowserHostLink>> {
        match key {
            RouteKey::Session(sid) => self.sessions.get(sid).cloned(),
            RouteKey::Page(pid) => self.pages.get(pid).cloned(),
            RouteKey::Route(rid) => self.routes.get(rid).cloned(),
            RouteKey::Dialog(did) => self.dialogs.get(did).cloned(),
            RouteKey::CdpSession(cid) => self.cdp_sessions.get(cid).cloned(),
            RouteKey::Lend(eid) => self.lends.get(eid).cloned(),
        }
    }
}

/// Reads `browser.host` from `run`'s resolved settings, if configured.
pub(crate) fn browser_host_of(run: &RunParts) -> Result<Option<BrowserHostSetting>, CallError> {
    let Some(settings) = run.identity.settings() else {
        return Ok(None);
    };
    let value = settings.resolved().value(&BROWSER_HOST);
    if value.is_null() {
        return Ok(None);
    }
    serde_json::from_value(value.clone())
        .map(Some)
        .map_err(|error| unreachable_error(format!("invalid `browser.host` setting: {error}")))
}

/// Refuses `egress.local_addresses` on a Browser host Session (ADR 0044; issue 638).
fn refuse_local_addresses(run: &RunParts) -> Result<(), CallError> {
    let Some(settings) = run.identity.settings() else {
        return Ok(());
    };
    let value = settings.resolved().value(&EGRESS_LOCAL_ADDRESSES);
    if value.is_null() {
        return Ok(());
    }
    let entry = match value {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    };
    Err(Unavailable {
        entry,
        failure: LocalAddressFailure::NotLaunchedHere,
    }
    .into())
}

/// Resolves the authentication token for `config` from `token_file` or a
/// `SecretReference` (`token`).
async fn resolve_token(run: &RunParts, config: &BrowserHostSetting) -> Result<String, CallError> {
    if let Some(path) = &config.token_file {
        let path_buf = std::path::PathBuf::from(path);
        let text = run
            .identity
            .owner()
            .blocking(move || {
                #[expect(
                    clippy::disallowed_methods,
                    reason = "reading the Browser host token file once per Session on the blocking policy (ADR 0059)"
                )]
                std::fs::read_to_string(&path_buf)
            })
            .await
            .map_err(|error| {
                unreachable_error(format!(
                    "could not read Browser host token file `{path}`: {error}"
                ))
            })?;
        let trimmed = text.trim().to_owned();
        if trimmed.is_empty() {
            return Err(unreachable_error(format!(
                "Browser host token file `{path}` is empty"
            )));
        }
        return Ok(trimmed);
    }
    if let Some(reference) = &config.token {
        let parsed = SecretReference::parse(reference)
            .map_err(|error| unreachable_error(format!("invalid token reference: {error}")))?;
        let resolved = run
            .keys
            .secrets()
            .resolve(
                &parsed,
                &Purpose {
                    placeholder: BROWSER_HOST.name,
                    domain: None,
                    domains: &[],
                    layer: Some(SettingsLayer::Run),
                    run_id: Some(run.identity.id()),
                },
            )
            .await
            .map_err(|error| goodfetch_engine::run::secret_error(&error))?;
        return Ok(resolved.expose_secret().to_owned());
    }
    Err(unreachable_error(
        "`browser.host` has neither `token_file` nor `token`",
    ))
}

/// Collects every non-default Run-scoped setting from `settings` except
/// `browser.host` itself (so the Browser host runs the browser locally instead
/// of recursing).
fn forwardable_settings(settings: Option<&RunSettings>) -> Map<String, Value> {
    let Some(settings) = settings else {
        return Map::new();
    };
    let resolved = settings.resolved();
    let mut nested = Map::new();
    for module in goodfetch_protocol::entries::MODULES {
        for setting in module.settings {
            if setting.name == BROWSER_HOST.name {
                continue;
            }
            if !goodfetch_protocol::setting_scopes::of(setting).contains(&SettingScope::Run) {
                continue;
            }
            let Some(effective) = resolved.effective(setting) else {
                continue;
            };
            if effective.layer == SettingsLayer::Default {
                continue;
            }
            insert_nested(&mut nested, setting.name, effective.value.clone());
        }
    }
    nested
}

fn insert_nested(root: &mut Map<String, Value>, key: &str, value: Value) {
    let mut parts = key.split('.').peekable();
    let mut current = root;
    while let Some(part) = parts.next() {
        if parts.peek().is_none() {
            current.insert(part.to_owned(), value);
            return;
        }
        let next = current
            .entry(part.to_owned())
            .or_insert_with(|| Value::Object(Map::new()));
        if !next.is_object() {
            *next = Value::Object(Map::new());
        }
        current = next
            .as_object_mut()
            .expect("invariant: ensured object above");
    }
}

fn unreachable_error(message: impl Into<String>) -> CallError {
    CallError::new(
        &BROWSER_HOST_UNREACHABLE,
        message,
        &json!({
            "fix": "Ensure the Browser host is running and reachable at `browser.host.address` with the matching token.",
        }),
    )
}

/// Guard for a listening Browser host (`browser-host --listen`) enforcing that
/// it serves at most one Session (ADR 0022; issue 753).
pub(crate) fn single_session_guard() -> Guard {
    let opened = Arc::new(AtomicUsize::new(0));
    let default_run: Arc<Mutex<Option<RunId>>> = Arc::new(Mutex::new(None));
    Arc::new(move |operation, params, _read, _context| {
        if operation.name != RUN_NEW_SESSION.name && operation.name != RUN_NEW_PAGE.name {
            return None;
        }
        let raw = params.json().to_owned();
        let opened = Arc::clone(&opened);
        let default_run = Arc::clone(&default_run);
        if operation.name == RUN_NEW_SESSION.name {
            if opened.fetch_add(1, Ordering::SeqCst) >= 1 {
                return Some(Box::pin(async {
                    Err(CallError::new(
                        &CLOSED,
                        "This Browser host serves one Session and has already served it.",
                        &json!({}),
                    ))
                }));
            }
            return None;
        }
        let run_id = serde_json::from_str::<Value>(&raw)
            .ok()
            .and_then(|v| v.get("run_id").and_then(Value::as_str).map(str::to_owned))
            .and_then(|s| RunId::parse(&s).ok())?;
        let mut guard = default_run.try_lock().ok()?;
        match &*guard {
            Some(existing) if existing == &run_id => None,
            Some(_) => Some(Box::pin(async {
                Err(CallError::new(
                    &CLOSED,
                    "This Browser host serves one Session and has already served it.",
                    &json!({}),
                ))
            })),
            None => {
                if opened.fetch_add(1, Ordering::SeqCst) >= 1 {
                    return Some(Box::pin(async {
                        Err(CallError::new(
                            &CLOSED,
                            "This Browser host serves one Session and has already served it.",
                            &json!({}),
                        ))
                    }));
                }
                *guard = Some(run_id);
                None
            }
        }
    })
}

/// Routes `run.new_page` or `run.new_session` to a Browser host whenever the
/// Run configures `browser.host`; returns `Ok(None)` when local.
pub(crate) async fn maybe_open_remote(
    remote: &Arc<RemoteSessions>,
    operation: &'static Operation,
    params: &impl Serialize,
    run_id: RunId,
    context: &CallContext,
) -> Result<Option<Value>, CallError> {
    let Ok(run) = context.runs().live(run_id.clone()).await else {
        return Ok(None);
    };
    let Some(config) = browser_host_of(&run)? else {
        return Ok(None);
    };
    refuse_local_addresses(&run)?;

    let link = if operation.name == RUN_NEW_PAGE.name {
        let existing = remote
            .state
            .lock()
            .await
            .default_sessions
            .get(&run_id)
            .cloned();
        if let Some(link) = existing {
            link
        } else {
            let link = remote.connect_for_run(&run, &config, context).await?;
            remote
                .state
                .lock()
                .await
                .default_sessions
                .insert(run_id.clone(), Arc::clone(&link));
            link
        }
    } else {
        remote.connect_for_run(&run, &config, context).await?
    };

    let params_value = serde_json::to_value(params)
        .map_err(|error| CallError::protocol(ProtocolFailure::InternalError, error.to_string()))?;
    let result = link
        .call(operation.name, params_value, context.cancellation().clone())
        .await?;
    {
        let mut state = remote.state.lock().await;
        if let Some(sid) = result.get("session_id").and_then(Value::as_str) {
            state
                .sessions
                .insert(SessionId(sid.to_owned()), Arc::clone(&link));
        }
        if let Some(pid) = result.get("page_id").and_then(Value::as_str) {
            state
                .pages
                .insert(PageId(pid.to_owned()), Arc::clone(&link));
        }
    }
    Ok(Some(result))
}
