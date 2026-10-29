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

pub mod event_stream;
pub mod framing;

mod connection;
mod handshake;
mod stream;
mod websocket;

use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::connection::NOT_IN_PRODUCT;
use goodfetch_protocol::jsonrpc::{
    ErrorResponse, META, Notification, ProtocolFailure, RequestId, Response, TraceContext, Version,
};
use goodfetch_protocol::log::{CONNECTION_SET_LOG_LEVEL, LogLevelSet, SetLogLevelParams};
use serde_json::Value;

use crate::composition::{CallContext, CallError, Handler};
use crate::product::{Identity, Requested};

pub use connection::CONSOLE_LISTENER;
pub(crate) use connection::{
    Access, ConsoleConnection, ConsoleSide, Dispatch, serve, serve_console,
};
pub use websocket::Authorities;
pub(crate) use websocket::{ConsoleSocket, Refusal};

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

/// The line of the notification `method` with `params`.
pub(crate) fn notification_line(method: &str, params: &impl serde::Serialize) -> Vec<u8> {
    notification_value_line(
        method,
        serde_json::to_value(params).expect("invariant: a notification's params encode"),
    )
}

/// The line of the notification `method` with `params`, already a value.
pub(crate) fn notification_value_line(method: &str, params: Value) -> Vec<u8> {
    line(&Notification {
        jsonrpc: Version,
        method: method.to_owned(),
        params: Some(params),
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
    line(&ErrorResponse {
        jsonrpc: Version,
        id,
        error: error.to_error_object(),
    })
}

fn line(message: &impl serde::Serialize) -> Vec<u8> {
    let mut line =
        serde_json::to_vec(message).expect("invariant: a message of JSON values always encodes");
    line.push(b'\n');
    line
}

/// Takes the Trace context out of a request's `params`, which then go to
/// its handler without it: none when they hold no `_meta`, and
/// `invalid_params` when their `_meta` holds a key other than
/// `traceparent` and `tracestate` or a value that is no string (ADRs 0077
/// and 0132).
fn trace_context(
    params: Option<Value>,
) -> Result<(Option<Value>, Option<TraceContext>), CallError> {
    let mut members = match params {
        Some(Value::Object(members)) => members,
        params => return Ok((params, None)),
    };
    let Some(meta) = members.remove(META) else {
        return Ok((Some(Value::Object(members)), None));
    };
    let refused = |error: String| {
        CallError::protocol(
            ProtocolFailure::InvalidParams,
            format!(
                "`{META}` is an object of `traceparent` and `tracestate`, each a string: {error}"
            ),
        )
    };
    // serde would read a struct from an array too, which the schema refuses.
    if !meta.is_object() {
        return Err(refused("it is no object".to_owned()));
    }
    let meta =
        serde_json::from_value::<TraceContext>(meta).map_err(|error| refused(error.to_string()))?;
    Ok((Some(Value::Object(members)), Some(meta)))
}
