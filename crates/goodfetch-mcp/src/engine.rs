//! Async JSON-RPC engine connection owned by an MCP session (ADRs 0024, 0038,
//! 0070, 0080, 0132): one actor task owns the stream, sends `hello` with
//! `Surface::Mcp` (Form `tool`), correlates concurrent calls, propagates W3C
//! `traceparent`/`tracestate` without `baggage`, translates cancellation to
//! `$/cancel`, and streams `run.progress`, Human pauses (with credential-free
//! links) and `warning`s to active MCP `progressToken`s.

use std::collections::BTreeMap;
use std::fmt::{self, Write as _};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::connection::{
    CANCEL, CONNECTION_LOST, CancelParams, Form, HELLO, HelloParams, HelloResult, ProductVersion,
    Surface,
};
use goodfetch_protocol::events::{
    CREDIT, CreditParams, EVENTS, EVENTS_SUBSCRIBE, EventBatch, EventsSubscribeParams,
};
use goodfetch_protocol::jsonrpc::{
    ErrorData, ErrorObject, META, Message, Notification, Request, RequestId, RequestMeta,
    TraceContext, Version,
};
use goodfetch_protocol::log::{LOG, LogEvent, LogLevel};
use goodfetch_protocol::needs_you::NEEDS_YOU_CHANGED;
use goodfetch_protocol::run::{HumanPause, RUN_PROGRESS, RunProgress};
use serde_json::{Map, Value, json};
use tokio::io::{AsyncBufReadExt as _, AsyncRead, AsyncWrite, AsyncWriteExt as _, BufReader};
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;
use url::Url;

/// Bounded command channel capacity for the engine link actor.
const COMMAND_BUFFER: usize = 64;

/// Initial credits granted to `events.subscribe`.
const EVENT_CREDITS: u32 = 16;

/// Progress update forwarded from the engine to an MCP `notifications/progress`.
#[derive(Clone, Debug, PartialEq)]
pub struct ProgressUpdate {
    /// Monotonically increasing progress value.
    pub progress: f64,
    /// Optional total if known.
    pub total: Option<f64>,
    /// Human-readable progress, warning or Human pause message.
    pub message: String,
}

/// Configuration for opening an [`EngineLink`].
#[derive(Clone)]
pub struct EngineLinkConfig {
    /// The Product this MCP server announces in `hello`.
    pub product: Product,
    /// The executable's Product version.
    pub product_version: ProductVersion,
    /// Optional Daemon bearer token (never logged or forwarded to tools).
    pub token: Option<String>,
    /// Optional base URL for Human pause links (credentials are always stripped).
    pub live_view_base_url: Option<String>,
}

impl fmt::Debug for EngineLinkConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EngineLinkConfig")
            .field("product", &self.product)
            .field("product_version", &self.product_version)
            .field("token", &self.token.as_ref().map(|_| "[redacted]"))
            .field("live_view_base_url", &self.live_view_base_url)
            .finish()
    }
}

/// Errors opening or shutting down an [`EngineLink`].
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum EngineLinkError {
    /// The engine refused `hello` with a typed protocol error.
    #[error("engine refused hello: {}", .0.message)]
    Refused(ErrorObject),
    /// The underlying transport closed or failed during handshake.
    #[error("engine connection failed: {0}")]
    Transport(String),
}

/// Async factory that opens one fresh [`EngineLink`] per MCP session so each
/// Streamable HTTP session gets its own engine Connection, per-Connection
/// Limits (ADR 0041) and independent cancellation (ADR 0070).
pub trait EngineConnector: Send + Sync + 'static {
    /// Opens a fresh engine Connection and completes its `hello` handshake.
    fn connect(
        &self,
    ) -> Pin<Box<dyn Future<Output = Result<EngineLink, EngineLinkError>> + Send + '_>>;
}

/// Newtype hiding dynamic dispatch for an [`EngineConnector`] (STYLE.md).
#[derive(Clone)]
pub struct SharedEngineConnector(Arc<dyn EngineConnector>);

impl SharedEngineConnector {
    /// Wraps `connector` in a cheaply cloneable [`SharedEngineConnector`].
    #[must_use]
    pub fn new(connector: impl EngineConnector) -> Self {
        Self(Arc::new(connector))
    }

    /// Opens a fresh engine Connection through the wrapped connector.
    ///
    /// # Errors
    ///
    /// Returns [`EngineLinkError`] when the transport or `hello` handshake fails.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future cancels the in-flight connect attempt.
    pub async fn connect(&self) -> Result<EngineLink, EngineLinkError> {
        self.0.connect().await
    }
}

/// An active handle to a Goodfetch engine JSON-RPC Connection.
///
/// Cheap to clone; all callers communicate with the single actor task over a
/// bounded channel (STYLE.md).
#[derive(Clone, Debug)]
pub struct EngineLink {
    commands: mpsc::Sender<Command>,
    tasks: TaskTracker,
}

enum Command {
    Call {
        method: String,
        params: Value,
        run_id: Option<String>,
        progress: Option<mpsc::Sender<ProgressUpdate>>,
        reply: oneshot::Sender<Result<Value, ErrorObject>>,
        id_out: oneshot::Sender<i64>,
    },
    Cancel {
        id: i64,
    },
    Close,
}

struct InFlight {
    reply: oneshot::Sender<Result<Value, ErrorObject>>,
    run_id: Option<String>,
    progress: Option<mpsc::Sender<ProgressUpdate>>,
    seq: u64,
}

impl EngineLink {
    /// Connects to an engine over `stream`, completes the `hello` handshake as
    /// `Surface::Mcp`, and starts the correlation actor on a tracked task.
    ///
    /// # Errors
    ///
    /// Returns [`EngineLinkError::Refused`] if the engine refuses `hello`, or
    /// [`EngineLinkError::Transport`] if the stream closes or fails.
    ///
    /// # Cancel safety
    ///
    /// Dropping the returned future before `hello` finishes stops the actor
    /// task when its command channel drops.
    pub async fn connect<S>(
        stream: S,
        config: EngineLinkConfig,
    ) -> Result<(Self, HelloResult), EngineLinkError>
    where
        S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    {
        let (read_half, write_half) = tokio::io::split(stream);
        Self::connect_split(read_half, write_half, config).await
    }

    /// Connects to an engine over separate `read` and `write` halves (such as
    /// `goodfetch_engine::transport::Connection::split`), completes the `hello`
    /// handshake as `Surface::Mcp`, and starts the correlation actor on a
    /// tracked task.
    ///
    /// # Errors
    ///
    /// Returns [`EngineLinkError::Refused`] if the engine refuses `hello`, or
    /// [`EngineLinkError::Transport`] if the stream closes or fails.
    ///
    /// # Cancel safety
    ///
    /// Dropping the returned future before `hello` finishes stops the actor
    /// task when its command channel drops.
    pub async fn connect_split<R, W>(
        read_half: R,
        write_half: W,
        config: EngineLinkConfig,
    ) -> Result<(Self, HelloResult), EngineLinkError>
    where
        R: AsyncRead + Unpin + Send + 'static,
        W: AsyncWrite + Unpin + Send + 'static,
    {
        let (tx, rx) = mpsc::channel(COMMAND_BUFFER);
        let tasks = TaskTracker::new();
        let (hello_tx, hello_rx) = oneshot::channel();

        let actor = Actor::new(config, rx);
        tasks.spawn(actor.run(read_half, write_half, hello_tx));

        let hello_result = hello_rx.await.map_err(|_| {
            EngineLinkError::Transport("engine link stopped before hello".to_owned())
        })??;

        Ok((
            Self {
                commands: tx,
                tasks,
            },
            hello_result,
        ))
    }

    /// Invokes a catalog operation on the engine.
    ///
    /// Injects `_meta` with `traceparent` and `tracestate` when present (never
    /// `baggage`). If `cancel` is triggered while waiting, sends `$/cancel`
    /// with the request's ID so the engine stops its work.
    ///
    /// # Errors
    ///
    /// Returns the engine's JSON-RPC [`ErrorObject`], or an internal error
    /// object if the engine link closed unexpectedly.
    ///
    /// # Cancel safety
    ///
    /// Dropping this future leaves the actor intact; cancelling via `cancel`
    /// sends `$/cancel` to the engine.
    pub async fn call(
        &self,
        method: &str,
        arguments: Map<String, Value>,
        trace: TraceContext,
        progress: Option<mpsc::Sender<ProgressUpdate>>,
        cancel: CancellationToken,
    ) -> Result<Value, ErrorObject> {
        let run_id = arguments
            .get("run_id")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let params = attach_trace_meta(arguments, trace);
        let (reply_tx, mut reply_rx) = oneshot::channel();
        let (id_tx, id_rx) = oneshot::channel();

        if self
            .commands
            .send(Command::Call {
                method: method.to_owned(),
                params,
                run_id,
                progress,
                reply: reply_tx,
                id_out: id_tx,
            })
            .await
            .is_err()
        {
            return Err(connection_closed_error("The engine connection is closed."));
        }

        let req_id = id_rx.await.ok();

        tokio::select! {
            biased;
            reply = &mut reply_rx => {
                reply.unwrap_or_else(|_| Err(connection_closed_error("The engine connection closed before answering.")))
            }
            () = cancel.cancelled() => {
                if let Some(id) = req_id {
                    let _ = self.commands.send(Command::Cancel { id }).await;
                }
                Err(ErrorObject {
                    code: -32001,
                    message: "The tool call was cancelled.".to_owned(),
                    data: ErrorData {
                        code: "call_cancelled".to_owned(),
                        retryable: false,
                        details: Map::new(),
                    },
                })
            }
        }
    }

    /// Closes the engine connection and waits for the actor task to finish.
    ///
    /// # Cancel safety
    ///
    /// Safe to cancel; closing is idempotent.
    pub async fn close(&self) {
        let _ = self.commands.send(Command::Close).await;
        self.tasks.close();
        self.tasks.wait().await;
    }

    /// Signals the engine actor to close without waiting, for use in `Drop`
    /// when a Streamable HTTP session ends.
    pub fn close_now(&self) {
        let _ = self.commands.try_send(Command::Close);
        self.tasks.close();
    }
}

/// Attaches `_meta` carrying `traceparent` and `tracestate` (never `baggage`)
/// when either is present (ADR 0132).
fn attach_trace_meta(mut arguments: Map<String, Value>, trace: TraceContext) -> Value {
    if trace.traceparent.is_some() || trace.tracestate.is_some() {
        let meta = RequestMeta {
            traceparent: trace.traceparent,
            tracestate: trace.tracestate,
            raw: false,
            form: None,
        };
        if let Ok(value) = serde_json::to_value(meta) {
            arguments.insert(META.to_owned(), value);
        }
    }
    Value::Object(arguments)
}

struct Actor {
    config: EngineLinkConfig,
    commands: mpsc::Receiver<Command>,
    next_id: i64,
    in_flight: BTreeMap<i64, InFlight>,
    subscription_id: Option<i64>,
    subscription_attempted: bool,
}

impl Actor {
    fn new(config: EngineLinkConfig, commands: mpsc::Receiver<Command>) -> Self {
        Self {
            config,
            commands,
            next_id: 1,
            in_flight: BTreeMap::new(),
            subscription_id: None,
            subscription_attempted: false,
        }
    }

    async fn run<R, W>(
        mut self,
        read_half: R,
        mut write_half: W,
        hello_tx: oneshot::Sender<Result<HelloResult, EngineLinkError>>,
    ) where
        R: AsyncRead + Unpin + Send + 'static,
        W: AsyncWrite + Unpin + Send + 'static,
    {
        let mut lines = BufReader::new(read_half).lines();

        let hello_params = HelloParams {
            form: Some(Form::Tool),
            token: self.config.token.clone(),
            needs: Some(self.config.product),
            log_level: Some(LogLevel::Warn),
            ..HelloParams::new(Surface::Mcp, self.config.product_version.clone())
        };
        let hello_id = self.alloc_id();
        let hello_value =
            serde_json::to_value(&hello_params).expect("invariant: HelloParams serializes");
        if let Err(err) = write_request(&mut write_half, hello_id, HELLO, hello_value).await {
            let _ = hello_tx.send(Err(EngineLinkError::Transport(err)));
            return;
        }

        let hello_outcome = await_hello(&mut lines, hello_id).await;
        let ok = hello_outcome.is_ok();
        let _ = hello_tx.send(hello_outcome);
        if !ok {
            let _ = write_half.shutdown().await;
            return;
        }

        loop {
            tokio::select! {
                cmd = self.commands.recv() => {
                    match cmd {
                        Some(Command::Call { method, params, run_id, progress, reply, id_out }) => {
                            if progress.is_some() && !self.subscription_attempted {
                                self.subscription_attempted = true;
                                let sub_id = self.alloc_id();
                                let sub_params = serde_json::to_value(EventsSubscribeParams::default())
                                    .expect("invariant: EventsSubscribeParams serializes");
                                if write_request(&mut write_half, sub_id, EVENTS_SUBSCRIBE.name, sub_params).await.is_ok() {
                                    let credit = serde_json::to_value(CreditParams {
                                        id: RequestId::Number(sub_id),
                                        credits: EVENT_CREDITS,
                                    }).expect("invariant: CreditParams serializes");
                                    let _ = write_notification(&mut write_half, CREDIT, credit).await;
                                    self.subscription_id = Some(sub_id);
                                }
                            }
                            let id = self.alloc_id();
                            let _ = id_out.send(id);
                            if let Err(err) = write_request(&mut write_half, id, &method, params).await {
                                let _ = reply.send(Err(connection_closed_error(&err)));
                                break;
                            }
                            self.in_flight.insert(id, InFlight { reply, run_id, progress, seq: 0 });
                        }
                        Some(Command::Cancel { id }) => {
                            let cancel_params = serde_json::to_value(CancelParams {
                                id: RequestId::Number(id),
                            }).expect("invariant: CancelParams serializes");
                            let _ = write_notification(&mut write_half, CANCEL, cancel_params).await;
                        }
                        Some(Command::Close) | None => break,
                    }
                }
                line = lines.next_line() => {
                    match line {
                        Ok(Some(text)) => {
                            if text.trim().is_empty() {
                                continue;
                            }
                            match serde_json::from_str::<Message>(&text) {
                                Ok(message) => {
                                    self.handle_message(message, &mut write_half).await;
                                }
                                Err(error) => {
                                    tracing::warn!(%error, "ignoring non-JSON-RPC line from engine");
                                }
                            }
                        }
                        Ok(None) => break,
                        Err(error) => {
                            tracing::debug!(%error, "engine stream read ended");
                            break;
                        }
                    }
                }
            }
        }

        for (_, pending) in std::mem::take(&mut self.in_flight) {
            let _ = pending.reply.send(Err(connection_closed_error(
                "The engine connection closed.",
            )));
        }
        let _ = write_half.shutdown().await;
    }

    fn alloc_id(&mut self) -> i64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    async fn handle_message<W: AsyncWrite + Unpin>(&mut self, message: Message, writer: &mut W) {
        match message {
            Message::Response(response) => {
                if let RequestId::Number(id) = response.id {
                    if self.subscription_id == Some(id) {
                        self.subscription_id = None;
                        return;
                    }
                    if let Some(pending) = self.in_flight.remove(&id) {
                        let _ = pending.reply.send(Ok(response.result));
                    }
                }
            }
            Message::Error(failure) => {
                if let Some(RequestId::Number(id)) = failure.id {
                    if self.subscription_id == Some(id) {
                        self.subscription_id = None;
                        tracing::debug!(code = %failure.error.data.code, "events.subscribe ended");
                        return;
                    }
                    if let Some(pending) = self.in_flight.remove(&id) {
                        let _ = pending.reply.send(Err(failure.error));
                    }
                }
            }
            Message::Notification(notification) => {
                self.handle_notification(notification, writer).await;
            }
            Message::Request(_) => {
                tracing::debug!("ignoring unexpected server-initiated request from engine");
            }
        }
    }

    async fn handle_notification<W: AsyncWrite + Unpin>(
        &mut self,
        notification: Notification,
        writer: &mut W,
    ) {
        let Some(params) = notification.params else {
            return;
        };
        if notification.method == EVENTS {
            if let Ok(batch) = serde_json::from_value::<EventBatch>(params) {
                if let RequestId::Number(id) = batch.id
                    && self.subscription_id == Some(id)
                {
                    let count = u32::try_from(batch.events.len()).unwrap_or(1).max(1);
                    let credit = serde_json::to_value(CreditParams {
                        id: RequestId::Number(id),
                        credits: count,
                    })
                    .expect("invariant: CreditParams serializes");
                    let _ = write_notification(writer, CREDIT, credit).await;
                }
                for envelope in batch.events {
                    self.dispatch_event(&envelope.event, &envelope.data);
                }
            }
        } else if notification.method == LOG {
            if let Ok(event) = serde_json::from_value::<LogEvent>(params)
                && event.level == LogLevel::Warn
            {
                let msg = format!("warning: {}", event.message);
                self.broadcast_progress(None, None, &msg);
            }
        } else if notification.method == RUN_PROGRESS.name {
            self.dispatch_event(RUN_PROGRESS.name, &params);
        } else if notification.method == "warning" || notification.method == "$/warning" {
            let text = params
                .get("message")
                .and_then(Value::as_str)
                .map_or_else(|| params.to_string(), str::to_owned);
            let run_id = params.get("run_id").and_then(Value::as_str);
            self.broadcast_progress(run_id, None, &format!("warning: {text}"));
        }
    }

    fn dispatch_event(&mut self, event_name: &str, data: &Value) {
        if event_name == RUN_PROGRESS.name {
            if let Ok(progress) = serde_json::from_value::<RunProgress>(data.clone()) {
                let run_id = progress.run_id.as_str().to_owned();
                for pause in &progress.pauses {
                    let message = format_human_pause(
                        &run_id,
                        pause,
                        self.config.product,
                        self.config.live_view_base_url.as_deref(),
                    );
                    self.broadcast_progress(Some(&run_id), Some(progress.position), &message);
                }
                let summary = format_run_progress(&progress);
                self.broadcast_progress(Some(&run_id), Some(progress.position), &summary);
            }
        } else if event_name == NEEDS_YOU_CHANGED.name {
            let run_id = data.get("run_id").and_then(Value::as_str);
            let kind = data
                .get("kind")
                .and_then(Value::as_str)
                .unwrap_or("confirmation");
            let link = build_clean_link(self.config.live_view_base_url.as_deref(), "/needs-you");
            let executable = self.config.product.executable_name();
            let message = format!(
                "Waiting for a person ({kind}): open {link} or run `{executable} needs-you answer`."
            );
            self.broadcast_progress(run_id, None, &message);
        } else if event_name == "warning" {
            let run_id = data.get("run_id").and_then(Value::as_str);
            let text = data
                .get("message")
                .and_then(Value::as_str)
                .map_or_else(|| data.to_string(), str::to_owned);
            self.broadcast_progress(run_id, None, &format!("warning: {text}"));
        }
    }

    fn broadcast_progress(&mut self, run_id: Option<&str>, position: Option<u64>, message: &str) {
        for pending in self.in_flight.values_mut() {
            let matches_run = match (run_id, pending.run_id.as_deref()) {
                (Some(ev_run), Some(call_run)) => ev_run == call_run,
                _ => true,
            };
            if !matches_run {
                continue;
            }
            if let Some(sink) = &pending.progress {
                pending.seq += 1;
                #[expect(
                    clippy::cast_precision_loss,
                    reason = "MCP progress is an f64 counter; sequence counts stay well within 53-bit precision"
                )]
                let progress_value =
                    position.map_or(pending.seq as f64, |p| p.max(pending.seq) as f64);
                let _ = sink.try_send(ProgressUpdate {
                    progress: progress_value,
                    total: None,
                    message: message.to_owned(),
                });
            }
        }
    }
}

async fn await_hello<R: AsyncRead + Unpin>(
    lines: &mut tokio::io::Lines<BufReader<R>>,
    hello_id: i64,
) -> Result<HelloResult, EngineLinkError> {
    let expected = RequestId::Number(hello_id);
    while let Some(line) = lines
        .next_line()
        .await
        .map_err(|err| EngineLinkError::Transport(err.to_string()))?
    {
        if line.trim().is_empty() {
            continue;
        }
        let message: Message = serde_json::from_str(&line)
            .map_err(|err| EngineLinkError::Transport(format!("invalid hello reply: {err}")))?;
        match message {
            Message::Response(response) if response.id == expected => {
                return serde_json::from_value(response.result)
                    .map_err(|err| EngineLinkError::Transport(format!("bad hello result: {err}")));
            }
            Message::Error(failure)
                if failure.id.is_none() || failure.id.as_ref() == Some(&expected) =>
            {
                return Err(EngineLinkError::Refused(failure.error));
            }
            _ => {}
        }
    }
    Err(EngineLinkError::Transport(
        "engine closed connection during hello".to_owned(),
    ))
}

async fn write_request<W: AsyncWrite + Unpin>(
    writer: &mut W,
    id: i64,
    method: &str,
    params: Value,
) -> Result<(), String> {
    let request = Request {
        jsonrpc: Version,
        id: RequestId::Number(id),
        method: method.to_owned(),
        params: Some(params),
    };
    write_json_line(writer, &request).await
}

async fn write_notification<W: AsyncWrite + Unpin>(
    writer: &mut W,
    method: &str,
    params: Value,
) -> Result<(), String> {
    let notification = Notification {
        jsonrpc: Version,
        method: method.to_owned(),
        params: Some(params),
    };
    write_json_line(writer, &notification).await
}

async fn write_json_line<W: AsyncWrite + Unpin, T: serde::Serialize>(
    writer: &mut W,
    value: &T,
) -> Result<(), String> {
    let mut bytes = serde_json::to_vec(value).map_err(|err| err.to_string())?;
    bytes.push(b'\n');
    writer
        .write_all(&bytes)
        .await
        .map_err(|err| err.to_string())?;
    writer.flush().await.map_err(|err| err.to_string())
}

fn connection_closed_error(message: &str) -> ErrorObject {
    ErrorObject {
        code: CONNECTION_LOST.group.json_rpc_class(),
        message: message.to_owned(),
        data: ErrorData {
            code: CONNECTION_LOST.name.to_owned(),
            retryable: CONNECTION_LOST.retryable,
            details: json!({
                "fix": "Reconnect to the engine or restart the Daemon, then retry the tool call."
            })
            .as_object()
            .cloned()
            .unwrap_or_default(),
        },
    }
}

/// Formats a [`RunProgress`] snapshot as a concise progress message.
fn format_run_progress(progress: &RunProgress) -> String {
    let mut text = format!(
        "run {} {:?} (position {})",
        progress.run_id.as_str(),
        progress.lifecycle,
        progress.position
    );
    if let Some(msg) = &progress.message {
        let _ = write!(text, ": {}", msg.as_str());
    }
    if let Some(storage) = &progress.storage {
        let _ = write!(
            text,
            " [storage {:?}: {} ({} bytes free)]",
            storage.level, storage.folder, storage.free_bytes
        );
    }
    text
}

/// Formats a [`HumanPause`] notification message with a credential-free link
/// and `needs-you answer` hint (#1359, #1708, #1940).
#[must_use]
pub fn format_human_pause(
    run_id: &str,
    pause: &HumanPause,
    product: Product,
    base_url: Option<&str>,
) -> String {
    let path = format!(
        "/runs/{run_id}/sessions/{}/pages/{}",
        pause.session, pause.page
    );
    let link = build_clean_link(base_url, &path);
    let reason = pause
        .reason
        .as_deref()
        .unwrap_or("Human pause waiting for a person");
    let executable = product.executable_name();
    format!(
        "Human pause waiting ({reason}): open {link} or run `{executable} needs-you answer` to continue."
    )
}

/// Constructs a link carrying no credentials (strips username, password, query
/// parameters and fragments from `base_url`).
#[must_use]
pub fn build_clean_link(base_url: Option<&str>, path: &str) -> String {
    let raw = base_url.unwrap_or("http://127.0.0.1:7601");
    let mut url = Url::parse(raw).unwrap_or_else(|_| {
        Url::parse("http://127.0.0.1:7601").expect("invariant: fallback URL is valid")
    });
    let _ = url.set_username("");
    let _ = url.set_password(None);
    url.set_query(None);
    url.set_fragment(None);
    url.set_path(path);
    url.to_string()
}
