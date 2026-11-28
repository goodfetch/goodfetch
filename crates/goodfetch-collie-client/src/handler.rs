//! The engine's requests to the program, such as the Crawl requests a
//! crawl's handlers answer (ADR 0093) and Plugin-kind SDK handler calls
//! (ADR 0104). A Connection hands out each method's requests as one stream;
//! the program answers each request once, with a result or an error object.
//! When the engine cancels a request in flight with `$/cancel`, the call's
//! [`HandlerCall::cancelled`] resolves and the Client answers `call_cancelled`
//! (`by: client`), discarding any late answer. A request the program drops
//! unanswered goes back to the engine as `call_cancelled` (`by: program`),
//! which returns it to pending and counts no attempt (ADRs 0093 and 0102).
//! A request of a method no stream serves is answered with the protocol's
//! method error.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::pin::Pin;
use std::sync::{Arc, PoisonError};
use std::task::{Context, Poll};

use goodfetch_protocol::catalog::ErrorCodeDefinition;
use goodfetch_protocol::connection::{CALL_CANCELLED, CallCancelledDetails, CancelledBy};
use goodfetch_protocol::error_code::ErrorCode;
use goodfetch_protocol::jsonrpc::{
    ErrorData, ErrorObject, ErrorResponse, ProtocolFailure, Request, RequestId, Response, Version,
};
use goodfetch_protocol::plugin::extractor::{
    EXTRACT_CONTENT, EXTRACTOR, EXTRACTOR_UNAVAILABLE, ExtractorUnavailableDetails,
};
use goodfetch_protocol::plugin::notification::{
    NOTIFICATION_CHANNEL, NOTIFICATION_CHANNEL_UNAVAILABLE, NOTIFICATION_SEND,
    NotificationChannelUnavailableDetails,
};
use goodfetch_protocol::plugin::secret::{SECRET_PROVIDER, SECRET_RESOLVE};
use goodfetch_protocol::secrets::{SECRET_UNAVAILABLE, SecretFailure, SecretUnavailableDetails};
use goodfetch_protocol::settings::SettingInvalidDetails;
use serde::Serialize;
use serde_json::{Map, Value};
use tokio::sync::{mpsc, watch};

use crate::error::{BoxError, Error};
use crate::line;
use crate::logging;
use crate::opening_process::OpeningProcess;
use crate::sync::{Mutex, MutexGuard};
use crate::trace_context::{self, TraceContext, TraceContextSource};
use crate::write_queue::{WeakWriteQueue, WriteQueue};

/// A slot for a named Plugin-kind SDK handler stream.
#[derive(Debug)]
enum NamedStreamSlot {
    /// A stream is currently reading calls for this handler name.
    Open(mpsc::UnboundedSender<HandlerCall>),
    /// A stream was opened and then dropped; new calls fail immediately
    /// with the kind's unreachable code rather than hanging open.
    Closed,
}

/// What a [`HandlerCall`] answers the engine with if the program drops it
/// unanswered.
#[derive(Clone, Debug)]
enum DropOutcome {
    /// Generic requests (e.g. crawl requests): `call_cancelled` (`by: program`).
    CancelledByProgram,
    /// A Plugin-kind SDK handler call dropped unanswered fails with its
    /// kind's unreachable code (`<kind>_unavailable`, ADR 0104).
    KindUnavailable {
        kind: &'static str,
        handler: String,
        reference: Option<String>,
    },
}

/// The streams the engine's requests go to, by method or by `(kind, name)`,
/// and the in-flight requests tracked for engine `$/cancel`.
#[derive(Debug)]
struct State {
    /// Open streams by method, or `None` once the Connection has ended.
    streams: Option<BTreeMap<String, mpsc::UnboundedSender<HandlerCall>>>,
    /// Open or closed named streams for declared SDK handlers, keyed by
    /// `(kind, handler_name)`.
    named_streams: BTreeMap<(String, String), NamedStreamSlot>,
    /// Calls that arrived for a declared SDK handler before its stream was
    /// first opened; drained into the stream on [`Streams::open_named`] or
    /// cancelled on `$/cancel`.
    pending_named: BTreeMap<(String, String), VecDeque<HandlerCall>>,
    /// In-flight calls handed to a stream and not yet answered or cancelled.
    active: BTreeMap<RequestId, watch::Sender<bool>>,
}

/// The Connection's streams of the engine's requests, which the program
/// opens and the Connection's correlating task hands requests to.
#[derive(Clone, Debug)]
pub(crate) struct Streams {
    state: Arc<Mutex<State>>,
    declared: Arc<BTreeMap<String, BTreeSet<String>>>,
    trace_context_source: Option<Arc<dyn TraceContextSource>>,
}

impl Streams {
    /// No stream yet, on a Connection that has not ended, with `declared`
    /// SDK handlers from `hello`.
    pub(crate) fn new(
        trace_context_source: Option<Arc<dyn TraceContextSource>>,
        declared: BTreeMap<String, BTreeSet<String>>,
    ) -> Self {
        Self {
            state: Arc::new(Mutex::new(State {
                streams: Some(BTreeMap::new()),
                named_streams: BTreeMap::new(),
                pending_named: BTreeMap::new(),
                active: BTreeMap::new(),
            })),
            declared: Arc::new(declared),
            trace_context_source,
        }
    }

    /// Opens the one stream of calls for the declared SDK handler `name`
    /// of Plugin kind `kind` (ADR 0104).
    ///
    /// # Errors
    ///
    /// `closed` once the Connection has ended, and `setting_invalid` when
    /// `name` was not declared in `ConnectOptions` for `kind` or another
    /// stream is already open for `(kind, name)`.
    pub(crate) fn open_named(
        &self,
        kind: &'static str,
        name: &str,
    ) -> Result<NamedHandlerStream, Error> {
        let mut state = self.state();
        if state.streams.is_none() {
            return Err(Error::closed());
        }
        let is_declared = self
            .declared
            .get(kind)
            .is_some_and(|names| names.contains(name));
        if !is_declared {
            let key = format!("handlers.{kind}");
            return Err(Error::setting_invalid(
                format!("SDK handler `{name}` of kind `{kind}` was not declared in ConnectOptions"),
                &SettingInvalidDetails {
                    key,
                    layer: None,
                    reason: format!("undeclared `{kind}` handler `{name}`"),
                    limit: None,
                    fix: Some(format!(
                        "Declare `{name}` on ConnectOptions before opening its `{kind}` request stream."
                    )),
                },
            ));
        }
        let slot_key = (kind.to_owned(), name.to_owned());
        if let Some(NamedStreamSlot::Open(tx)) = state.named_streams.get(&slot_key)
            && !tx.is_closed()
        {
            let key = format!("handlers.{kind}");
            return Err(Error::setting_invalid(
                format!("a request stream is already open for `{kind}` handler `{name}`"),
                &SettingInvalidDetails {
                    key,
                    layer: None,
                    reason: format!("duplicate stream for `{kind}` handler `{name}`"),
                    limit: None,
                    fix: Some(format!(
                        "Read `{name}` from its existing stream, or drop that stream before opening another."
                    )),
                },
            ));
        }
        let (tx, rx) = mpsc::unbounded_channel();
        if let Some(pending) = state.pending_named.remove(&slot_key) {
            for call in pending {
                let _ = tx.send(call);
            }
        }
        state
            .named_streams
            .insert(slot_key, NamedStreamSlot::Open(tx));
        Ok(NamedHandlerStream { rx: Mutex::new(rx) })
    }

    /// The stream of the engine's requests of `method`.
    pub(crate) fn open(&self, method: &str) -> Result<HandlerRequests, Error> {
        let mut state = self.state();
        let Some(streams) = state.streams.as_mut() else {
            return Err(Error::closed());
        };
        if streams
            .get(method)
            .is_some_and(|stream| !stream.is_closed())
        {
            return Err(Error::already_served(method));
        }
        // Unbounded because the engine bounds it: it sends a Client at most
        // `crawl.handler.concurrency` requests at once, so the SDK keeps no
        // queue of its own (ADR 0081, How many at once). A smaller bound
        // would stall the Connection's input, the replies to the program's
        // own calls included, or refuse requests the engine was allowed to
        // send.
        let (stream, requests) = mpsc::unbounded_channel();
        streams.insert(method.to_owned(), stream);
        Ok(HandlerRequests(requests))
    }

    /// Hands `request` to its named SDK handler stream or its method's stream,
    /// queues it if its declared handler stream has not been opened yet, fails
    /// it with `<kind>_unavailable` if its stream was dropped, or answers it
    /// with the method error when none serves it. Answers go out through `lines`.
    pub(crate) fn hand(&self, mut request: Request, lines: &WriteQueue) {
        let trace_context = trace_context::extract(request.params.as_mut());
        let (cancel_tx, cancel_rx) = watch::channel(false);
        let id = request.id.clone();
        let params = request.params.unwrap_or(Value::Null);
        let plugin_kind = plugin_kind_for_method(&request.method);
        let handler_name = params
            .get("handler")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let on_drop = match (plugin_kind, &handler_name) {
            (Some(kind), Some(handler)) => DropOutcome::KindUnavailable {
                kind,
                handler: handler.clone(),
                reference: params
                    .get("reference")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
            },
            _ => DropOutcome::CancelledByProgram,
        };
        let call = HandlerCall {
            id: request.id,
            method: request.method,
            params,
            lines: lines.downgrade(),
            answered: false,
            opener: OpeningProcess::current(),
            cancel: cancel_rx,
            trace_context,
            trace_context_source: self.trace_context_source.clone(),
            streams: self.clone(),
            on_drop,
        };
        // 1. Named SDK handler stream, if this request targets a Plugin kind
        //    handler by name.
        if let (Some(kind), Some(handler)) = (plugin_kind, handler_name.as_deref()) {
            if let Some(remaining) = self.hand_named(kind, handler, id.clone(), cancel_tx, call) {
                let (cancel_tx, call) = remaining;
                self.hand_method(&id, cancel_tx, call);
            }
            return;
        }
        self.hand_method(&id, cancel_tx, call);
    }

    /// Routes a Plugin-kind SDK handler request by `(kind, handler)` name.
    /// Returns `None` once handled (delivered, queued, or dropped as
    /// unavailable), or `Some((cancel_tx, call))` when no named or declared
    /// handler owns it.
    fn hand_named(
        &self,
        kind: &'static str,
        handler: &str,
        id: RequestId,
        cancel_tx: watch::Sender<bool>,
        call: HandlerCall,
    ) -> Option<(watch::Sender<bool>, HandlerCall)> {
        let mut state = self.state();
        state.streams.as_ref()?;
        let slot_key = (kind.to_owned(), handler.to_owned());
        let State {
            streams,
            named_streams,
            pending_named,
            active,
        } = &mut *state;
        match named_streams.get(&slot_key) {
            Some(NamedStreamSlot::Open(tx)) => {
                active.insert(id, cancel_tx);
                if let Err(mpsc::error::SendError(call)) = tx.send(call) {
                    named_streams.insert(slot_key, NamedStreamSlot::Closed);
                    drop(state);
                    drop(call);
                }
                None
            }
            Some(NamedStreamSlot::Closed) => {
                active.insert(id, cancel_tx);
                drop(state);
                drop(call);
                None
            }
            None => {
                if let Some(streams) = streams.as_mut()
                    && let Some(stream) = streams.get(&call.method)
                {
                    active.insert(id, cancel_tx);
                    if let Err(mpsc::error::SendError(call)) = stream.send(call) {
                        streams.remove(&call.method);
                        drop(state);
                        drop(call);
                    }
                    return None;
                }
                if self
                    .declared
                    .get(kind)
                    .is_some_and(|names| names.contains(handler))
                {
                    active.insert(id, cancel_tx);
                    pending_named.entry(slot_key).or_default().push_back(call);
                    return None;
                }
                Some((cancel_tx, call))
            }
        }
    }

    /// Routes a request to a method-level stream, or refuses it with
    /// `method_not_found` when none serves it.
    fn hand_method(&self, id: &RequestId, cancel_tx: watch::Sender<bool>, call: HandlerCall) {
        let mut state = self.state();
        let State {
            streams, active, ..
        } = &mut *state;
        let Some(streams) = streams.as_mut() else {
            return;
        };
        let refused = match streams.get(&call.method) {
            Some(stream) => {
                active.insert(id.clone(), cancel_tx);
                match stream.send(call) {
                    Ok(()) => return,
                    Err(mpsc::error::SendError(call)) => {
                        active.remove(id);
                        streams.remove(&call.method);
                        call
                    }
                }
            }
            None => call,
        };
        drop(state);
        tracing::debug!(target: logging::CLIENT, method = %refused.method, "the engine sent a request no stream serves");
        refused.refuse_unserved();
    }

    /// Cancels an active handler call named by the engine's `$/cancel` (ADR
    /// 0038, ADR 0104): resolves [`HandlerCall::cancelled`], answers the
    /// engine `call_cancelled` (`by: client`), and suppresses any late reply.
    pub(crate) fn cancel(&self, id: &RequestId, lines: &WriteQueue) -> bool {
        let cancelled = self.state().active.remove(id);
        let Some(cancel_tx) = cancelled else {
            return false;
        };
        let _ = cancel_tx.send(true);
        let details = CallCancelledDetails {
            by: CancelledBy::Client,
            closed: None,
            person: None,
        };
        let details: Map<String, Value> = serde_json::to_value(details)
            .and_then(serde_json::from_value)
            .expect("invariant: call_cancelled's details serialize as an object");
        let error = ErrorObject {
            code: CALL_CANCELLED.group.json_rpc_class(),
            message: "the engine cancelled its request".to_owned(),
            data: ErrorData {
                code: CALL_CANCELLED.name.to_owned(),
                retryable: CALL_CANCELLED.retryable,
                details,
            },
        };
        if let Ok(line) = line::encode(&ErrorResponse {
            jsonrpc: Version,
            id: Some(id.clone()),
            error,
        }) {
            let _closed = lines.send(line);
        }
        true
    }

    /// Claims the right to answer `id`: returns `true` when `id` was still
    /// active, and `false` when it was already cancelled or answered.
    fn take_active(&self, id: &RequestId) -> bool {
        self.state().active.remove(id).is_some()
    }

    /// Ends every stream and resolves every in-flight cancellation handle:
    /// the Connection has ended.
    pub(crate) fn end(&self) {
        let mut state = self.state();
        state.streams.take();
        state.named_streams.clear();
        for cancel_tx in std::mem::take(&mut state.active).into_values() {
            let _ = cancel_tx.send(true);
        }
        // Clear pending calls after `active` is empty so their Drop does not
        // try to answer on a closed Connection.
        let pending = std::mem::take(&mut state.pending_named);
        drop(state);
        drop(pending);
    }

    /// The streams state, which a panic elsewhere leaves as it was.
    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Maps a Plugin-kind wire method name to its Plugin kind name.
fn plugin_kind_for_method(method: &str) -> Option<&'static str> {
    match method {
        EXTRACT_CONTENT => Some(EXTRACTOR.name),
        NOTIFICATION_SEND => Some(NOTIFICATION_CHANNEL.name),
        SECRET_RESOLVE => Some(SECRET_PROVIDER.name),
        _ => None,
    }
}

/// Builds the `<kind>_unavailable` error object for a Plugin-kind SDK handler
/// call dropped unanswered (ADR 0104).
fn kind_unavailable_error(
    kind: &'static str,
    handler: &str,
    reference: Option<&str>,
) -> ErrorObject {
    let (def, message, details): (&ErrorCodeDefinition, String, Map<String, Value>) = match kind {
        "extract" => {
            let details = serde_json::to_value(ExtractorUnavailableDetails {
                extractor: handler.to_owned(),
            })
            .and_then(serde_json::from_value)
            .expect("invariant: extractor_unavailable details serialize as an object");
            (
                &EXTRACTOR_UNAVAILABLE,
                format!("SDK extractor handler `{handler}` left its call unanswered"),
                details,
            )
        }
        "notification" => {
            let details = serde_json::to_value(NotificationChannelUnavailableDetails {
                channel: handler.to_owned(),
            })
            .and_then(serde_json::from_value)
            .expect("invariant: notification_channel_unavailable details serialize as an object");
            (
                &NOTIFICATION_CHANNEL_UNAVAILABLE,
                format!("SDK notification channel handler `{handler}` left its call unanswered"),
                details,
            )
        }
        "secret" => {
            let details = serde_json::to_value(SecretUnavailableDetails {
                placeholder: String::new(),
                reference: reference.unwrap_or_default().to_owned(),
                provider: handler.to_owned(),
                failure: SecretFailure::ProviderFailed,
                fix: format!(
                    "Answer `secret.resolve` calls from `{handler}`'s secret_requests stream."
                ),
            })
            .and_then(serde_json::from_value)
            .expect("invariant: secret_unavailable details serialize as an object");
            (
                &SECRET_UNAVAILABLE,
                format!("SDK secret provider handler `{handler}` left its call unanswered"),
                details,
            )
        }
        other => unreachable!("invariant: unknown Plugin kind `{other}` in DropOutcome"),
    };
    ErrorObject {
        code: def.group.json_rpc_class(),
        message,
        data: ErrorData {
            code: def.name.to_owned(),
            retryable: def.retryable,
            details,
        },
    }
}

/// The `futures_core::Stream` backing an [`crate::EventStream`] of a named
/// SDK handler's calls. Polls its channel directly without spawning any task
/// (ADR 0081).
pub(crate) struct NamedHandlerStream {
    rx: Mutex<mpsc::UnboundedReceiver<HandlerCall>>,
}

impl futures_core::Stream for NamedHandlerStream {
    type Item = Result<HandlerCall, Error>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let mut rx = self.rx.lock().unwrap_or_else(PoisonError::into_inner);
        rx.poll_recv(cx).map(|item| item.map(Ok))
    }
}

/// The engine's requests of one method, in the order it sent them, from
/// [`crate::ClientConnection::handler_requests`]. Dropping it stops serving the
/// method: the requests it still held go back to the engine as dropped,
/// and later ones are answered with the method error.
#[derive(Debug)]
pub struct HandlerRequests(mpsc::UnboundedReceiver<HandlerCall>);

impl HandlerRequests {
    /// The engine's next request, or `None` once the Connection has ended
    /// and the requests it handed out before are taken; those can no longer
    /// be answered.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future before it completes loses no request: the next
    /// call returns it.
    pub async fn next(&mut self) -> Option<HandlerCall> {
        self.0.recv().await
    }
}

/// One request the engine sent the program, answered once with
/// [`HandlerCall::answer`] or [`HandlerCall::fail`]. Dropping it unanswered
/// answers the engine with `call_cancelled` by the program, which returns
/// the request to pending (ADRs 0093 and 0102).
#[derive(Debug)]
pub struct HandlerCall {
    id: RequestId,
    method: String,
    params: Value,
    /// The Connection's write queue, which a held request never keeps
    /// going.
    lines: WeakWriteQueue,
    answered: bool,
    /// The process whose Connection read the request, the only one whose
    /// answer reaches the engine.
    opener: OpeningProcess,
    /// Notified when the engine cancels this call via `$/cancel` or the
    /// Connection ends.
    cancel: watch::Receiver<bool>,
    /// The incoming W3C Trace context from `params._meta`, if any (ADR 0132).
    trace_context: Option<TraceContext>,
    /// Creates a `tracing::Span` parented to `trace_context`, when active.
    trace_context_source: Option<Arc<dyn TraceContextSource>>,
    /// The owning streams table, used to prevent replying after `$/cancel`.
    streams: Streams,
    /// What to answer the engine with if this call is dropped unanswered.
    on_drop: DropOutcome,
}

impl HandlerCall {
    /// The request's method.
    pub fn method(&self) -> &str {
        &self.method
    }

    /// The request's params, `null` when it had none.
    pub fn params(&self) -> &Value {
        &self.params
    }

    /// Whether the engine cancelled this call with `$/cancel` or the
    /// Connection ended (ADR 0104).
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        *self.cancel.borrow()
    }

    /// Resolves once the engine cancels this call with `$/cancel` or the
    /// Connection ends (ADR 0104).
    ///
    /// # Cancel safety
    ///
    /// Dropping the future stops waiting without altering the call's
    /// cancellation state.
    pub async fn cancelled(&self) {
        let mut rx = self.cancel.clone();
        let _ = rx.wait_for(|&cancelled| cancelled).await;
    }

    /// The incoming W3C Trace context the engine's request carried in
    /// `_meta`, if any (ADR 0132).
    #[must_use]
    pub fn trace_context(&self) -> Option<&TraceContext> {
        self.trace_context.as_ref()
    }

    /// A `tracing::Span` parented to the engine's incoming Trace context
    /// when OpenTelemetry is enabled on the SDK, or [`tracing::Span::none`]
    /// otherwise (ADR 0132). Entering or instrumenting with this span nests
    /// the handler's child spans under the engine's request span.
    #[must_use]
    pub fn span(&self) -> tracing::Span {
        match (&self.trace_context, &self.trace_context_source) {
            (Some(context), Some(source)) => source.handler_span(context),
            _ => tracing::Span::none(),
        }
    }

    /// Answers the request with `result`. If the engine already cancelled
    /// the call with `$/cancel`, the late answer is discarded.
    ///
    /// # Errors
    ///
    /// `closed` when the Connection has ended, so the engine never
    /// reads the answer, and `connection_forked` in a process other than
    /// the one that opened it, such as the child of a `fork`.
    pub fn answer(mut self, result: Value) -> Result<(), Error> {
        if !self.streams.take_active(&self.id) {
            self.answered = true;
            return Ok(());
        }
        let id = self.id.clone();
        self.send(&Response {
            jsonrpc: Version,
            id,
            result,
        })
    }

    /// Answers the request with the program's own error `error`, as ADR
    /// 0109 reads it: a Goodfetch [`Error`] keeps its code, and any other
    /// error fails with `own`, the code of the place that called the
    /// program, such as a handler's `handler_failed` (see
    /// [`Error::of_program`]). If the engine already cancelled the call with
    /// `$/cancel`, the late answer is discarded.
    ///
    /// # Errors
    ///
    /// `closed` when the Connection has ended, so the engine never
    /// reads the answer, and `connection_forked` in a process other than
    /// the one that opened it, such as the child of a `fork`.
    pub fn fail(mut self, error: impl Into<BoxError>, own: ErrorCode) -> Result<(), Error> {
        if !self.streams.take_active(&self.id) {
            self.answered = true;
            return Ok(());
        }
        let id = Some(self.id.clone());
        self.send(&ErrorResponse {
            jsonrpc: Version,
            id,
            error: Error::of_program(error.into(), own).to_object(),
        })
    }

    /// Answers with the method error: no stream serves the method.
    fn refuse_unserved(mut self) {
        let failure = ProtocolFailure::MethodNotFound;
        let error = ErrorObject {
            code: failure.code(),
            message: format!("this Client serves no `{}`", self.method),
            data: ErrorData {
                code: failure.name().to_owned(),
                retryable: false,
                details: Map::new(),
            },
        };
        let id = Some(self.id.clone());
        let _closed = self.send(&ErrorResponse {
            jsonrpc: Version,
            id,
            error,
        });
    }

    /// Writes `answer` as the request's one answer.
    fn send(&mut self, answer: &impl Serialize) -> Result<(), Error> {
        self.answered = true;
        self.opener.ensure_current()?;
        let line = line::encode(answer).map_err(Error::shape)?;
        let lines = self.lines.upgrade().ok_or_else(Error::closed)?;
        lines.send(line).map_err(|_closed| Error::closed())
    }
}

impl Drop for HandlerCall {
    fn drop(&mut self) {
        if self.answered || !self.streams.take_active(&self.id) {
            return;
        }
        let error = match &self.on_drop {
            DropOutcome::CancelledByProgram => {
                let details = CallCancelledDetails {
                    by: CancelledBy::Program,
                    closed: None,
                    person: None,
                };
                let details: Map<String, Value> = serde_json::to_value(details)
                    .and_then(serde_json::from_value)
                    .expect("invariant: call_cancelled's details serialize as an object");
                ErrorObject {
                    code: CALL_CANCELLED.group.json_rpc_class(),
                    message: "the program dropped the engine's request without answering it"
                        .to_owned(),
                    data: ErrorData {
                        code: CALL_CANCELLED.name.to_owned(),
                        retryable: CALL_CANCELLED.retryable,
                        details,
                    },
                }
            }
            DropOutcome::KindUnavailable {
                kind,
                handler,
                reference,
            } => kind_unavailable_error(kind, handler, reference.as_deref()),
        };
        let id = Some(self.id.clone());
        if self
            .send(&ErrorResponse {
                jsonrpc: Version,
                id,
                error,
            })
            .is_err()
        {
            tracing::debug!(target: logging::CLIENT, method = %self.method, "a dropped request's Connection had ended");
        }
    }
}

/// C-SEND-SYNC: the program serves the engine's requests from any task or
/// thread (ADR 0081).
const _: () = {
    const fn send_sync_static<T: Send + Sync + 'static>() {}
    send_sync_static::<HandlerRequests>();
    send_sync_static::<HandlerCall>();
};
