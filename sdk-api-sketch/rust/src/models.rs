//! Models, usage and cost in the SDK API (ADR 0101, decided in #475).
//!
//! - A program names a model by a [`ModelReference`], `"<provider>/<model id>"`: the setting
//!   [`crate::settings::ai::MODEL`] for a scope, or the AI verbs' per-call `.model(..)` (ADR 0091),
//!   which is that setting at the run layer. It has no default: no model is called unless one is
//!   configured (ADR 0043).
//! - Model providers are built in (`openai`, `anthropic`, `google`, `vercel`, `openrouter`, and the
//!   local `ollama`, `llamacpp`, `lmstudio`, `vllm`), declared with [`Settings::model_provider`], or
//!   answered by the program through [`Goodfetch::model_requests`] (ADR 0033's SDK-handler form, an
//!   [`EventStream`] as ADR 0092's `secret_requests`, since the async API spawns nothing, ADR 0081).
//! - Capabilities are declared, never sniffed from a name; prices come from the price table, set only
//!   in configuration.
//! - Every AI result carries its [`ModelUsage`], priced; [`crate::Run::usage`] gives the Run's
//!   totals, what remains of each Limit and ADR 0041's uncapped mark. An error from a call that made
//!   model calls carries its usage too ([`crate::Error::usage`]).

use std::fmt;
use std::time::Duration;

use crate::concurrency::EventStream;
use crate::lifecycle::{Goodfetch, Run};
use crate::page::Headers;
use crate::results::Error;
use crate::sessions::SecretReference;
use crate::settings::Settings;
use crate::time::{Call, Money};

/// A Model reference: a Model provider's name, a slash, and that provider's own model ID verbatim,
/// so a gateway's ID keeps its slash: `"anthropic/claude-sonnet-4-5"`,
/// `"vercel/anthropic/claude-sonnet-4.5"`, `"ollama/qwen3:8b"`. Checked where the scope opens or the
/// call starts: a provider neither built in nor declared fails with `setting_invalid`, naming the
/// closest; the model ID is the provider's to accept (`model_not_found`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ModelReference(String);

impl ModelReference {
    /// The reference as written.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for ModelReference {
    fn from(reference: &str) -> Self {
        Self(reference.to_owned())
    }
}

impl From<String> for ModelReference {
    fn from(reference: String) -> Self {
        Self(reference)
    }
}

impl fmt::Display for ModelReference {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// ADR 0012's three request formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ModelFormat {
    /// OpenAI-compatible chat completions.
    OpenAi,
    /// Anthropic Messages.
    Anthropic,
    /// Gemini's native format.
    Gemini,
}

/// How the engine gets JSON that fits a schema.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum StructuredOutputStrategy {
    /// The provider's own schema mode.
    Native,
    /// A forced tool call whose input is the schema.
    Tool,
    /// The schema in the prompt, with JSON repair.
    Prompt,
}

/// `ai.structured_output`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum StructuredOutput {
    /// The built-in: the best strategy the model declares, or `Native` for one it does not know,
    /// stepping down when the provider rejects a strategy or a schema feature. An answer that fails
    /// validation is retried, never stepped down.
    Auto,
    /// One strategy only.
    Only(StructuredOutputStrategy),
}

/// What a model can take and produce, declared rather than inferred from its name. Its price is the
/// price table's, set only in configuration, so it has no method here.
#[derive(Debug, Clone, Default)]
#[must_use]
pub struct ModelDeclaration {
    _private: (),
}

impl ModelDeclaration {
    /// Nothing declared: text only, structured output from the top of the ladder, no price.
    pub fn new() -> Self {
        Self::default()
    }

    /// The context window, in tokens.
    pub fn context_window(self, tokens: u64) -> Self {
        let _ = tokens;
        unimplemented!()
    }

    /// The most output tokens one answer may hold.
    pub fn max_output_tokens(self, tokens: u64) -> Self {
        let _ = tokens;
        unimplemented!()
    }

    /// The model takes images.
    pub fn image_input(self) -> Self {
        unimplemented!()
    }

    /// The model calls tools.
    pub fn tool_calls(self) -> Self {
        unimplemented!()
    }

    /// The structured output strategies the model accepts, best first.
    pub fn strategies(self, strategies: &[StructuredOutputStrategy]) -> Self {
        let _ = strategies;
        unimplemented!()
    }
}

/// `ai.providers.<name>`: a Model provider the program declares, or a built-in it changes. Its key
/// and secret headers go only to its base URL; a built-in's default key goes only to its own
/// address, so a built-in pointed elsewhere sends none. A Client acting for a model never sets the
/// key, the headers or the base URL (`setting_locked`).
#[derive(Debug, Clone)]
#[must_use]
pub struct ModelProvider {
    _private: (),
}

impl ModelProvider {
    /// A provider speaking `format` at `base_url`.
    pub fn new(format: ModelFormat, base_url: &str) -> Self {
        let _ = (format, base_url);
        unimplemented!()
    }

    /// The key, a Secret reference resolved by the engine (ADR 0092).
    pub fn api_key(self, key: SecretReference) -> Self {
        let _ = key;
        unimplemented!()
    }

    /// Headers sent with every request.
    pub fn headers(self, headers: Headers) -> Self {
        let _ = headers;
        unimplemented!()
    }

    /// Capabilities of one model, by the provider's own model ID.
    pub fn model(self, id: &str, declaration: ModelDeclaration) -> Self {
        let _ = (id, declaration);
        unimplemented!()
    }
}

impl Settings {
    /// Declares the Model provider `name` (`ai.providers.<name>`) for this scope.
    pub fn model_provider(self, name: &str, provider: ModelProvider) -> Self {
        let _ = (name, provider);
        unimplemented!()
    }
}

// The program's own Model provider.

/// A message of a [`ModelRequest`].
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum ModelMessage {
    /// Instructions.
    System(String),
    /// Text and images from the engine.
    User(Vec<ModelContentPart>),
    /// An earlier answer.
    Assistant {
        /// Its parts.
        content: Vec<ModelContentPart>,
        /// Its tool calls.
        tool_calls: Vec<ModelToolCall>,
    },
    /// A tool's result.
    Tool {
        /// The call it answers.
        tool_call_id: String,
        /// The result.
        content: String,
    },
}

/// A part of a message.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum ModelContentPart {
    /// Text.
    Text(String),
    /// An image.
    Image {
        /// Its media type, such as `image/png`.
        media_type: String,
        /// Its bytes.
        data: Vec<u8>,
    },
}

/// A tool the model may call.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct ModelTool {
    /// Its name.
    pub name: String,
    /// What it does.
    pub description: String,
    /// Its input's JSON Schema.
    pub input_schema: serde_json::Value,
}

/// A tool call the model made.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct ModelToolCall {
    /// Its ID.
    pub id: String,
    /// The tool's name.
    pub name: String,
    /// The arguments as JSON text, as the model wrote them: the engine parses and validates them.
    pub arguments: String,
}

/// What the engine asks the program's Model provider: Goodfetch's own types (G7).
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct ModelRequest {
    /// The model ID after the provider's name in the Model reference.
    pub model: String,
    /// The conversation.
    pub messages: Vec<ModelMessage>,
    /// The tools offered.
    pub tools: Vec<ModelTool>,
    /// Set when the strategy is native: the answer must be JSON fitting it.
    pub response_schema: Option<serde_json::Value>,
    /// The most output tokens the answer may hold.
    pub max_output_tokens: u64,
}

/// The token counts a handler reports: input and output are required.
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct ReportedTokens {
    /// Every input token, cached ones included.
    pub input_tokens: u64,
    /// Every output token, reasoning included.
    pub output_tokens: u64,
    /// Part of `input_tokens`.
    pub cache_read_tokens: u64,
    /// Part of `input_tokens`.
    pub cache_write_tokens: u64,
    /// Part of `output_tokens`.
    pub reasoning_tokens: u64,
}

impl ReportedTokens {
    /// Input and output tokens; the parts default to zero.
    pub fn new(input_tokens: u64, output_tokens: u64) -> Self {
        Self {
            input_tokens,
            output_tokens,
            ..Self::default()
        }
    }
}

/// The program's answer.
#[derive(Debug, Clone)]
#[must_use]
pub struct ModelResponse {
    _private: (),
}

impl ModelResponse {
    /// A text answer that stopped on its own.
    pub fn text(text: String, usage: ReportedTokens) -> Self {
        let _ = (text, usage);
        unimplemented!()
    }

    /// Adds the tool calls the model made.
    pub fn tool_calls(self, calls: Vec<ModelToolCall>) -> Self {
        let _ = calls;
        unimplemented!()
    }

    /// The answer was cut at the output limit.
    pub fn truncated(self) -> Self {
        unimplemented!()
    }
}

/// One model call the engine asks the program to answer. The engine journals, counts and caps it
/// (ADR 0012) and bounds it with `timeout.model`; dropped unanswered, it fails as the provider being
/// unreachable.
#[derive(Debug)]
pub struct ModelCall {
    _private: (),
}

impl ModelCall {
    /// What the engine asks.
    pub fn request(&self) -> &ModelRequest {
        unimplemented!()
    }

    /// The Run asking.
    pub fn run_id(&self) -> &str {
        unimplemented!()
    }

    /// Sends the answer to the engine.
    pub fn answer(self, response: ModelResponse) {
        let _ = response;
        unimplemented!()
    }

    /// Fails the call: a Goodfetch [`Error`] keeps its code, such as `model_unavailable`, which the
    /// engine retries as a provider's.
    pub fn fail(self, error: Error) {
        let _ = error;
        unimplemented!()
    }
}

impl Goodfetch {
    /// The model calls for the provider `name`, which Model references then reach as
    /// `"<name>/<any model>"` while the stream is open: subscribed when called. A reference to a
    /// name with no stream open fails with `setting_invalid` where its scope opens.
    pub fn model_requests(&self, name: &str) -> EventStream<ModelCall> {
        let _ = name;
        unimplemented!()
    }
}

// Usage and its price.

/// Token counts of one model, in OpenTelemetry's and the AI SDK's convention whatever the
/// provider's: input counts every input token, cached ones included, and output counts reasoning.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct ModelUsageOf {
    /// The Model reference.
    pub model: ModelReference,
    /// Model calls sent, retries and Repairs included.
    pub calls: u64,
    /// Input plus output: what [`crate::Limits::model_tokens`] counts.
    pub tokens: u64,
    /// Every input token.
    pub input_tokens: u64,
    /// Part of `input_tokens`.
    pub cache_read_tokens: u64,
    /// Part of `input_tokens`.
    pub cache_write_tokens: u64,
    /// Every output token.
    pub output_tokens: u64,
    /// Part of `output_tokens`.
    pub reasoning_tokens: u64,
    /// Their price from the price table, as [`ModelUsage::price`].
    pub price: Option<Money>,
}

/// What model calls consumed: on every AI result, and on every error from a call that made model
/// calls. A step replayed from the Plan sends no call and costs nothing.
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct ModelUsage {
    /// Model calls sent, retries and Repairs included.
    pub calls: u64,
    /// Input plus output: what [`crate::Limits::model_tokens`] counts.
    pub tokens: u64,
    /// Every input token, cached ones included.
    pub input_tokens: u64,
    /// Part of `input_tokens`.
    pub cache_read_tokens: u64,
    /// Part of `input_tokens`.
    pub cache_write_tokens: u64,
    /// Every output token, reasoning included.
    pub output_tokens: u64,
    /// Part of `output_tokens`.
    pub reasoning_tokens: u64,
    /// Their price from the price table: a list-price estimate, never a bill, in the table's
    /// currency (USD as shipped). `None` when part of it has no price, or needs a rate between
    /// currencies the table lacks: never a partial sum.
    pub price: Option<Money>,
    /// The same per Model reference, in first-use order.
    pub by_model: Vec<ModelUsageOf>,
}

/// Where a Limit attaches (ADR 0041). A newer engine's value is kept as received (G3).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum LimitScope {
    /// The Run.
    Run,
    /// The Client connection.
    Connection,
    /// A named token.
    NamedToken,
    /// A person on a shared Daemon.
    Person,
    /// The Daemon or Daemon group.
    Daemon,
    /// Every run of the user on the machine.
    Install,
    /// A Schedule's period.
    Schedule,
    /// A Partitioned crawl.
    PartitionedCrawl,
    /// A scope this SDK does not know, as received.
    Unknown(String),
}

/// What a Limit caps and how much of it, with what remains.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum LimitAmount {
    /// Model tokens.
    ModelTokens {
        /// The Limit.
        value: u64,
        /// What remains.
        remaining: u64,
    },
    /// Model calls.
    ModelCalls {
        /// The Limit.
        value: u64,
        /// What remains.
        remaining: u64,
    },
    /// Browser time of Usage.
    BrowserTime {
        /// The Limit.
        value: Duration,
        /// What remains.
        remaining: Duration,
    },
    /// Proxy bytes.
    ProxyBytes {
        /// The Limit.
        value: u64,
        /// What remains.
        remaining: u64,
    },
    /// Paid solves.
    PaidSolves {
        /// The Limit.
        value: u64,
        /// What remains.
        remaining: u64,
    },
    /// Paid fetches.
    PaidFetches {
        /// The Limit.
        value: u64,
        /// What remains.
        remaining: u64,
    },
    /// A price.
    Price {
        /// The Limit.
        value: Money,
        /// What remains.
        remaining: Money,
    },
    /// A kind this SDK does not know, as received.
    Unknown(String),
}

/// Who set a Limit (ADRs 0041, 0045): only a person's or a managed policy's allows Paid fetches.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum LimitSetBy {
    /// A person, in the Console or an interactive CLI prompt.
    Person,
    /// A managed Operator policy.
    Policy,
    /// A Client, for its own work.
    Client,
    /// A value this SDK does not know, as received.
    Unknown(String),
}

/// A Limit that applies to the Run, and what remains of it.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct AppliedLimit {
    /// Where it attaches.
    pub scope: LimitScope,
    /// What it caps.
    pub amount: LimitAmount,
    /// Who set it.
    pub set_by: LimitSetBy,
}

/// A Run's Usage so far: the figures the Usage ledger charges. Its report carries the final one
/// (#478).
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct RunUsage {
    /// What model calls consumed.
    pub model: ModelUsage,
    /// Browser time.
    pub browser_time: Duration,
    /// Proxy bytes.
    pub proxy_bytes: u64,
    /// Paid solves.
    pub paid_solves: u64,
    /// Paid fetches.
    pub paid_fetches: u64,
    /// The price of all priced Usage; `None` as [`ModelUsage::price`].
    pub price: Option<Money>,
    /// Each Limit that applies to the Run.
    pub limits: Vec<AppliedLimit>,
    /// ADR 0041's mark: the Run can spend money with nothing capping it.
    pub uncapped: bool,
}

impl Run {
    /// The Run's Usage so far, priced, with what remains of each Limit and the uncapped mark.
    pub fn usage(&self) -> Call<RunUsage> {
        unimplemented!()
    }
}

// Details of the Model provider Error codes (ADR 0079's groups). `model_not_configured` is ADR
// 0091's.

/// What every model failure names.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct ModelFailure {
    /// The Model reference.
    pub model: ModelReference,
    /// The provider's HTTP status, when one answered.
    pub status: Option<u16>,
    /// The provider's request ID, when it sent one.
    pub request_id: Option<String>,
}

/// Details of `model_capability_missing`: `fix` names the key that declares it.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct ModelCapabilityMissing {
    /// The Model reference.
    pub model: ModelReference,
    /// The capability, such as `image_input`.
    pub capability: String,
}

/// Details of `model_handler_failed`.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct ModelHandlerFailed {
    /// The provider's name.
    pub provider: String,
    /// The error's type name.
    pub error_type: String,
    /// Its message.
    pub error_message: String,
}

/// Why a provider was unavailable.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum UnavailableReason {
    /// Unreachable.
    Unreachable,
    /// A 5xx.
    ServerError,
    /// Rate limited after the engine's retries.
    RateLimited,
    /// A reason this SDK does not know, as received.
    Unknown(String),
}

/// Details of `model_unavailable`.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct ModelUnavailable {
    /// The failure.
    pub failure: ModelFailure,
    /// Why.
    pub reason: UnavailableReason,
    /// How long the provider asked to wait, when it said.
    pub retry_after: Option<Duration>,
}

/// Details of `model_output_invalid`.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct ModelOutputInvalid {
    /// The Model reference.
    pub model: ModelReference,
    /// The strategy of the last attempt.
    pub strategy: StructuredOutputStrategy,
    /// How many answers the model gave.
    pub attempts: u32,
    /// The last attempt's problem.
    pub problem: String,
}
