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
use std::fmt;

use goodfetch_protocol::jsonrpc::{
    ErrorResponse, META, ProtocolFailure, RequestId, TraceContext, Version,
};
use goodfetch_protocol::log::{CONNECTION_SET_LOG_LEVEL, LogLevelSet, SetLogLevelParams};
use serde::de::{IgnoredAny, MapAccess, Visitor};
use serde::{Deserialize, Deserializer};
use serde_json::value::RawValue;

use crate::call_error::CallError;
use crate::composition::params::without_position;
use crate::composition::{CallContext, Handler};
use crate::product::{Identity, Requested};
use crate::trace::{self, ID_FIELDS};
use crate::wire::{Key, KeySeed, line};

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

/// What dispatch reads of a request's params before its handler, which
/// reads them without their `_meta`: the Trace context it holds and the
/// Run ID and Session ID the request's span records (ADRs 0077 and 0132).
#[derive(Debug, Default)]
pub(crate) struct Preamble<'a> {
    /// The Trace context of their `_meta`.
    pub(crate) meta: Option<TraceContext>,
    /// Their Run ID and Session ID.
    pub(crate) ids: trace::Ids<'a>,
}

impl<'a> Preamble<'a> {
    /// Reads it from a request's `params` where they are, each member the
    /// last of its name: no Trace context when they hold no `_meta`, and
    /// `invalid_params` when their `_meta` holds a key other than
    /// `traceparent` and `tracestate` or a value that is no string. Params
    /// that are no object hold none, and their handler refuses them.
    pub(crate) fn read(params: Option<&'a str>) -> Result<Self, CallError> {
        let Some(Members { meta, ids }) =
            params.and_then(|params| serde_json::from_str::<Members<'a>>(params).ok())
        else {
            return Ok(Self::default());
        };
        let meta = meta.map(trace_context).transpose()?;
        Ok(Self { meta, ids })
    }
}

/// The members of a request's params dispatch reads, where they are.
#[derive(Default)]
struct Members<'a> {
    meta: Option<&'a RawValue>,
    ids: trace::Ids<'a>,
}

impl<'de> Deserialize<'de> for Members<'de> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_map(MembersVisitor)
    }
}

struct MembersVisitor;

impl<'de> Visitor<'de> for MembersVisitor {
    type Value = Members<'de>;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("an object")
    }

    fn visit_map<A: MapAccess<'de>>(self, mut members: A) -> Result<Self::Value, A::Error> {
        let mut found = Members::default();
        while let Some(Key(key)) = members.next_key_seed(KeySeed)? {
            if key == META {
                found.meta = Some(members.next_value()?);
            } else if let Some(at) = ID_FIELDS.iter().position(|field| *field == key) {
                let value: &RawValue = members.next_value()?;
                found.ids[at] = framing::text(value);
            } else {
                members.next_value::<IgnoredAny>()?;
            }
        }
        Ok(found)
    }
}

/// The Trace context a request's `_meta` holds.
fn trace_context(meta: &RawValue) -> Result<TraceContext, CallError> {
    let refused = |error: String| {
        CallError::protocol(
            ProtocolFailure::InvalidParams,
            format!(
                "`{META}` is an object of `traceparent` and `tracestate`, each a string: {error}"
            ),
        )
    };
    // serde would read a struct from an array too, which the schema refuses.
    if !meta.get().starts_with('{') {
        return Err(refused("it is no object".to_owned()));
    }
    serde_json::from_str(meta.get()).map_err(|error| refused(without_position(&error)))
}
