//! The OpenAI-compatible chat completions format: a `chat.completion`, its
//! stream of `chat.completion.chunk` events ended by `[DONE]`, and its error
//! body.

use hyper::StatusCode;
use serde_json::{Value, json};

use super::request::Streaming;
use super::{Content, Failure, Rendering, Stream, pieces};

/// When every answer was created: a fixed time, so answers depend only on
/// the request and the seed.
const CREATED: u64 = 1_760_000_000;

const FINGERPRINT: &str = "fp_goodfetch";

fn usage(rendering: &Rendering<'_>) -> Value {
    json!({
        "prompt_tokens": rendering.input_tokens,
        "completion_tokens": rendering.output_tokens,
        "total_tokens": rendering.input_tokens + rendering.output_tokens,
        "prompt_tokens_details": { "cached_tokens": 0, "audio_tokens": 0 },
        "completion_tokens_details": {
            "reasoning_tokens": 0,
            "audio_tokens": 0,
            "accepted_prediction_tokens": 0,
            "rejected_prediction_tokens": 0,
        },
    })
}

fn finish(content: &Content) -> &'static str {
    match content {
        Content::ToolCall { .. } => "tool_calls",
        Content::Text(_) | Content::Refusal(_) => "stop",
    }
}

fn call_id(rendering: &Rendering<'_>) -> String {
    format!("call_{}", rendering.id)
}

/// The whole answer.
pub(crate) fn complete(rendering: &Rendering<'_>) -> Value {
    let message = match rendering.content {
        Content::Text(text) => json!({
            "role": "assistant", "content": text, "refusal": null, "annotations": [],
        }),
        Content::ToolCall { name, arguments } => json!({
            "role": "assistant",
            "content": null,
            "refusal": null,
            "annotations": [],
            "tool_calls": [{
                "id": call_id(rendering),
                "type": "function",
                "function": { "name": name, "arguments": arguments },
            }],
        }),
        Content::Refusal(text) => json!({
            "role": "assistant", "content": null, "refusal": text, "annotations": [],
        }),
    };
    json!({
        "id": format!("chatcmpl-{}", rendering.id),
        "object": "chat.completion",
        "created": CREATED,
        "model": rendering.ask.model,
        "choices": [{
            "index": 0,
            "message": message,
            "logprobs": null,
            "finish_reason": finish(rendering.content),
        }],
        "usage": usage(rendering),
        "service_tier": "default",
        "system_fingerprint": FINGERPRINT,
    })
}

/// The answer as a stream of chunks.
pub(crate) fn stream(rendering: &Rendering<'_>) -> Stream {
    let chunk = |choices: Value, usage: Option<Value>| {
        let mut chunk = json!({
            "id": format!("chatcmpl-{}", rendering.id),
            "object": "chat.completion.chunk",
            "created": CREATED,
            "model": rendering.ask.model,
            "service_tier": "default",
            "system_fingerprint": FINGERPRINT,
            "choices": choices,
        });
        if rendering.ask.streaming == Some(Streaming::EventsWithUsage) {
            chunk["usage"] = usage.unwrap_or(Value::Null);
        }
        format!("data: {chunk}\n\n")
    };
    let delta = |delta: Value, finish: Option<&str>| {
        chunk(
            json!([{ "index": 0, "delta": delta, "logprobs": null, "finish_reason": finish }]),
            None,
        )
    };
    let (first, text) = match rendering.content {
        Content::Text(text) => (
            json!({ "role": "assistant", "content": "", "refusal": null }),
            text,
        ),
        Content::Refusal(text) => (
            json!({ "role": "assistant", "content": null, "refusal": "" }),
            text,
        ),
        Content::ToolCall { name, arguments } => (
            json!({
                "role": "assistant",
                "content": null,
                "refusal": null,
                "tool_calls": [{
                    "index": 0,
                    "id": call_id(rendering),
                    "type": "function",
                    "function": { "name": name, "arguments": "" },
                }],
            }),
            arguments,
        ),
    };
    let mut frames = vec![delta(first, None)];
    frames.extend(pieces(text).into_iter().map(|piece| {
        delta(
            match rendering.content {
                Content::Text(_) => json!({ "content": piece }),
                Content::Refusal(_) => json!({ "refusal": piece }),
                Content::ToolCall { .. } => {
                    json!({ "tool_calls": [{ "index": 0, "function": { "arguments": piece } }] })
                }
            },
            None,
        )
    }));
    frames.push(delta(json!({}), Some(finish(rendering.content))));
    if rendering.ask.streaming == Some(Streaming::EventsWithUsage) {
        frames.push(chunk(json!([]), Some(usage(rendering))));
    }
    frames.push("data: [DONE]\n\n".to_owned());
    Stream {
        frames,
        first_content: 1,
    }
}

/// The error body and status of `failure`.
pub(crate) fn error(failure: Failure, message: &str) -> (StatusCode, Value) {
    let (status, kind, code) = match failure {
        Failure::InvalidRequest => (
            StatusCode::BAD_REQUEST,
            "invalid_request_error",
            json!("invalid_request"),
        ),
        Failure::RateLimit => (
            StatusCode::TOO_MANY_REQUESTS,
            "requests",
            json!("rate_limit_exceeded"),
        ),
        Failure::Overload => (StatusCode::SERVICE_UNAVAILABLE, "server_error", Value::Null),
    };
    (
        status,
        json!({ "error": { "message": message, "type": kind, "param": null, "code": code } }),
    )
}
