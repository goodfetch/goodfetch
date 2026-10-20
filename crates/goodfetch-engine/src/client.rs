//! The Client at the other end of a Connection, as the engine's handlers
//! see it: the handler requests the engine sends it, such as a crawl's
//! `crawl_request.handle`, and their replies (ADRs 0024 and 0069). The
//! protocol server carries each request on the Connection and routes the
//! reply back; a Connection's end fails every request still waiting with
//! [`ClientRequestError::Gone`], which a run tells apart from a failed
//! handler.

use goodfetch_protocol::catalog::HandlerRequest;
use goodfetch_protocol::jsonrpc::ErrorObject;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value};
use tokio::sync::{mpsc, oneshot};

/// How many handler requests wait for the Connection's writer before their
/// senders wait.
pub(crate) const OUTGOING_REQUESTS: usize = 64;

/// A request on its way to the Client.
pub(crate) struct Outgoing {
    /// Its method.
    pub(crate) method: &'static str,
    /// Its params.
    pub(crate) params: Value,
    /// Where its reply goes.
    pub(crate) reply: oneshot::Sender<Result<Value, ErrorObject>>,
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
    /// The Connection ended before the reply: the Client is gone.
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

impl Client {
    /// A Client whose requests go to `requests`, and the end the protocol
    /// server reads them from.
    pub(crate) fn new() -> (Self, mpsc::Receiver<Outgoing>) {
        let (requests, outgoing) = mpsc::channel(OUTGOING_REQUESTS);
        (Self { requests }, outgoing)
    }

    /// Sends the handler request `request` with `params`, and answers the
    /// handler's reply read as `R`.
    ///
    /// # Errors
    ///
    /// `Gone` when the Connection ends first, `Failed` when the handler
    /// answered a failure, `Malformed` when its answer is not an `R`.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future after the request was sent leaves the handler
    /// running; its late reply is discarded.
    pub async fn request<R: DeserializeOwned>(
        &self,
        request: &'static HandlerRequest,
        params: &impl Serialize,
    ) -> Result<R, ClientRequestError> {
        let params = serde_json::to_value(params)
            .expect("invariant: a handler request's params encode as JSON");
        let (reply, replied) = oneshot::channel();
        self.requests
            .send(Outgoing {
                method: request.name,
                params,
                reply,
            })
            .await
            .map_err(|_| ClientRequestError::Gone)?;
        match replied.await.map_err(|_| ClientRequestError::Gone)? {
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
}
