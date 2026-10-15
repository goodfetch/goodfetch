//! Models, usage and cost in the SDK API (ADR 0101, decided in #475).
//!
//! - A program names a model by a [`ModelReference`], `"<provider>/<model id>"`: the setting
//!   [`crate::settings::ai::MODEL`] for a scope, or the AI verbs' per-call `.model(..)` (ADR 0091),
//!   which is that setting at the run layer. It has no default: no model is called unless one is
//!   configured (ADR 0043).
//! - Model providers are built in (`openai`, `anthropic`, `google`, `vercel`, `openrouter`, and the
//!   local `ollama`, `llamacpp`, `lmstudio`, `vllm`), declared with [`Settings::model_provider`], a
//!   Model provider Plugin the operator allowed (ADR 0033), named by its manifest, or declared with
//!   [`ConnectOptions::model_provider`] and
//!   answered by the program through [`Goodfetch::model_requests`] (ADR 0033's SDK-handler form, an
//!   [`EventStream`] as ADR 0092's `secret_requests`, since the async API spawns nothing, ADR 0081).
//! - Capabilities are declared, never sniffed from a name; prices come from the price table, set only
//!   in configuration.
//! - Every AI result carries its [`ModelUsage`], priced; [`crate::Run::usage`] gives the Run's
//!   totals, what remains of each Limit and ADR 0041's uncapped mark. An error from a call that made
//!   model calls carries its usage too ([`crate::Error::usage`]).

use std::fmt;
use std::time::{Duration, SystemTime};

use crate::concurrency::EventStream;
use crate::hooks::HandlerError;
use crate::lifecycle::{ConnectOptions, Goodfetch, Run};
use crate::sessions::SecretReference;
use crate::settings::Settings;
use crate::time::{Call, Money};

/// A Model reference: a Model provider's name, a slash, and that provider's own model ID verbatim,
/// so a gateway's ID keeps its slash: `"anthropic/claude-sonnet-4-5"`,
/// `"vercel/anthropic/claude-sonnet-4.5"`, `"ollama/qwen3:8b"`. Checked where the scope opens or the
/// call starts: a provider that is not built in, declared, an allowed Model provider Plugin or the
/// program's own fails with `setting_invalid`, naming the closest; the model ID is the provider's to
/// accept (`model_not_found`).
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
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum StructuredOutputStrategy {
    /// The provider's own schema mode.
    Native,
    /// A forced tool call whose input is the schema.
    Tool,
    /// The schema in the prompt, with JSON repair.
    Prompt,
    /// A strategy a newer engine reports, as received (G3).
    Unknown(String),
}

/// `ai.structured_output`.
#[derive(Debug, Clone, PartialEq, Eq)]
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
/// price table's, overridden only in configuration under `ai.providers.<name>.models.<id>.price`, so
/// it has no method here. A model ID holding a dot or a slash is one quoted segment of a key:
/// `ai.providers.lab.models."gpt-5.1".image_input`.
#[derive(Debug, Clone, Default)]
#[must_use]
pub struct ModelDeclaration {
    _private: (),
}

impl ModelDeclaration {
    /// Nothing declared: text only, structured output from the top of the ladder (`Prompt` for the
    /// program's own provider), no price.
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

    /// Whether the model takes images.
    pub fn image_input(self, takes: bool) -> Self {
        let _ = takes;
        unimplemented!()
    }

    /// Whether the model calls tools.
    pub fn tool_calls(self, calls: bool) -> Self {
        let _ = calls;
        unimplemented!()
    }

    /// The structured output strategies the model accepts, best first; `Unknown` is refused here
    /// with `setting_invalid` (G3).
    pub fn strategies(self, strategies: &[StructuredOutputStrategy]) -> Self {
        let _ = strategies;
        unimplemented!()
    }
}

/// `ai.providers.<name>`: a Model provider the program declares, or a built-in it changes, at the
/// connection, a Run or a Session; what is left unset keeps the built-in's. Its key and secret
/// headers go only to its base URL; a built-in's default key goes only to its own address, so a
/// built-in pointed elsewhere sends none. The key, the headers and the base URL are set by the
/// program's own code, configuration or an Operator policy: a Recipe and a Client acting for a model,
/// an Integration included, never set them (`setting_locked`). For the program's own provider, only
/// [`ModelProvider::model`] applies.
#[derive(Debug, Clone, Default)]
#[must_use]
pub struct ModelProvider {
    _private: (),
}

impl ModelProvider {
    /// Nothing set: a built-in's own values, or, for a new name, a declaration still missing its
    /// format and base URL (`setting_invalid` where its scope opens).
    pub fn new() -> Self {
        Self::default()
    }

    /// The request format.
    pub fn format(self, format: ModelFormat) -> Self {
        let _ = format;
        unimplemented!()
    }

    /// The base URL.
    pub fn base_url(self, url: &str) -> Self {
        let _ = url;
        unimplemented!()
    }

    /// The key, a Secret reference resolved by the engine (ADR 0092).
    pub fn api_key(self, key: SecretReference) -> Self {
        let _ = key;
        unimplemented!()
    }

    /// Sends no key, not even a built-in's default.
    pub fn no_api_key(self) -> Self {
        unimplemented!()
    }

    /// A header sent with every request; its value is a Secret reference, as ADR 0056's
    /// `cdp_url` headers.
    pub fn header(self, name: &str, value: SecretReference) -> Self {
        let _ = (name, value);
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

/// Whether the answer must call a tool.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ModelToolChoice {
    /// The model chooses.
    Auto,
    /// No tool.
    None,
    /// This tool: the forced tool call of the `Tool` strategy.
    Tool(String),
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
    pub model_id: String,
    /// The conversation.
    pub messages: Vec<ModelMessage>,
    /// The tools offered.
    pub tools: Vec<ModelTool>,
    /// Whether the answer must call one.
    pub tool_choice: ModelToolChoice,
    /// Set when the strategy is native: the answer must be JSON fitting it.
    pub response_schema: Option<serde_json::Value>,
    /// The most output tokens the answer may hold.
    pub max_output_tokens: u64,
}

/// The token counts the program reports: input and output are required, so a Limit counts every call.
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

    /// The model declined to answer, which the engine raises as `model_declined`.
    pub fn refusal(reason: String, usage: ReportedTokens) -> Self {
        let _ = (reason, usage);
        unimplemented!()
    }
}

/// One model call the engine asks the program to answer. The engine journals, counts and caps it
/// (ADR 0012) and bounds it with `timeout.model`; dropped unanswered, it fails as the provider being
/// unreachable. The provider gets the `Prompt` strategy unless its models declare others, and only
/// configuration prices it.
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

    /// Resolves when the engine gives up on the call (`timeout.model` passed, or the call was
    /// cancelled), so the program can stop working on it, as with `tokio::select!`.
    pub async fn cancelled(&self) {
        unimplemented!()
    }

    /// Sends the answer to the engine.
    pub fn answer(self, response: ModelResponse) {
        let _ = response;
        unimplemented!()
    }

    /// Fails the call: a Goodfetch [`Error`](crate::Error) keeps its code, such as `model_unavailable`, which the
    /// engine retries as a provider's; any other error is `model_handler_failed`, with its type and
    /// message.
    pub fn fail(self, error: HandlerError) {
        let _ = error;
        unimplemented!()
    }
}

impl ConnectOptions {
    /// Declares the program's own Model provider `name`, which Model references then reach as
    /// `"<name>/<any model>"` from the Runs this connection opens, and whose calls
    /// [`Goodfetch::model_requests`] reads. A name taken by a built-in, a declared provider or a
    /// Plugin fails with `setting_invalid` when connecting.
    pub fn model_provider(self, name: &str) -> Self {
        let _ = name;
        unimplemented!()
    }
}

impl Goodfetch {
    /// The model calls for the declared provider `name`, subscribed when called; a name not declared
    /// with [`ConnectOptions::model_provider`], or a second stream while one is open, fails with
    /// `setting_invalid`. A call waits for its stream until its own deadline.
    pub fn model_requests(&self, name: &str) -> Result<EventStream<ModelCall>, crate::Error> {
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
/// calls. A step replayed from the Plan sends no call and is priced zero.
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
    /// currency (USD as shipped), other currencies converted only through the table's fixed rates.
    /// `None` when part of it has no price, or needs a rate the table lacks: never a partial sum.
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

/// A Limit's period, for one that counts per day, week or month (ADR 0041).
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct LimitPeriod {
    /// `day`, `week` or `month`.
    pub unit: String,
    /// An IANA time zone, such as `Europe/Paris`.
    pub time_zone: String,
    /// When the current period ends and the Limit's Usage starts again (ADR 0102).
    pub resets_at: SystemTime,
}

/// Details of `limit_reached`: the Limit, its scope and the scope's ID (ADR 0041), and when its
/// period resets (ADR 0102).
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct LimitReached {
    /// The Limit, by name.
    pub limit: String,
    /// Where it attaches.
    pub scope: LimitScope,
    /// The scope's ID.
    pub scope_id: String,
    /// `None` for a Limit with no period.
    pub resets_at: Option<SystemTime>,
}

/// A Limit that applies to the Run, and what remains of it.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct AppliedLimit {
    /// Where it attaches.
    pub scope: LimitScope,
    /// The scope's ID (ADR 0102): the Run's ID, the Schedule's, the named token's name, the person,
    /// the Daemon or the install.
    pub scope_id: String,
    /// `None` for a Limit with no period.
    pub period: Option<LimitPeriod>,
    /// What it caps.
    pub amount: LimitAmount,
    /// Who set it.
    pub set_by: LimitSetBy,
}

/// A Run's Usage so far: the figures the Usage ledger charges. Its Run report carries the final
/// one (ADR 0102).
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
    /// The price of the Usage third parties charge for: model calls, metered proxies, paid solves and
    /// Paid fetches; `None` as [`ModelUsage::price`] when any of it has no price or rate. Browser
    /// time and an unmetered proxy carry none.
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

/// What the price table lacks for `model_price_unknown`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum PriceMissing {
    /// The model's price.
    Price,
    /// A fixed rate from the model's currency to the Limit's.
    Rate,
    /// A value this SDK does not know, as received.
    Unknown(String),
}

/// Details of `model_price_unknown`: `fix` names the configuration key.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct ModelPriceUnknown {
    /// The Model reference.
    pub model: ModelReference,
    /// What the price table lacks.
    pub missing: PriceMissing,
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

/// Details of `model_declined`.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct ModelDeclined {
    /// The failure.
    pub failure: ModelFailure,
    /// The provider's reason, when it gave one.
    pub reason: Option<String>,
}

/// Details of `model_context_exceeded`.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct ModelContextExceeded {
    /// The failure.
    pub failure: ModelFailure,
    /// The model's context window, when known.
    pub context_window: Option<u64>,
}

/// Details of `model_request_rejected`: a request the provider rejected for another reason (a
/// 400 or 422), such as a strategy `ai.structured_output` forces.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct ModelRequestRejected {
    /// The failure.
    pub failure: ModelFailure,
    /// The provider's message, when it sent one.
    pub provider_message: Option<String>,
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
