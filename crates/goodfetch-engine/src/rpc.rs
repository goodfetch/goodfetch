//! The protocol server (ADRs 0024 and 0038) over each Connection
//! `transport` gives it: newline-delimited JSON-RPC 2.0, or on an Exposed
//! Daemon's address the same messages over WebSocket (ADR 0049), opened by
//! `hello`, with `$/cancel`, dispatch to the composition's handlers, and the one
//! place where a [`CallError`](crate::CallError) becomes the wire's
//! `{code, message, data: {code, retryable, …}}`. Dispatch emits one
//! `tracing` record per request, at `trace` under the target
//! [`DISPATCH_TARGET`], with its method and its reply's Error code, and one
//! per event sent, with its name, which ADR 0072's exercise audit reads,
//! and one per `$/cancel` that stops a call in flight.
//! The streaming calls, `events.subscribe` and `page.body`, send their
//! events and chunks as the credits the Client grants with `$/credit` allow.
//! Each Connection receives the engine's log events as `$/log` at the level
//! its `hello` names, which `connection.set_log_level` changes.
//! Dispatch takes each request's Trace context out of its params' `_meta`,
//! refusing one with another key with `invalid_params`, and runs its
//! handler in the request's `SERVER` span, which it alone opens; a handler
//! request the engine sends has its `CLIENT` span, whose Trace context its
//! `_meta` carries ([`crate::trace`], ADR 0132).

pub mod framing;

mod connection;
mod handshake;
mod stream;
mod websocket;

pub use self::stream::send_events;

use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::connection::NOT_IN_PRODUCT;

use goodfetch_protocol::jsonrpc::{ErrorResponse, RequestId, Version};
use goodfetch_protocol::log::{CONNECTION_SET_LOG_LEVEL, LogLevelSet, SetLogLevelParams};

use crate::call_error::CallError;
use crate::composition::{CallContext, Handler};
use crate::product::{Identity, Requested};
use crate::wire::line;

pub(crate) use crate::wire::{Line, notification_line, success_line};

pub(crate) use connection::{Access, Dispatch, serve};
pub use handshake::version_refusal;

/// The protocol server's own operations: the streaming calls and
/// `connection.set_log_level`.
pub(crate) fn handlers() -> [Handler; 3] {
    let [subscribe, body] = stream::handlers();
    [
        subscribe,
        body,
        Handler::new(&CONNECTION_SET_LOG_LEVEL, set_log_level),
    ]
}

async fn set_log_level(
    params: SetLogLevelParams,
    context: CallContext,
) -> Result<LogLevelSet, CallError> {
    context.log().set(params.log_level);
    Ok(LogLevelSet {})
}

/// The `tracing` target of dispatch's records.
pub const DISPATCH_TARGET: &str = "goodfetch::rpc::dispatch";

/// Records one request: its method and its reply's Error code, if any.
fn record(method: &str, error_code: Option<&str>) {
    match error_code {
        None => tracing::trace!(target: DISPATCH_TARGET, method, "request answered"),
        Some(error_code) => {
            tracing::trace!(target: DISPATCH_TARGET, method, error_code, "request failed");
        }
    }
}

/// Records a `$/cancel` that stopped a call in flight.
fn record_cancel(notification: &str) {
    tracing::trace!(target: DISPATCH_TARGET, notification, "cancel received");
}

/// Records one event sent to a Client: its name.
fn record_event(event: &str) {
    tracing::trace!(target: DISPATCH_TARGET, event, "event sent");
}

/// `not_in_product` for `requested`, which `needs` carries.
fn not_in_product(
    identity: &Identity,
    requested: Requested<'_>,
    needs: Product,
    behind_gateway: bool,
) -> CallError {
    let (message, details) = identity.not_in_product(requested, needs, behind_gateway);
    CallError::new(&NOT_IN_PRODUCT, message, &details)
}

/// The line of a failure, the one conversion of an error for the wire.
fn failure_line(id: Option<RequestId>, error: &CallError) -> Vec<u8> {
    line(&ErrorResponse {
        jsonrpc: Version,
        id,
        error: error.to_error_object(),
    })
}
