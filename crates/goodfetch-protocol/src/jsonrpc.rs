//! JSON-RPC 2.0's messages as Goodfetch's protocol frames them (ADRs 0024
//! and 0038): one message per line, never a batch, every failure an error
//! object whose `data` names its Error code and says whether a retry may
//! succeed. A message is the JSON-RPC protocol's unit in the format
//! registry, whose recorded transcripts hold one per line (ADR 0066).

use std::borrow::Cow;

use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::catalog::ErrorGroup;

/// The `jsonrpc` member every message carries, `"2.0"`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Version;

impl Serialize for Version {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str("2.0")
    }
}

impl<'de> Deserialize<'de> for Version {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = Cow::<str>::deserialize(deserializer)?;
        if text == "2.0" {
            Ok(Self)
        } else {
            Err(serde::de::Error::custom("`jsonrpc` is not \"2.0\""))
        }
    }
}

impl JsonSchema for Version {
    fn schema_name() -> Cow<'static, str> {
        "JsonRpcVersion".into()
    }

    fn json_schema(_generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "description": "JSON-RPC's version, always \"2.0\".",
            "const": "2.0",
        })
    }
}

/// A request's id, which its reply carries back.
#[derive(
    Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(untagged)]
pub enum RequestId {
    /// A number the Client chose.
    Number(i64),
    /// A string the Client chose.
    Text(String),
}

/// A call that expects a reply.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Request {
    /// Always `"2.0"`.
    pub jsonrpc: Version,
    /// What the reply carries back.
    pub id: RequestId,
    /// The operation, or `hello`.
    pub method: String,
    /// Its params, a JSON object.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

/// The member of a request's params that holds its [`RequestMeta`] (ADR
/// 0132), an envelope field, not a catalog parameter: `xtask codegen` adds
/// it to every request's params, and no generator gives it a member.
pub const META: &str = "_meta";

/// What a request's `_meta` holds (ADR 0132), which its runtime writes and
/// never a program: its Trace context, with no Baggage, and the raw mark of
/// a call a program made through the raw `call` (ADR 0099). A closed
/// object: the engine refuses any other key. The schema checks only that
/// each Trace context field is a string: a receiver drops a value it cannot
/// use, and never fails the call for it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RequestMeta {
    /// The caller's span, as W3C Trace Context's `traceparent` header
    /// holds it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub traceparent: Option<String>,
    /// Its vendors' values, as W3C Trace Context's `tracestate` header
    /// holds them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub tracestate: Option<String>,
    /// Whether the program sent the call through the raw `call`, the way
    /// down to an operation its SDK does not generate (ADR 0099): an SDK
    /// sets it on that call alone, and refuses a `_meta` the program writes
    /// into its params, so a program neither forges nor removes it. The
    /// engine validates and enforces a raw call as any other.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub raw: bool,
}

impl RequestMeta {
    /// Its Trace context.
    #[must_use]
    pub fn trace_context(self) -> TraceContext {
        TraceContext {
            traceparent: self.traceparent,
            tracestate: self.tracestate,
        }
    }
}

/// A request's Trace context (ADR 0132): W3C Trace Context's `traceparent`
/// and `tracestate`, as the engine reads them from a request's
/// [`RequestMeta`] and writes them as the `_meta` of a request it sends,
/// which carries no raw mark.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct TraceContext {
    /// The caller's span, as W3C Trace Context's `traceparent` header
    /// holds it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub traceparent: Option<String>,
    /// Its vendors' values, as W3C Trace Context's `tracestate` header
    /// holds them.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tracestate: Option<String>,
}

/// A message that expects no reply, such as `$/cancel`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Notification {
    /// Always `"2.0"`.
    pub jsonrpc: Version,
    /// What it says.
    pub method: String,
    /// Its params, a JSON object.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

/// The reply to a call that succeeded.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Response {
    /// Always `"2.0"`.
    pub jsonrpc: Version,
    /// The call's id.
    pub id: RequestId,
    /// What the call answered.
    pub result: Value,
}

/// The reply to a call that failed.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ErrorResponse {
    /// Always `"2.0"`.
    pub jsonrpc: Version,
    /// The call's id, or `null` when the engine could not read one.
    pub id: Option<RequestId>,
    /// The failure.
    pub error: ErrorObject,
}

/// A failure on the wire (ADR 0038).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ErrorObject {
    /// One of JSON-RPC's predefined codes for a protocol failure; otherwise
    /// the numeric class of the Error code's group, from −32000 to −32099.
    pub code: i32,
    /// What went wrong, for people; never matched on.
    pub message: String,
    /// The Error code and its details.
    pub data: ErrorData,
}

/// What a failure's `data` holds: its Error code, whether the same call may
/// succeed when made again, and the code's own details.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ErrorData {
    /// The Error code, or the name of the protocol failure.
    pub code: String,
    /// Whether the same call may succeed when made again.
    pub retryable: bool,
    /// The code's details, such as `fix`.
    #[serde(flatten)]
    pub details: Map<String, Value>,
}

/// Any message of the protocol, as one line of a transcript holds it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum Message {
    /// A call.
    Request(Request),
    /// A message that expects no reply.
    Notification(Notification),
    /// A success.
    Response(Response),
    /// A failure.
    Error(ErrorResponse),
}

/// A failure of the protocol itself, which takes one of the codes JSON-RPC
/// 2.0 predefines (ADR 0038) and maps to a group as ADR 0079 says.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ProtocolFailure {
    /// A line that is no JSON.
    ParseError,
    /// JSON that is no request or notification this protocol reads, a
    /// batch included, or a connection that does not open with `hello`.
    InvalidRequest,
    /// A line longer than the engine reads.
    MessageTooLarge,
    /// JSON nested deeper than the engine reads.
    MessageTooDeep,
    /// An operation no Product carries.
    MethodNotFound,
    /// Params that are not the operation's.
    InvalidParams,
    /// A failure inside the RPC layer itself, such as a reply it cannot
    /// encode.
    InternalError,
}

impl ProtocolFailure {
    /// Every failure of the protocol itself, in declaration order.
    pub const ALL: [Self; 7] = [
        Self::ParseError,
        Self::InvalidRequest,
        Self::MessageTooLarge,
        Self::MessageTooDeep,
        Self::MethodNotFound,
        Self::InvalidParams,
        Self::InternalError,
    ];

    /// Its name, which the error's `data.code` carries.
    pub fn name(self) -> &'static str {
        match self {
            Self::ParseError => "parse_error",
            Self::InvalidRequest => "invalid_request",
            Self::MessageTooLarge => "message_too_large",
            Self::MessageTooDeep => "message_too_deep",
            Self::MethodNotFound => "method_not_found",
            Self::InvalidParams => "invalid_params",
            Self::InternalError => "internal_error",
        }
    }

    /// The JSON-RPC code it takes.
    pub fn code(self) -> i32 {
        match self {
            Self::ParseError => -32700,
            Self::InvalidRequest | Self::MessageTooLarge | Self::MessageTooDeep => -32600,
            Self::MethodNotFound => -32601,
            Self::InvalidParams => -32602,
            Self::InternalError => -32603,
        }
    }

    /// The group an SDK raises it in (ADR 0079).
    pub fn group(self) -> ErrorGroup {
        match self {
            Self::MethodNotFound | Self::InvalidParams => ErrorGroup::Program,
            Self::ParseError
            | Self::InvalidRequest
            | Self::MessageTooLarge
            | Self::MessageTooDeep
            | Self::InternalError => ErrorGroup::Internal,
        }
    }
}

#[cfg(test)]
mod tests;
