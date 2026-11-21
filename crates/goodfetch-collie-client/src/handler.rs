//! The engine's requests to the program, such as the Crawl requests a
//! crawl's handlers answer (ADR 0093). A Connection hands out each method's
//! requests as one stream; the program answers each request once, with a
//! result or an error object. A request it drops unanswered goes back to
//! the engine as `call_cancelled` by the program, which returns it to
//! pending and counts no attempt (ADRs 0093 and 0102). A request of a
//! method no stream serves is answered with the protocol's method error.

use std::collections::BTreeMap;
use std::sync::{Arc, PoisonError};

use goodfetch_protocol::connection::{CALL_CANCELLED, CallCancelledDetails, CancelledBy};
use goodfetch_protocol::jsonrpc::{
    ErrorData, ErrorObject, ErrorResponse, ProtocolFailure, Request, RequestId, Response, Version,
};
use serde::Serialize;
use serde_json::{Map, Value};
use tokio::sync::mpsc;

use crate::error::{BoxError, Error};
use crate::line;
use crate::opening_process::OpeningProcess;
use crate::sync::{Mutex, MutexGuard};
use goodfetch_protocol::error_code::ErrorCode;

/// The streams the engine's requests go to, by method, or `None` once the
/// Connection has ended, which ends every stream.
type Served = Option<BTreeMap<String, mpsc::UnboundedSender<HandlerCall>>>;

/// The Connection's streams of the engine's requests, which the program
/// opens and the Connection's correlating task hands requests to.
#[derive(Clone, Debug)]
pub(crate) struct Streams(Arc<Mutex<Served>>);

impl Streams {
    /// No stream yet, on a Connection that has not ended.
    pub(crate) fn new() -> Self {
        Self(Arc::new(Mutex::new(Some(BTreeMap::new()))))
    }

    /// The stream of the engine's requests of `method`.
    pub(crate) fn open(&self, method: &str) -> Result<HandlerRequests, Error> {
        let mut served = self.served();
        let Some(streams) = served.as_mut() else {
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
    pub(crate) fn hand(&self, request: Request, lines: &mpsc::UnboundedSender<Vec<u8>>) {
        let call = HandlerCall {
            id: request.id,
            method: request.method,
            params: request.params.unwrap_or(Value::Null),
            lines: lines.downgrade(),
            answered: false,
            opener: OpeningProcess::current(),
        };
        let mut served = self.served();
        let Some(streams) = served.as_mut() else {
            return;
        };
        let refused = match streams.get(&call.method) {
            Some(stream) => match stream.send(call) {
                Ok(()) => return,
                Err(mpsc::error::SendError(call)) => {
                    streams.remove(&call.method);
                    call
                }
            },
            None => call,
        };
        drop(served);
        tracing::debug!(target: "goodfetch::client", method = %refused.method, "the engine sent a request no stream serves");
        refused.refuse_unserved();
    }

    /// Ends every stream: the Connection has ended.
    pub(crate) fn end(&self) {
        self.served().take();
    }

    /// The streams, which a panic elsewhere leaves as they were.
    fn served(&self) -> MutexGuard<'_, Served> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
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
    /// The Connection's output, which a held request never keeps open.
    lines: mpsc::WeakUnboundedSender<Vec<u8>>,
    answered: bool,
    /// The process whose Connection read the request, the only one whose
    /// answer reaches the engine.
    opener: OpeningProcess,
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

    /// Answers the request with `result`.
    ///
    /// # Errors
    ///
    /// `closed` when the Connection has ended, so the engine never
    /// reads the answer, and `connection_forked` in a process other than
    /// the one that opened it, such as the child of a `fork`.
    pub fn answer(mut self, result: Value) -> Result<(), Error> {
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
    /// [`Error::of_program`]).
    ///
    /// # Errors
    ///
    /// `closed` when the Connection has ended, so the engine never
    /// reads the answer, and `connection_forked` in a process other than
    /// the one that opened it, such as the child of a `fork`.
    pub fn fail(mut self, error: impl Into<BoxError>, own: ErrorCode) -> Result<(), Error> {
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
        if self.answered {
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
            tracing::debug!(target: "goodfetch::client", method = %self.method, "a dropped request's Connection had ended");
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
