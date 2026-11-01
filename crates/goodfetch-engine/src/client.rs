//! The Client at the other end of a Connection, as the engine's handlers
//! see it: the handler requests the engine sends it, such as a crawl's
//! `crawl_request.handle`, and their replies (ADRs 0024 and 0069). The
//! protocol server carries each request on the Connection under an id of
//! the engine's own and routes the reply back; a sender that stops waiting
//! early, such as an attempt whose lease ended, cancels the request with
//! `$/cancel` and may still read the answer that comes (ADR 0093). A
//! Connection's end fails every request still waiting with
//! [`ClientRequestError::Gone`], which a run tells apart from a failed
//! handler. Each request is sent in a `CLIENT` span of its own, a child of
//! the sender's span, whose Trace context its params' `_meta` carries while
//! the engine exports its spans (ADR 0132).

use std::marker::PhantomData;

use goodfetch_protocol::catalog::HandlerRequest;
use goodfetch_protocol::jsonrpc::{ErrorObject, META, RequestId};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value};
use tokio::sync::{mpsc, oneshot};

use crate::trace;

/// How many handler requests wait for the Connection's writer before their
/// senders wait.
pub(crate) const OUTGOING_REQUESTS: usize = 64;

/// What the engine's handlers give the Connection to send.
pub(crate) enum Outgoing {
    /// A handler request on its way to the Client.
    Request {
        /// Its method.
        method: &'static str,
        /// Its params.
        params: Value,
        /// Where the id it is sent under goes.
        sent: oneshot::Sender<RequestId>,
        /// Where its reply goes.
        reply: oneshot::Sender<Result<Value, ErrorObject>>,
    },
    /// `$/cancel` of the request sent under this id, unless it is answered.
    Cancel(RequestId),
}

/// The Client of one Connection, to which the engine sends handler
/// requests.
#[derive(Clone, Debug)]
pub struct Client {
    requests: mpsc::Sender<Outgoing>,
}

/// Why a handler request got no answer.
#[derive(Clone, Debug, PartialEq, thiserror::Error)]
#[non_exhaustive]
pub enum ClientRequestError {
    /// The Connection ended before the reply, or the answer was read
    /// already: the Client is gone.
    #[error("the Client's Connection ended before it answered")]
    Gone,
    /// The handler answered with a failure.
    #[error("the handler failed with `{code}`: {message}")]
    Failed {
        /// Its Error code, or `handler_failed` when its failure named none.
        code: String,
        /// Whether it says a retry may succeed.
        retryable: bool,
        /// What it said.
        message: String,
        /// The code's details.
        details: Map<String, Value>,
    },
    /// The handler answered something that is not the request's result.
    #[error("the handler's answer is not the request's result: {0}")]
    Malformed(String),
}

/// A handler request on the Connection, whose answer `R` is still to come.
#[derive(Debug)]
pub struct Sent<R> {
    id: RequestId,
    /// Where its reply comes, until it is read.
    replied: Option<oneshot::Receiver<Result<Value, ErrorObject>>>,
    requests: mpsc::Sender<Outgoing>,
    /// Its `CLIENT` span, which ends once its answer is read or it is
    /// dropped (ADR 0132).
    span: tracing::Span,
    answer: PhantomData<fn() -> R>,
}

impl Client {
    /// A Client whose requests go to `requests`, and the end the protocol
    /// server reads them from.
    pub(crate) fn new() -> (Self, mpsc::Receiver<Outgoing>) {
        let (requests, outgoing) = mpsc::channel(OUTGOING_REQUESTS);
        (Self { requests }, outgoing)
    }

    /// Sends the handler request `request` with `params`, and answers once
    /// it is on its way, its answer to be read as `R`.
    ///
    /// # Errors
    ///
    /// `Gone` when the Connection ends first.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future may still send the request; its reply is then
    /// discarded.
    pub async fn send<R: DeserializeOwned>(
        &self,
        request: &'static HandlerRequest,
        params: &impl Serialize,
    ) -> Result<Sent<R>, ClientRequestError> {
        let span = trace::client_span(request.name);
        let mut params = serde_json::to_value(params)
            .expect("invariant: a handler request's params encode as JSON");
        if let (Some(meta), Value::Object(members)) = (trace::sent(&span), &mut params) {
            members.insert(
                META.to_owned(),
                serde_json::to_value(meta).expect("invariant: a Trace context encodes as JSON"),
            );
        }
        let (sent, assigned) = oneshot::channel();
        let (reply, replied) = oneshot::channel();
        self.requests
            .send(Outgoing::Request {
                method: request.name,
                params,
                sent,
                reply,
            })
            .await
            .map_err(|_| ClientRequestError::Gone)?;
        let id = assigned.await.map_err(|_| ClientRequestError::Gone)?;
        trace::record_id(&span, &id);
        Ok(Sent {
            id,
            replied: Some(replied),
            requests: self.requests.clone(),
            span,
            answer: PhantomData,
        })
    }
}

impl<R: DeserializeOwned> Sent<R> {
    /// The handler's answer, read as `R`.
    ///
    /// # Errors
    ///
    /// `Gone` when the Connection ends first, or once the answer was read,
    /// `Failed` when the handler answered a failure, `Malformed` when its
    /// answer is not an `R`.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future only stops waiting: the answer stays for the
    /// next call.
    pub async fn answer(&mut self) -> Result<R, ClientRequestError> {
        let replied = self.replied.as_mut().ok_or(ClientRequestError::Gone)?;
        let answer = replied.await;
        self.replied = None;
        let answer = answer.map_err(|_| ClientRequestError::Gone)?;
        if let Err(error) = &answer {
            trace::record_failure(&self.span, &error.data.code, error.code);
        }
        // The span ends with the answer.
        self.span = tracing::Span::none();
        match answer {
            Ok(result) => serde_json::from_value(result)
                .map_err(|error| ClientRequestError::Malformed(error.to_string())),
            Err(error) => Err(ClientRequestError::Failed {
                code: error.data.code,
                retryable: error.data.retryable,
                message: error.message,
                details: error.data.details,
            }),
        }
    }

    /// Sends the Client `$/cancel` for the request, unless it is answered
    /// already; the answer the Client still gives is read through
    /// [`Sent::answer`].
    ///
    /// # Cancel safety
    ///
    /// Dropping the future may leave the `$/cancel` unsent.
    pub async fn cancel(&self) {
        let _ = self.requests.send(Outgoing::Cancel(self.id.clone())).await;
    }
}
