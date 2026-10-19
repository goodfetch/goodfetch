//! Gemini's `generateContent` format: a response with its candidates and
//! `usageMetadata`, its stream of the same chunks (server-sent events with
//! `alt=sse`, else one JSON array), and its error body.

use hyper::StatusCode;
use serde_json::{Value, json};

use super::{Content, Failure, Rendering, Stream, arguments_value, pieces};

fn usage(rendering: &Rendering<'_>, last: bool) -> Value {
    if last {
        json!({
            "promptTokenCount": rendering.input_tokens,
            "candidatesTokenCount": rendering.output_tokens,
            "totalTokenCount": rendering.input_tokens + rendering.output_tokens,
            "promptTokensDetails": [{ "modality": "TEXT", "tokenCount": rendering.input_tokens }],
        })
    } else {
        json!({
            "promptTokenCount": rendering.input_tokens,
            "totalTokenCount": rendering.input_tokens,
            "promptTokensDetails": [{ "modality": "TEXT", "tokenCount": rendering.input_tokens }],
        })
    }
}

/// A response or chunk with one candidate of `parts`, finished when
/// `finish` is set.
fn response(rendering: &Rendering<'_>, parts: Option<Value>, finish: Option<&str>) -> Value {
    let mut candidate = json!({ "index": 0 });
    if let Some(parts) = parts {
        candidate["content"] = json!({ "parts": parts, "role": "model" });
    }
    if let Some(finish) = finish {
        candidate["finishReason"] = json!(finish);
    }
    json!({
        "candidates": [candidate],
        "usageMetadata": usage(rendering, finish.is_some()),
        "modelVersion": rendering.ask.model,
        "responseId": rendering.id,
    })
}

/// The whole answer.
pub(crate) fn complete(rendering: &Rendering<'_>) -> Value {
    match rendering.content {
        Content::Text(text) => response(rendering, Some(json!([{ "text": text }])), Some("STOP")),
        Content::ToolCall { name, arguments } => response(
            rendering,
            Some(json!([{ "functionCall": { "name": name, "args": arguments_value(arguments) } }])),
            Some("STOP"),
        ),
        Content::Refusal(_) => response(rendering, None, Some("SAFETY")),
    }
}

/// The answer's chunks, in order: its text in pieces, the last with the
/// finish reason; a tool call or a refusal in one.
pub(crate) fn chunks(rendering: &Rendering<'_>) -> Vec<Value> {
    match rendering.content {
        Content::Text(text) => {
            let pieces = pieces(text);
            let last = pieces.len().saturating_sub(1);
            pieces
                .into_iter()
                .enumerate()
                .map(|(at, piece)| {
                    response(
                        rendering,
                        Some(json!([{ "text": piece }])),
                        (at == last).then_some("STOP"),
                    )
                })
                .collect()
        }
        Content::ToolCall { .. } | Content::Refusal(_) => vec![complete(rendering)],
    }
}

/// The answer as server-sent events.
pub(crate) fn stream(rendering: &Rendering<'_>) -> Stream {
    Stream {
        frames: chunks(rendering)
            .iter()
            .map(|chunk| format!("data: {chunk}\r\n\r\n"))
            .collect(),
        first_content: 0,
    }
}

/// The error body and status of `failure`.
pub(crate) fn error(failure: Failure, message: &str) -> (StatusCode, Value) {
    let (status, name) = match failure {
        Failure::InvalidRequest => (StatusCode::BAD_REQUEST, "INVALID_ARGUMENT"),
        Failure::RateLimit => (StatusCode::TOO_MANY_REQUESTS, "RESOURCE_EXHAUSTED"),
        Failure::Overload => (StatusCode::SERVICE_UNAVAILABLE, "UNAVAILABLE"),
    };
    let mut error = json!({ "code": status.as_u16(), "message": message, "status": name });
    if matches!(failure, Failure::RateLimit) {
        error["details"] = json!([{
            "@type": "type.googleapis.com/google.rpc.RetryInfo",
            "retryDelay": "1s",
        }]);
    }
    (status, json!({ "error": error }))
}
