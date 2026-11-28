//! The caller as a Secret provider (ADRs 0067, 0092 and 0104): Rust declares
//! the program's resolver on `ConnectOptions::resolve_secret()` and reads
//! `secrets.resolve` requests from `collie.secret_requests()`, an
//! [`EventStream`] with one reader. Each [`SecretRequest`] is answered once
//! with [`SecretRequest::answer`] or failed with [`SecretRequest::fail`];
//! nothing here keeps a copy of a resolved value, logs it or echoes a failed
//! error's message into the JSON-RPC error reply.

use std::fmt;
use std::pin::Pin;
use std::task::{Context, Poll};

use goodfetch_protocol::error_code::ErrorCode;
use goodfetch_protocol::run::RunId;
use goodfetch_protocol::secrets::{
    SECRET_UNAVAILABLE, SecretFailure, SecretResolved, SecretUnavailableDetails,
    SecretsResolveParams,
};
use goodfetch_protocol::settings::SettingInvalidDetails;
use serde_json::{Map, json};

use crate::connection::ClientConnection;
use crate::error::{BoxError, Error};
use crate::event_stream::EventStream;
use crate::handler::{HandlerCall, HandlerRequests};

/// One `secrets.resolve` request from the engine for a `caller:<name>` Secret
/// reference (ADR 0092). Answer it once with [`SecretRequest::answer`] or
/// [`SecretRequest::fail`]. Its [`fmt::Debug`] shows only the reference's
/// name, Run ID and domain, never a value.
pub struct SecretRequest {
    params: SecretsResolveParams,
    call: HandlerCall,
}

impl fmt::Debug for SecretRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SecretRequest")
            .field("name", &self.params.name)
            .field("run_id", &self.params.run_id)
            .field("domain", &self.params.domain)
            .finish_non_exhaustive()
    }
}

impl SecretRequest {
    /// The reference's name, what follows `caller:`.
    pub fn name(&self) -> &str {
        &self.params.name
    }

    /// The Run that uses it, which this Connection opened.
    pub fn run_id(&self) -> &RunId {
        &self.params.run_id
    }

    /// The host the value is sent to, where the use has one.
    pub fn domain(&self) -> Option<&str> {
        self.params.domain.as_deref()
    }

    /// Answers the engine with `value` and drops the request without keeping
    /// or logging a copy (ADRs 0067 and 0092).
    ///
    /// # Errors
    ///
    /// `closed` when the Connection has ended, and `connection_forked` in a
    /// process other than the one that opened it.
    pub fn answer(self, value: impl Into<String>) -> Result<(), Error> {
        let resolved = SecretResolved {
            value: value.into(),
        };
        let payload = serde_json::to_value(resolved)
            .expect("invariant: SecretResolved serializes as an object");
        self.call.answer(payload)
    }

    /// Fails the request with a redacted `secret_unavailable` reply: keeps
    /// only a [`SecretFailure`] tag if `error` is a `secret_unavailable`
    /// [`Error`], and never echoes `error`'s message or chain into the
    /// JSON-RPC error reply so no Secret canary can leak (ADR 0067).
    ///
    /// # Errors
    ///
    /// `closed` when the Connection has ended, and `connection_forked` in a
    /// process other than the one that opened it.
    pub fn fail(self, error: impl Into<BoxError>) -> Result<(), Error> {
        let failure = failure_of(error.into());
        let redacted = redacted_unavailable(failure);
        self.call.fail(redacted, ErrorCode::SECRET_UNAVAILABLE)
    }
}

/// Turns a [`HandlerRequests`] stream of `secrets.resolve` calls into an
/// [`EventStream<SecretRequest>`].
fn stream(requests: HandlerRequests) -> EventStream<SecretRequest> {
    EventStream::from_items(SecretRequestItems(requests))
}

impl ClientConnection {
    /// Marks this Connection as having declared `ConnectOptions::resolve_secret()`
    /// so [`ClientConnection::secret_requests`] may open the `secrets.resolve`
    /// stream (ADRs 0092 and 0104).
    pub fn declare_resolve_secret(&self) {
        self.mark_resolve_secret_declared();
    }

    /// The engine's `secrets.resolve` requests for `caller:<name>` Secret
    /// references of Runs opened on this Connection (ADRs 0092 and 0104).
    ///
    /// # Errors
    ///
    /// `setting_invalid` when [`ClientConnection::declare_resolve_secret`] was
    /// not called or when another `secret_requests()` stream is already active,
    /// and `closed` once the Connection has ended.
    pub fn secret_requests(&self) -> Result<EventStream<SecretRequest>, Error> {
        if !self.is_resolve_secret_declared() {
            return Err(not_declared());
        }
        match self.handler_requests(goodfetch_protocol::secrets::SECRETS_RESOLVE.name) {
            Ok(requests) => Ok(stream(requests)),
            Err(error) if error.code() == ErrorCode::INTERNAL => Err(already_reading()),
            Err(error) => Err(error),
        }
    }
}

/// `setting_invalid` when `secret_requests()` is called without declaring
/// `ConnectOptions::resolve_secret()`.
pub(crate) fn not_declared() -> Error {
    Error::setting_invalid(
        "secret_requests() requires ConnectOptions::resolve_secret() to be declared when connecting"
            .to_owned(),
        &SettingInvalidDetails {
            key: "resolve_secret".to_owned(),
            layer: None,
            reason: "secret_requests() was called without ConnectOptions::resolve_secret()"
                .to_owned(),
            fix: Some(
                "Declare ConnectOptions::resolve_secret() when connecting before calling secret_requests()."
                    .to_owned(),
            ),
            limit: None,
        },
    )
}

/// `setting_invalid` when a second `secret_requests()` stream is opened while
/// one is already live.
pub(crate) fn already_reading() -> Error {
    Error::setting_invalid(
        "secret_requests() already has an active reader on this Connection".to_owned(),
        &SettingInvalidDetails {
            key: "resolve_secret".to_owned(),
            layer: None,
            reason: "secret_requests() allows only one reader at a time".to_owned(),
            fix: Some(
                "Drop the existing secret_requests() stream before opening another.".to_owned(),
            ),
            limit: None,
        },
    )
}

/// Extracts a [`SecretFailure`] from `error` if it is a `secret_unavailable`
/// [`Error`], defaulting to [`SecretFailure::ResolverFailed`].
fn failure_of(error: BoxError) -> SecretFailure {
    match error.downcast::<Error>() {
        Ok(sdk_error) if sdk_error.code() == ErrorCode::SECRET_UNAVAILABLE => sdk_error
            .details::<SecretUnavailableDetails>()
            .map_or(SecretFailure::ResolverFailed, |details| details.failure),
        _ => SecretFailure::ResolverFailed,
    }
}

/// Builds a redacted `secret_unavailable` [`Error`] carrying only `failure`
/// and a fixed message with no text from the caller's error.
fn redacted_unavailable(failure: SecretFailure) -> Error {
    let mut details = Map::new();
    details.insert("failure".to_owned(), json!(failure.as_str()));
    Error::of_protocol_details(
        &SECRET_UNAVAILABLE,
        "resolveSecret failed".to_owned(),
        details,
    )
}

/// Stream adapter turning [`HandlerRequests`] into `Result<SecretRequest, Error>`.
struct SecretRequestItems(HandlerRequests);

impl futures_core::Stream for SecretRequestItems {
    type Item = Result<SecretRequest, Error>;

    fn poll_next(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        match self.0.poll_recv(context) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(None) => Poll::Ready(None),
            Poll::Ready(Some(call)) => {
                if let Ok(params) =
                    serde_json::from_value::<SecretsResolveParams>(call.params().clone())
                {
                    Poll::Ready(Some(Ok(SecretRequest { params, call })))
                } else {
                    let _ = call.fail(
                        redacted_unavailable(SecretFailure::ResolverFailed),
                        ErrorCode::SECRET_UNAVAILABLE,
                    );
                    Poll::Ready(Some(Err(redacted_unavailable(
                        SecretFailure::ResolverFailed,
                    ))))
                }
            }
        }
    }
}

/// C-SEND-SYNC: a `SecretRequest` moves between threads and is `Send + Sync + 'static`.
const _: () = {
    const fn send_sync_static<T: Send + Sync + 'static>() {}
    send_sync_static::<SecretRequest>();
};
