"""Models, usage and cost in the SDK API (ADR 0101, decided in #475).

- A program names a model by a Model reference, `"<provider>/<model id>"`: the setting `ai.model` for a scope, or the
  AI verbs' per-call `model=` (ADR 0091), which is that setting at the run layer. `ai.model` has no default: no model
  is called unless one is configured (ADR 0043).
- Model providers are built in (`openai`, `anthropic`, `google`, `vercel`, `openrouter`, and the local `ollama`,
  `llamacpp`, `lmstudio`, `vllm`), declared under `ai.providers.<name>`, or a handler registered on the connection
  (ADR 0033's SDK-handler form of the Model provider kind).
- Capabilities are declared, never sniffed from a name; prices come from the price table, set only in configuration.
- Every AI result carries its `ModelUsage`, priced; `run.usage()` gives the Run's totals, what remains of each Limit and
  ADR 0041's uncapped mark. An error from a call that made model calls carries its `usage` too.
"""

from collections.abc import Awaitable, Callable, Mapping, Sequence
from dataclasses import dataclass
from datetime import timedelta
from typing import Literal, TypeAlias, TypedDict, final

from goodfetch._data import JsonSchema
from goodfetch._results import ModelError, ProgramError, RefusedError, UnavailableError
from goodfetch._sessions import HeadersInput, SecretReference
from goodfetch._time import Money

ModelReference: TypeAlias = str
"""A Model reference: a Model provider's name, a slash, and that provider's own model ID verbatim, so a gateway's ID
keeps its slash: `"anthropic/claude-sonnet-4-5"`, `"vercel/anthropic/claude-sonnet-4.5"`, `"ollama/qwen3:8b"`. The
provider must be built in or declared where the scope opens (`SettingInvalidError`, naming the closest); the model ID is
the provider's to accept (`ModelNotFoundError`)."""

BuiltinModelProvider: TypeAlias = Literal[
    "openai", "anthropic", "google", "vercel", "openrouter", "ollama", "llamacpp", "lmstudio", "vllm"
]
"""The built-in Model providers. A local one sits at its server's default loopback address."""

ModelFormat: TypeAlias = Literal["openai", "anthropic", "gemini"]
"""ADR 0012's three request formats."""

StructuredOutputStrategy: TypeAlias = Literal["native", "tool", "prompt"]
"""How the engine gets JSON that fits a schema: the provider's schema mode, a forced tool call, or the schema in the
prompt with repair."""

class ModelDeclaration(TypedDict, total=False):
    """What a model can take and produce, declared rather than inferred from its name.

    The shipped list covers the tested models (ADR 0065); a model it does not know is text only, gets structured output
    from the top of the ladder, and has no price. Its price is the price table's, set only in configuration, so it is no
    key here: a price set in code raises `SettingLockedError`."""

    context_window: int
    max_output_tokens: int
    image_input: bool
    tool_calls: bool
    strategies: list[StructuredOutputStrategy]
    """The structured output strategies the model accepts, best first."""

class ModelProviderSettings(TypedDict, total=False):
    """`ai.providers.<name>`: a Model provider the program declares, or a built-in it changes.

    `api_key` and secret `headers` go only to `base_url`. A built-in's default key (its conventional variable, such as
    `env:OPENAI_API_KEY`) goes only to its own address, so a built-in pointed elsewhere sends none. A Client acting for a
    model never sets `api_key`, `headers` or `base_url` (`SettingLockedError`)."""

    format: ModelFormat
    base_url: str
    api_key: SecretReference | None
    headers: HeadersInput
    models: Mapping[str, ModelDeclaration]
    """Capabilities by the provider's own model ID."""

class AiSettings(TypedDict, total=False):
    """`ai.*` (ADR 0101)."""

    model: ModelReference | None
    """`ai.model`: the Model reference every AI call of the scope uses unless the call names its own. No default."""
    providers: Mapping[str, ModelProviderSettings]
    structured_output: Literal["auto"] | StructuredOutputStrategy
    """`auto` (the default) starts at the best strategy the model declares, or `native` for one it does not know, and
    steps down when the provider rejects a strategy or a schema feature; an answer that fails validation is retried,
    never stepped down. Every answer is validated against the program's own schema and validators (ADR 0082)."""

class PageAiSettings(TypedDict, total=False):
    """The model and the strategy for a page's AI calls; Model providers are declared above a page."""

    model: ModelReference | None
    structured_output: Literal["auto"] | StructuredOutputStrategy

# The handler: ADR 0033's SDK-handler form of the Model provider kind.

@final
@dataclass(frozen=True, kw_only=True)
class ModelToolCall:
    id: str
    name: str
    arguments: str
    """The arguments as JSON text, as the model wrote them: the engine parses and validates them."""

@final
@dataclass(frozen=True, kw_only=True)
class ModelImage:
    media_type: str
    data: bytes

@final
@dataclass(frozen=True, kw_only=True)
class ModelMessage:
    role: Literal["system", "user", "assistant", "tool"]
    content: Sequence[str | ModelImage]
    tool_calls: Sequence[ModelToolCall] = ()
    tool_call_id: str | None = None

@final
@dataclass(frozen=True, kw_only=True)
class ModelTool:
    name: str
    description: str
    input_schema: JsonSchema

@final
@dataclass(frozen=True, kw_only=True)
class ModelRequest:
    """What the engine asks a handler: Goodfetch's own types, so no provider SDK is needed (G7)."""

    model: str
    """The model ID after the provider's name in the Model reference."""
    messages: Sequence[ModelMessage]
    tools: Sequence[ModelTool]
    response_schema: JsonSchema | None
    """Set when the strategy is `native`: the answer must be JSON fitting it."""
    max_output_tokens: int

@final
@dataclass(frozen=True, kw_only=True)
class ReportedTokens:
    """The token counts a handler reports: input and output are required, as Stagehand requires of custom models."""

    input_tokens: int
    output_tokens: int
    cache_read_tokens: int = 0
    cache_write_tokens: int = 0
    reasoning_tokens: int = 0

@final
@dataclass(frozen=True, kw_only=True)
class ModelResponse:
    text: str
    finish_reason: Literal["stop", "length", "tool_calls", "refusal"] | str
    usage: ReportedTokens
    tool_calls: Sequence[ModelToolCall] = ()

AsyncModelProviderHandler: TypeAlias = Callable[[ModelRequest], Awaitable[ModelResponse]]
"""Answers model calls on the program's own event loop (ADR 0081); its task is cancelled when `timeout.model` passes or
the call is cancelled. A `GoodfetchError` it raises keeps its code (a `ModelUnavailableError` is retried as a
provider's would be); any other exception is `ModelHandlerFailedError`."""

ModelProviderHandler: TypeAlias = Callable[[ModelRequest], ModelResponse]
"""The sync client's form, run where the program's own code runs (ADR 0081)."""

# Usage and its price.

@dataclass(frozen=True, kw_only=True)
class TokenCounts:
    """Token counts in OpenTelemetry's and the AI SDK's convention, whatever the provider's: input counts every input
    token, cached ones included, and output counts reasoning tokens."""

    calls: int
    """Model calls sent, retries and Repairs included; a step replayed from the Plan sends none."""
    tokens: int
    """Input plus output: what `Limits.model_tokens` counts."""
    input_tokens: int
    cache_read_tokens: int
    """Part of `input_tokens`."""
    cache_write_tokens: int
    """Part of `input_tokens`."""
    output_tokens: int
    reasoning_tokens: int
    """Part of `output_tokens`."""
    price: Money | None
    """Their price from the price table: a list-price estimate, never a bill, in the table's currency (USD as shipped).
    `None` when part of it has no price, or needs a rate between currencies the table lacks: never a partial sum."""

@final
@dataclass(frozen=True, kw_only=True)
class ModelUsageOf(TokenCounts):
    model: ModelReference

@final
@dataclass(frozen=True, kw_only=True)
class ModelUsage(TokenCounts):
    """What model calls consumed: on every AI result, and on every error from a call that made model calls."""

    by_model: Sequence[ModelUsageOf]
    """The same per Model reference, in first-use order."""

LimitScope: TypeAlias = (
    Literal["run", "connection", "named_token", "person", "daemon", "install", "schedule", "partitioned_crawl"] | str
)
"""Where a Limit attaches (ADR 0041). A newer engine's value is kept as received (G3)."""

LimitKind: TypeAlias = (
    Literal["model_tokens", "model_calls", "browser_time", "proxy_bytes", "paid_solves", "paid_fetches", "price"] | str
)

@final
@dataclass(frozen=True, kw_only=True)
class AppliedLimit:
    """A Limit that applies to the Run, and what remains of it."""

    scope: LimitScope
    kind: LimitKind
    value: int | timedelta | Money
    """A count, a `timedelta` for `browser_time`, or `Money` for `price`."""
    remaining: int | timedelta | Money
    set_by: Literal["person", "policy", "client"] | str
    """Who set it (ADRs 0041, 0045): only a person's or a managed policy's Limit allows Paid fetches."""

@final
@dataclass(frozen=True, kw_only=True)
class RunUsage:
    """A Run's Usage so far: the figures the Usage ledger charges. Its report carries the final one (#478)."""

    model: ModelUsage
    browser_time: timedelta
    proxy_bytes: int
    paid_solves: int
    paid_fetches: int
    price: Money | None
    """The price of all priced Usage; `None` as `TokenCounts.price`."""
    limits: Sequence[AppliedLimit]
    uncapped: bool
    """ADR 0041's mark: the Run can spend money with nothing capping it."""

# The Model provider Error codes (ADR 0079's groups). `model_not_configured` is ADR 0091's (`_ai`).

@final
@dataclass(frozen=True, kw_only=True)
class ModelFailureDetails:
    model: ModelReference
    status: int | None
    """The provider's HTTP status, when one answered."""
    request_id: str | None
    """The provider's request ID, when it sent one."""

@final
@dataclass(frozen=True, kw_only=True)
class ModelCapabilityMissingDetails:
    model: ModelReference
    capability: Literal["image_input", "tool_calls"] | str

@final
@dataclass(frozen=True, kw_only=True)
class ModelHandlerFailedDetails:
    provider: str
    exception_type: str
    exception_message: str

@final
@dataclass(frozen=True, kw_only=True)
class ModelPriceUnknownDetails:
    model: ModelReference

@final
@dataclass(frozen=True, kw_only=True)
class ModelUnavailableDetails:
    model: ModelReference
    status: int | None
    request_id: str | None
    reason: Literal["unreachable", "server_error", "rate_limited"] | str
    retry_after: float | None
    """Seconds the provider asked to wait, when it said."""

@final
@dataclass(frozen=True, kw_only=True)
class ModelDeclinedDetails:
    model: ModelReference
    status: int | None
    request_id: str | None
    reason: str | None

@final
@dataclass(frozen=True, kw_only=True)
class ModelContextExceededDetails:
    model: ModelReference
    status: int | None
    request_id: str | None
    context_window: int | None

@final
@dataclass(frozen=True, kw_only=True)
class ModelOutputInvalidDetails:
    model: ModelReference
    strategy: StructuredOutputStrategy
    attempts: int
    problem: str
    """The last attempt's problem."""

@final
class ModelNotFoundError(ProgramError):
    """The provider says the model does not exist or is not available to this key."""

    @property
    def details(self) -> ModelFailureDetails: ...

@final
class ModelCapabilityMissingError(ProgramError):
    """The operation needs a capability the model does not declare; `fix` names the key that declares it."""

    @property
    def details(self) -> ModelCapabilityMissingDetails: ...

@final
class ModelHandlerFailedError(ProgramError):
    """The handler raised an exception that is not a `GoodfetchError`; it is the `__cause__`."""

    @property
    def details(self) -> ModelHandlerFailedDetails: ...

@final
class ModelUnauthorizedError(RefusedError):
    """The provider rejected the key. Not retryable: fix the provider's `api_key`."""

    @property
    def details(self) -> ModelFailureDetails: ...

@final
class ModelQuotaExhaustedError(RefusedError):
    """The provider refused for credit, quota or billing, such as an exhausted free credit. Not retryable."""

    @property
    def details(self) -> ModelFailureDetails: ...

@final
class ModelPriceUnknownError(RefusedError):
    """A currency Limit applies and the price table has no price for the model; `fix` names the configuration key."""

    @property
    def details(self) -> ModelPriceUnknownDetails: ...

@final
class ModelUnavailableError(UnavailableError):
    """Unreachable, a 5xx, or rate limited after the engine's retries. Retryable."""

    @property
    def details(self) -> ModelUnavailableDetails: ...

@final
class ModelDeclinedError(ModelError):
    """The model declined to answer: a refusal or a content filter."""

    @property
    def details(self) -> ModelDeclinedDetails: ...

@final
class ModelContextExceededError(ModelError):
    """The request does not fit the model's context window: narrow the scope, or choose a larger model."""

    @property
    def details(self) -> ModelContextExceededDetails: ...

@final
class ModelOutputInvalidError(ModelError):
    """An `act`, `observe` or Agent step's answer never parsed within ADR 0012's retries."""

    @property
    def details(self) -> ModelOutputInvalidDetails: ...
