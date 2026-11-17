//! The contracts of one model call (ADRs 0012, 0101 and 0125): what the
//! engine asks a Model provider, a [`ModelRequest`], and what the provider
//! answers, a stream of [`ModelResponsePart`]s whose last is the whole
//! [`ModelResponse`] with the call's token counts, [`ModelTokens`]. Every
//! Model provider speaks them: the built-in adapters, a Plugin, the
//! program's own provider and the Scripted model in tests. What a Run's
//! model calls add up to, its `ModelUsage`, is the Run's own contract
//! (`run`), built from these counts.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// What the engine asks a Model provider in one call.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ModelRequest {
    /// The model it asks, by its Model reference: the provider's name, a
    /// slash and the provider's own identifier for the model, such as
    /// `anthropic/claude-sonnet-4-5` (ADR 0101).
    #[schemars(extend("arrived_in" = "0.1"))]
    pub model: String,
    /// The conversation so far, oldest message first.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub messages: Vec<ModelMessage>,
    /// The tools the model may call; none when it only answers.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub tools: Vec<ModelTool>,
    /// Whether the model must call one of `tools`.
    #[serde(default, skip_serializing_if = "ModelToolChoice::is_auto")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub tool_choice: ModelToolChoice,
    /// The JSON Schema the answer's text must match, as the structured
    /// output strategy adapted it to the model (ADR 0082); none for free
    /// text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub response_schema: Option<Value>,
    /// The most tokens the answer may hold, `ai.max_output_tokens` bounded
    /// by the model's declared maximum; none leaves the provider's own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub max_output_tokens: Option<u64>,
}

/// One message of a model call's conversation, by who said it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "role", rename_all = "snake_case")]
pub enum ModelMessage {
    /// The instructions the engine gives the model.
    System {
        /// Their text.
        #[schemars(extend("arrived_in" = "0.1"))]
        text: String,
    },
    /// What the model is shown: the task, an observation of the page.
    User {
        /// Its parts, in order.
        #[schemars(extend("arrived_in" = "0.1"))]
        content: Vec<ModelContent>,
    },
    /// What the model answered earlier in the conversation.
    Assistant {
        /// Its text; empty when it only called tools.
        #[serde(default, skip_serializing_if = "String::is_empty")]
        #[schemars(extend("arrived_in" = "0.1"))]
        text: String,
        /// The tools it called.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        #[schemars(extend("arrived_in" = "0.1"))]
        tool_calls: Vec<ModelToolCall>,
    },
    /// What one of the model's tool calls gave back.
    Tool {
        /// The call it answers, by the ID the model gave it.
        #[schemars(extend("arrived_in" = "0.1"))]
        tool_call_id: String,
        /// What the tool gave back, as text.
        #[schemars(extend("arrived_in" = "0.1"))]
        text: String,
    },
}

/// One part of what the model is shown.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ModelContent {
    /// Text.
    Text {
        /// The text.
        #[schemars(extend("arrived_in" = "0.1"))]
        text: String,
    },
    /// An image, such as a screenshot of the page.
    Image {
        /// Its media type, such as `image/png`.
        #[schemars(extend("arrived_in" = "0.1"))]
        media_type: String,
        /// Its bytes, in base64.
        #[schemars(extend("arrived_in" = "0.1"))]
        data: String,
    },
}

/// A tool the model may call.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ModelTool {
    /// Its name, which the model's calls give.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub name: String,
    /// What it does, for the model.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub description: String,
    /// The JSON Schema of its arguments.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub parameters: Value,
}

/// Whether the model must call a tool.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ModelToolChoice {
    /// The model answers or calls a tool, as it chooses.
    #[default]
    Auto,
    /// The model calls one of the tools.
    Required,
}

impl ModelToolChoice {
    #[expect(
        clippy::trivially_copy_pass_by_ref,
        reason = "serde's `skip_serializing_if` passes a reference"
    )]
    fn is_auto(&self) -> bool {
        matches!(self, Self::Auto)
    }
}

/// A tool call the model made.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ModelToolCall {
    /// The ID the model gave it, which the tool's answer names.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub id: String,
    /// The tool's name.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub name: String,
    /// Its arguments, the JSON text the model wrote, which the engine
    /// parses and validates against the tool's `parameters`.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub arguments: String,
}

/// One part of a Model provider's streamed answer: pieces of text as they
/// arrive, then the whole response, last.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ModelResponsePart {
    /// A piece of the answer's text.
    Text {
        /// The piece.
        #[schemars(extend("arrived_in" = "0.1"))]
        text: String,
    },
    /// The whole response, which ends the stream.
    Done {
        /// The response.
        #[schemars(extend("arrived_in" = "0.1"))]
        response: ModelResponse,
    },
}

/// A Model provider's whole answer to one call.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ModelResponse {
    /// The answer's text; empty when the model only called tools.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub text: String,
    /// The tools the model called.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub tool_calls: Vec<ModelToolCall>,
    /// Why the model stopped: `stop`, `length` (it reached
    /// `max_output_tokens`), `tool_calls`, `content_filter` or `refusal`;
    /// a reason this reader does not know is kept as it is.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub finish_reason: String,
    /// The provider's refusal message, a vendor safety signal journaled
    /// with the call (ADR 0014); none when it did not refuse.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub refusal: Option<String>,
    /// The tokens the call used, which every provider reports.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub usage: ModelTokens,
}

/// The tokens of model calls, by part, as OpenTelemetry's `GenAI` conventions
/// count them whatever the provider's own counts (ADR 0101): the cached
/// input is part of the input, and the reasoning part of the output.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ModelTokens {
    /// The input tokens, the cached ones included.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub input_tokens: u64,
    /// The input tokens read from the provider's cache, part of
    /// `input_tokens`.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub cache_read_tokens: u64,
    /// The input tokens written to the provider's cache, part of
    /// `input_tokens`.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub cache_write_tokens: u64,
    /// The output tokens, the reasoning ones included.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub output_tokens: u64,
    /// The output tokens the model spent reasoning, part of
    /// `output_tokens`.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub reasoning_tokens: u64,
}

#[cfg(test)]
mod tests;
