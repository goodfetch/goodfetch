//! What the JSON-RPC protocols' corpus writers share (ADR 0066): the
//! messages of a transcript built from the protocol's own types, and the
//! file that holds one side's, one message per line.

use std::path::Path;

use goodfetch_protocol::jsonrpc::{
    META, Message, Request, RequestId, RequestMeta, Response, Version,
};
use serde::Serialize;
use serde_json::Value;

/// The Trace context of the calling side's span, which every call carries.
const TRACEPARENT: &str = "00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01";

/// `value` as JSON.
pub(crate) fn to_value(value: &impl Serialize) -> Value {
    serde_json::to_value(value).expect("invariant: the protocol's types serialize")
}

/// The calling side's call of `method` with `params`, which carries its
/// Trace context in its `_meta` (ADR 0132).
pub(crate) fn call(id: RequestId, method: &str, params: &impl Serialize) -> Message {
    let mut params = to_value(params);
    let meta = to_value(&RequestMeta {
        traceparent: Some(TRACEPARENT.to_owned()),
        ..RequestMeta::default()
    });
    params
        .as_object_mut()
        .expect("invariant: every call's params are an object")
        .insert(META.to_owned(), meta);
    Message::Request(Request {
        jsonrpc: Version,
        id,
        method: method.to_owned(),
        params: Some(params),
    })
}

/// The answering side's success reply to the call `id`.
pub(crate) fn result(id: RequestId, result: &impl Serialize) -> Message {
    Message::Response(Response {
        jsonrpc: Version,
        id,
        result: to_value(result),
    })
}

/// Writes `messages` to `path`, one per line.
pub(crate) fn write(path: &Path, messages: &[Message]) {
    let mut text = String::new();
    for message in messages {
        text.push_str(&serde_json::to_string(message).expect("invariant: a message serializes"));
        text.push('\n');
    }
    std::fs::write(path, text).expect("invariant: the output folder is writable");
}
