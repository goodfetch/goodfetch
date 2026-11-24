//! One reply of a script: how the Scripted model answers one call.

use std::error::Error;

use goodfetch_protocol::model::{ModelResponse, ModelResponsePart, ModelTokens, ModelToolCall};
use serde_json::Value;

/// How the Scripted model answers one call: text, tool calls, structured
/// output, a stream of chunks, each with the token usage the call reports,
/// or a provider error.
#[derive(Debug)]
pub struct Reply {
    kind: Kind,
}

#[derive(Debug)]
enum Kind {
    /// The pieces of text the stream yields, then the whole response.
    Answer {
        chunks: Vec<String>,
        response: ModelResponse,
    },
    /// The provider's failure, which ends the stream.
    Failure(Box<dyn Error + Send + Sync>),
}

impl Reply {
    /// The model answers `text`, in one piece.
    pub fn text(text: impl Into<String>, usage: ModelTokens) -> Self {
        Self::stream([text.into()], usage)
    }

    /// The model calls `calls`, with no text.
    pub fn tool_calls(calls: Vec<ModelToolCall>, usage: ModelTokens) -> Self {
        Self::answer(
            Vec::new(),
            ModelResponse {
                text: String::new(),
                tool_calls: calls,
                finish_reason: "tool_calls".to_owned(),
                refusal: None,
                usage,
            },
        )
    }

    /// The model answers `output` as structured output: its JSON text, in
    /// one piece.
    pub fn structured(output: &Value, usage: ModelTokens) -> Self {
        Self::text(output.to_string(), usage)
    }

    /// The model answers in `chunks`, one piece each, the whole text their
    /// concatenation.
    pub fn stream(chunks: impl IntoIterator<Item = impl Into<String>>, usage: ModelTokens) -> Self {
        let chunks: Vec<String> = chunks.into_iter().map(Into::into).collect();
        let response = ModelResponse {
            text: chunks.concat(),
            tool_calls: Vec::new(),
            finish_reason: "stop".to_owned(),
            refusal: None,
            usage,
        };
        Self::answer(chunks, response)
    }

    /// The provider fails the call with `error`.
    pub fn error(error: impl Error + Send + Sync + 'static) -> Self {
        Self {
            kind: Kind::Failure(Box::new(error)),
        }
    }

    fn answer(chunks: Vec<String>, response: ModelResponse) -> Self {
        Self {
            kind: Kind::Answer { chunks, response },
        }
    }

    /// What the call's stream yields: each piece of text, then the whole
    /// response; or the provider's failure alone.
    pub(crate) fn into_parts(self) -> Vec<Result<ModelResponsePart, Box<dyn Error + Send + Sync>>> {
        match self.kind {
            Kind::Answer { chunks, response } => chunks
                .into_iter()
                .map(|text| Ok(ModelResponsePart::Text { text }))
                .chain([Ok(ModelResponsePart::Done { response })])
                .collect(),
            Kind::Failure(error) => vec![Err(error)],
        }
    }
}
