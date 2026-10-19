//! The Anthropic Messages format: a `message`, its stream of named events
//! (`message_start`, `content_block_start`, `ping`, `content_block_delta`,
//! `content_block_stop`, `message_delta`, `message_stop`), and its error
//! body.

use hyper::StatusCode;
use serde_json::{Value, json};

use super::{Content, Failure, Rendering, Stream, arguments_value, pieces};

fn stop_reason(content: &Content) -> &'static str {
    match content {
        Content::Text(_) => "end_turn",
        Content::ToolCall { .. } => "tool_use",
        Content::Refusal(_) => "refusal",
    }
}

fn usage(rendering: &Rendering<'_>, output_tokens: u64) -> Value {
    json!({
        "input_tokens": rendering.input_tokens,
        "cache_creation_input_tokens": 0,
        "cache_read_input_tokens": 0,
        "output_tokens": output_tokens,
        "service_tier": "standard",
    })
}

fn tool_id(rendering: &Rendering<'_>) -> String {
    format!("toolu_{}", rendering.id)
}

/// The whole answer.
pub(crate) fn complete(rendering: &Rendering<'_>) -> Value {
    let block = match rendering.content {
        Content::Text(text) | Content::Refusal(text) => json!({ "type": "text", "text": text }),
        Content::ToolCall { name, arguments } => json!({
            "type": "tool_use",
            "id": tool_id(rendering),
            "name": name,
            "input": arguments_value(arguments),
        }),
    };
    json!({
        "id": format!("msg_{}", rendering.id),
        "type": "message",
        "role": "assistant",
        "model": rendering.ask.model,
        "content": [block],
        "stop_reason": stop_reason(rendering.content),
        "stop_sequence": null,
        "usage": usage(rendering, rendering.output_tokens),
    })
}

/// The answer as a stream of named events.
pub(crate) fn stream(rendering: &Rendering<'_>) -> Stream {
    let event = |value: Value| {
        let kind = value["type"].as_str().unwrap_or_default().to_owned();
        format!("event: {kind}\ndata: {value}\n\n")
    };
    let (block, text, delta_kind, delta_field) = match rendering.content {
        Content::Text(text) | Content::Refusal(text) => (
            json!({ "type": "text", "text": "" }),
            text,
            "text_delta",
            "text",
        ),
        Content::ToolCall { name, arguments } => (
            json!({ "type": "tool_use", "id": tool_id(rendering), "name": name, "input": {} }),
            arguments,
            "input_json_delta",
            "partial_json",
        ),
    };
    let mut frames = vec![
        event(json!({
            "type": "message_start",
            "message": {
                "id": format!("msg_{}", rendering.id),
                "type": "message",
                "role": "assistant",
                "model": rendering.ask.model,
                "content": [],
                "stop_reason": null,
                "stop_sequence": null,
                "usage": usage(rendering, 1),
            },
        })),
        event(json!({ "type": "content_block_start", "index": 0, "content_block": block })),
        event(json!({ "type": "ping" })),
    ];
    frames.extend(pieces(text).into_iter().map(|piece| {
        let mut delta = json!({ "type": delta_kind });
        delta[delta_field] = json!(piece);
        event(json!({ "type": "content_block_delta", "index": 0, "delta": delta }))
    }));
    frames.extend([
        event(json!({ "type": "content_block_stop", "index": 0 })),
        event(json!({
            "type": "message_delta",
            "delta": { "stop_reason": stop_reason(rendering.content), "stop_sequence": null },
            "usage": { "output_tokens": rendering.output_tokens },
        })),
        event(json!({ "type": "message_stop" })),
    ]);
    Stream {
        frames,
        first_content: 3,
    }
}

/// The error body and status of `failure`.
pub(crate) fn error(failure: Failure, id: &str, message: &str) -> (StatusCode, Value) {
    let (status, kind) = match failure {
        Failure::InvalidRequest => (StatusCode::BAD_REQUEST, "invalid_request_error"),
        Failure::RateLimit => (StatusCode::TOO_MANY_REQUESTS, "rate_limit_error"),
        Failure::Overload => (
            StatusCode::from_u16(529).expect("invariant: 529 is a status code"),
            "overloaded_error",
        ),
    };
    (
        status,
        json!({
            "type": "error",
            "error": { "type": kind, "message": message },
            "request_id": format!("req_{id}"),
        }),
    )
}
