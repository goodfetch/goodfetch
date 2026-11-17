use serde_json::json;

use super::{ModelResponse, ModelResponsePart, ModelTokens};

fn tokens() -> serde_json::Value {
    json!({
        "input_tokens": 1200,
        "cache_read_tokens": 1000,
        "cache_write_tokens": 0,
        "output_tokens": 80,
        "reasoning_tokens": 30
    })
}

/// A stream ends with the whole response, its token counts included, after
/// the text it streamed in pieces.
#[test]
fn a_streams_last_part_carries_the_whole_response_and_its_tokens() {
    let parts = json!([
        { "kind": "text", "text": "The title is " },
        { "kind": "text", "text": "Dune." },
        {
            "kind": "done",
            "response": {
                "text": "The title is Dune.",
                "finish_reason": "stop",
                "usage": tokens()
            }
        }
    ]);

    let read: Vec<ModelResponsePart> = serde_json::from_value(parts.clone()).unwrap();

    let Some(ModelResponsePart::Done { response }) = read.last() else {
        panic!("the last part is no response: {read:?}");
    };
    assert_eq!(
        response.usage,
        ModelTokens {
            input_tokens: 1200,
            cache_read_tokens: 1000,
            cache_write_tokens: 0,
            output_tokens: 80,
            reasoning_tokens: 30,
        }
    );
    assert_eq!(serde_json::to_value(&read).unwrap(), parts);
}

/// A provider that reports no token counts gives no response: the driver
/// counts every call (ADR 0101), so an adapter must say what it used.
#[test]
fn a_response_without_its_token_counts_is_refused() {
    let sent = json!({ "text": "Dune.", "finish_reason": "stop" });

    let read = serde_json::from_value::<ModelResponse>(sent);

    assert!(read.is_err(), "{read:?}");
}

/// A finish reason a newer engine or provider gives is kept as it is, so a
/// safety signal this reader does not know is still journaled.
#[test]
fn a_finish_reason_this_reader_does_not_know_is_kept() {
    let sent = json!({
        "text": "",
        "finish_reason": "a_newer_reason",
        "refusal": "I can't help with that.",
        "usage": tokens()
    });

    let read: ModelResponse = serde_json::from_value(sent.clone()).unwrap();

    assert_eq!(read.finish_reason, "a_newer_reason");
    assert_eq!(serde_json::to_value(&read).unwrap(), sent);
}
