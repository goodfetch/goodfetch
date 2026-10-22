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

pub mod framing;

mod connection;
mod handshake;
mod stream;
mod websocket;

use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::connection::NOT_IN_PRODUCT;
use goodfetch_protocol::jsonrpc::{
    ErrorData, ErrorObject, ErrorResponse, Notification, RequestId, Response, Version,
};
use serde_json::Value;

use crate::composition::CallError;
use crate::product::{Identity, Requested};

pub use connection::REFUSAL_TARGET;
pub(crate) use connection::{Access, Dispatch, serve};
pub(crate) use stream::handlers;
pub use websocket::Authorities;

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

/// The line of the notification `method` with `params`.
pub(crate) fn notification_line(method: &str, params: &impl serde::Serialize) -> Vec<u8> {
    line(&Notification {
        jsonrpc: Version,
        method: method.to_owned(),
        params: Some(
            serde_json::to_value(params).expect("invariant: a notification's params encode"),
        ),
    })
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

/// The line of a success.
fn success_line(id: RequestId, result: Value) -> Vec<u8> {
    line(&Response {
        jsonrpc: Version,
        id,
        result,
    })
}

/// The line of a failure, the one conversion of an error for the wire.
fn failure_line(id: Option<RequestId>, error: &CallError) -> Vec<u8> {
    let (code, retryable, details) = error.wire_parts();
    line(&ErrorResponse {
        jsonrpc: Version,
        id,
        error: ErrorObject {
            code,
            message: error.message().to_owned(),
            data: ErrorData {
                code: error.code().to_owned(),
                retryable,
                details,
            },
        },
    })
}

fn line(message: &impl serde::Serialize) -> Vec<u8> {
    let mut line =
        serde_json::to_vec(message).expect("invariant: a message of JSON values always encodes");
    line.push(b'\n');
    line
}
