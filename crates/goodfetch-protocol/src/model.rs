//! The contracts of one model call (ADRs 0012, 0101 and 0125): what the
//! engine asks a Model provider, a [`ModelRequest`], and what the provider
//! answers, a stream of [`ModelResponsePart`]s whose last is the whole
//! [`ModelResponse`] with the call's token counts, [`ModelTokens`]. Every
//! Model provider speaks them: the built-in adapters, a Plugin, the
//! program's own provider and the Scripted model in tests. What a Run's
//! model calls add up to, its `ModelUsage`, is the Run's own contract
//! (`run`), built from these counts. A provider that cannot answer for now
//! fails the call with [`MODEL_UNAVAILABLE`].

use std::time::Duration;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::catalog::{Entries, ErrorCodeDefinition, ErrorGroup, ProtocolVersion};
use crate::open_value::UnknownValue;

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

const ARRIVED_IN: ProtocolVersion = ProtocolVersion { major: 0, minor: 1 };

/// A Model provider that cannot answer for now, after the engine's retries
/// (ADR 0101).
pub const MODEL_UNAVAILABLE: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "model_unavailable",
    description: "The Model provider could not answer for now: it could not be reached, it failed with a 5xx, or it kept rate limiting the calls, after the engine's retries. The details name the `reason` and `retry_after`, how long the provider asked to wait before trying again, in milliseconds, or `null` when it asked for no wait. Retry later, or name another model.",
    group: ErrorGroup::Unavailable,
    retryable: true,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// What `model_unavailable` details (ADR 0101).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ModelUnavailableDetails {
    /// Why the provider could not answer.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub reason: ModelUnavailableReason,
    /// How long the provider asked to wait before the next call, from its
    /// answer's `Retry-After` or its own rate-limit reset: a duration, not a
    /// point in time, in whole milliseconds on the wire; `null` when it
    /// asked for no wait.
    #[serde(default, with = "nullable_millis")]
    #[schemars(with = "Option<u64>", extend("arrived_in" = "0.1"))]
    pub retry_after: Option<Duration>,
}

/// Why a Model provider could not answer (ADR 0101), open (ADR 0079).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[schemars(extend("open" = true), transform = crate::open_value::known_members)]
#[non_exhaustive]
pub enum ModelUnavailableReason {
    /// The provider could not be reached, or a call to the program's own
    /// provider was dropped unanswered.
    Unreachable,
    /// The provider failed with a 5xx.
    ServerError,
    /// The provider kept rate limiting the calls.
    RateLimited,
    /// Why a newer engine says that this reader does not know, kept as
    /// received.
    #[serde(untagged)]
    #[schemars(skip)]
    Unknown(UnknownValue),
}

/// A nullable duration in whole milliseconds, the wire's unit (ADR 0080):
/// `null` is `None`, and a field left out reads as none with `default`.
mod nullable_millis {
    use std::time::Duration;

    use serde::{Deserialize, Deserializer, Serializer};

    use crate::catalog::wire_millis;

    #[expect(
        clippy::ref_option,
        reason = "serde's `with` hands the field over by reference"
    )]
    pub(super) fn serialize<S: Serializer>(
        duration: &Option<Duration>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        match duration {
            Some(duration) => serializer.serialize_u64(wire_millis(*duration)),
            None => serializer.serialize_none(),
        }
    }

    pub(super) fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Option<Duration>, D::Error> {
        Ok(Option::<u64>::deserialize(deserializer)?.map(Duration::from_millis))
    }
}

/// Every entry this module defines.
pub const ENTRIES: Entries = Entries {
    defined_in: module_path!(),
    operations: &[],
    events: &[],
    handler_requests: &[],
    error_codes: &[MODEL_UNAVAILABLE],
    plugin_kinds: &[],
    metrics: &[],
    settings: &[],
    hook_stages: &[],
    shared_type_values: &[],
};

#[cfg(test)]
mod tests;
