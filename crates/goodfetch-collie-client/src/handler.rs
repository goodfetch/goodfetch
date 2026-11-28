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

use std::collections::BTreeMap;
use std::sync::{Arc, PoisonError};

use goodfetch_protocol::connection::{CALL_CANCELLED, CallCancelledDetails, CancelledBy};
use goodfetch_protocol::error_code::ErrorCode;
use goodfetch_protocol::jsonrpc::{
    ErrorData, ErrorObject, ErrorResponse, ProtocolFailure, Request, RequestId, Response, Version,
};
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

/// The streams the engine's requests go to, by method, and the in-flight
/// requests tracked for engine `$/cancel`.
#[derive(Debug)]
struct State {
    /// Open streams by method, or `None` once the Connection has ended.
    streams: Option<BTreeMap<String, mpsc::UnboundedSender<HandlerCall>>>,
    /// In-flight calls handed to a stream and not yet answered or cancelled.
    active: BTreeMap<RequestId, watch::Sender<bool>>,
}

/// The Connection's streams of the engine's requests, which the program
/// opens and the Connection's correlating task hands requests to.
#[derive(Clone, Debug)]
pub(crate) struct Streams {
    state: Arc<Mutex<State>>,
    trace_context_source: Option<Arc<dyn TraceContextSource>>,
}

impl Streams {
    /// No stream yet, on a Connection that has not ended.
    pub(crate) fn new(trace_context_source: Option<Arc<dyn TraceContextSource>>) -> Self {
        Self {
            state: Arc::new(Mutex::new(State {
                streams: Some(BTreeMap::new()),
                active: BTreeMap::new(),
            })),
            trace_context_source,
        }
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

    /// Hands `request` to its method's stream, or answers it with the
    /// method error when none serves it. Answers go out through `lines`.
    pub(crate) fn hand(&self, mut request: Request, lines: &WriteQueue) {
        let trace_context = trace_context::extract(request.params.as_mut());
        let (cancel_tx, cancel_rx) = watch::channel(false);
        let id = request.id.clone();
        let call = HandlerCall {
            id: request.id,
            method: request.method,
            params: request.params.unwrap_or(Value::Null),
            lines: lines.downgrade(),
            answered: false,
            opener: OpeningProcess::current(),
            cancel: cancel_rx,
            trace_context,
            trace_context_source: self.trace_context_source.clone(),
            streams: self.clone(),
        };
        let mut state = self.state();
        let State { streams, active } = &mut *state;
        let Some(streams) = streams.as_mut() else {
            return;
        };
        let refused = match streams.get(&call.method) {
            Some(stream) => {
                active.insert(id.clone(), cancel_tx);
                match stream.send(call) {
                    Ok(()) => return,
                    Err(mpsc::error::SendError(call)) => {
                        active.remove(&id);
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
        for cancel_tx in std::mem::take(&mut state.active).into_values() {
            let _ = cancel_tx.send(true);
        }
    }

    /// The streams state, which a panic elsewhere leaves as it was.
    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
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
        let details = CallCancelledDetails {
            by: CancelledBy::Program,
            closed: None,
            person: None,
        };
        let details: Map<String, Value> = serde_json::to_value(details)
            .and_then(serde_json::from_value)
            .expect("invariant: call_cancelled's details serialize as an object");
        let error = ErrorObject {
            code: CALL_CANCELLED.group.json_rpc_class(),
            message: "the program dropped the engine's request without answering it".to_owned(),
            data: ErrorData {
                code: CALL_CANCELLED.name.to_owned(),
                retryable: CALL_CANCELLED.retryable,
                details,
            },
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
