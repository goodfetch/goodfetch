"""Models, usage and cost in the SDK API (ADR 0101, decided in #475).

- A program names a model by a Model reference, `"<provider>/<model id>"`: the setting `ai.model` for a scope, or the
  AI verbs' per-call `model=` (ADR 0091), which is that setting at the run layer. `ai.model` has no default: no model
  is called unless one is configured (ADR 0043).
- Model providers are built in (`openai`, `anthropic`, `google`, `vercel`, `openrouter`, and the local `ollama`,
  `llamacpp`, `lmstudio`, `vllm`), declared under `ai.providers.<name>`, a Model provider Plugin the operator allowed
  (ADR 0033), named by its manifest, or a handler registered on the connection (ADR 0033's SDK-handler form).
  Provider names form one namespace.
- Capabilities are declared, never sniffed from a name; prices come from the price table, set only in configuration.
- Every AI result carries its `ModelUsage`, priced; `run.usage()` gives the Run's totals, what remains of each Limit and
  ADR 0041's uncapped mark. An error from a call that made model calls carries its `usage` too.
"""

from collections.abc import Awaitable, Callable, Mapping, Sequence
from dataclasses import dataclass
from datetime import datetime, timedelta
from typing import Literal, TypeAlias, TypedDict, final

from goodfetch._data import JsonSchema
from goodfetch._results import LimitSource, ModelError, ProgramError, RefusedError, UnavailableError
from goodfetch._sessions import SecretReference
from goodfetch._time import Money

ModelReference: TypeAlias = str
"""A Model reference: a Model provider's name, a slash, and that provider's own model ID verbatim, so a gateway's ID
keeps its slash: `"anthropic/claude-sonnet-4-5"`, `"vercel/anthropic/claude-sonnet-4.5"`, `"ollama/qwen3:8b"`. The
provider must be built in, declared, an allowed Model provider Plugin or the program's own where the scope opens
(`SettingInvalidError`, naming the closest); the model ID is the provider's to accept (`ModelNotFoundError`)."""

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
    from the top of the ladder (`prompt` for the program's own provider), and has no price. Its price is the price
    table's, overridden only in configuration under `ai.providers.<name>.models.<id>.price`, so it is no key here: a
    price set in code raises `SettingLockedError`. A model ID holding a dot or a slash is one quoted segment of a key:
    `ai.providers.lab.models."gpt-5.1".image_input`."""

    context_window: int
    max_output_tokens: int
    image_input: bool
    tool_calls: bool
    strategies: list[StructuredOutputStrategy]
    """The structured output strategies the model accepts, best first."""

class ModelProviderSettings(TypedDict, total=False):
    """`ai.providers.<name>`: a Model provider the program declares, or a built-in it changes.

    Declared at the connection, a Run or a Session. `api_key` and `headers` go only to `base_url`. A built-in's
    default key (its conventional variable, such as `env:OPENAI_API_KEY`) goes only to its own address, so a built-in
    pointed elsewhere sends none. `api_key`, `headers` and `base_url` are set by the program's own code, configuration
    or an Operator policy: a Recipe and a Client acting for a model, an Integration included, never set them
    (`SettingLockedError`). For the program's own provider, only `models` applies."""

    format: ModelFormat
    base_url: str
    api_key: SecretReference | None
    headers: Mapping[str, SecretReference]
    """Header values are Secret references, as ADR 0056's `cdp_url` headers."""
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

class ModelAllowanceSettings(TypedDict, total=False):
    """`ai.allowance.*`: the most model Usage one Run may spend (ADR 0107), none by default, `None` for none.

    Not a Limit: each call reserves its worst case against it, so it is never passed; the first call that does not fit
    is refused with `ModelAllowanceReachedError`, and so is every later model call of the Run, a smaller one included,
    while the Run goes on. When several are set, the first reached holds. Any layer may set it, a Recipe and a Client
    acting for a model included; the Limits still stop the Run, and only they clear the uncapped mark."""

    price: Money | None
    """`ai.allowance.price`: priced through the price table; an unpriced model is refused with `model_price_unknown`."""
    tokens: int | None
    """`ai.allowance.tokens`: input plus output, as `ModelUsage.tokens`."""
    calls: int | None
    """`ai.allowance.calls`: as `ModelUsage.calls`."""

class RunAiSettings(AiSettings, total=False):
    """`ai.*` where a Run opens or above: a Label, a Session or a page takes no Model allowance
    (`setting_not_applicable`)."""

    allowance: ModelAllowanceSettings

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
class ModelToolChoice:
    """Whether the answer must call a tool: `tool`, with its name, is the forced tool call of the `tool` strategy."""

    type: Literal["auto", "none", "tool"]
    name: str | None = None

@final
@dataclass(frozen=True, kw_only=True)
class ModelRequest:
    """What the engine asks a handler: Goodfetch's own types, so no provider SDK is needed (G7)."""

    model_id: str
    """The model ID after the provider's name in the Model reference."""
    run_id: str
    """The Run asking."""
    messages: Sequence[ModelMessage]
    tools: Sequence[ModelTool]
    tool_choice: ModelToolChoice
    response_schema: JsonSchema | None
    """Set when the strategy is `native`: the answer must be JSON fitting it."""
    max_output_tokens: int

@final
@dataclass(frozen=True, kw_only=True)
class ReportedTokens:
    """The token counts a handler reports: input and output are required, so a Limit counts every call."""

    input_tokens: int
    output_tokens: int
    cache_read_tokens: int = 0
    cache_write_tokens: int = 0
    reasoning_tokens: int = 0

@final
@dataclass(frozen=True, kw_only=True)
class ModelResponse:
    """The program's answer; a refusal is `finish_reason="refusal"`, which the engine raises as `ModelDeclinedError`."""

    text: str
    finish_reason: Literal["stop", "length", "tool_calls", "refusal"] | str
    usage: ReportedTokens
    tool_calls: Sequence[ModelToolCall] = ()

AsyncModelProviderHandler: TypeAlias = Callable[[ModelRequest], Awaitable[ModelResponse]]
"""Answers model calls on the program's own event loop (ADR 0081); its task is cancelled when `timeout.model` passes or
the call is cancelled. It gets the `prompt` strategy unless its models declare others, and only configuration prices it.
A `GoodfetchError` it raises keeps its code (a `ModelUnavailableError` is retried as a provider's would be); any other
exception is `ModelHandlerFailedError`."""

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
    """Their price from the price table: a list-price estimate, never a bill, in the table's currency (USD as shipped),
    other currencies converted only through the table's fixed rates. `None` when part of it has no price, or needs a rate
    the table lacks: never a partial sum."""

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
class LimitPeriod:
    """A Limit's period, for one that counts per day, week or month (ADR 0041)."""

    unit: Literal["day", "week", "month"] | str
    time_zone: str
    """An IANA time zone, such as `Europe/Paris`."""
    resets_at: datetime
    """When the current period ends and the Limit's Usage starts again (ADR 0102)."""

@final
@dataclass(frozen=True, kw_only=True)
class AppliedLimit:
    """A Limit that applies to the Run, and what remains of it."""

    scope: LimitScope
    scope_id: str
    """The scope's ID (ADR 0102): the Run's ID, the Schedule's, the named token's name, the person, the Daemon or the
    install."""
    period: LimitPeriod | None
    """`None` for a Limit with no period."""
    kind: LimitKind
    value: int | timedelta | Money
    """A count, a `timedelta` for `browser_time`, or `Money` for `price`."""
    remaining: int | timedelta | Money
    set_by: Literal["person", "policy", "client"] | str
    """Who set it (ADRs 0041, 0045): only a person's or a managed policy's Limit allows Paid fetches."""

@final
@dataclass(frozen=True, kw_only=True)
class AppliedModelAllowance:
    """One kind of the Run's Model allowance, and what remains of it (ADR 0107). A newer engine's kind is kept (G3)."""

    kind: Literal["price", "tokens", "calls"] | str
    value: int | Money
    remaining: int | Money
    """The value less what the Run spent and has reserved for calls in flight."""
    reached_at: datetime | None
    """When a call was first refused for it; `None` until then."""
    source: LimitSource
    """`ai.allowance.<kind>`, its value and the layer that set it."""

@final
@dataclass(frozen=True, kw_only=True)
class RunUsage:
    """A Run's Usage so far: the figures the Usage ledger charges. Its Run report carries the final one (ADR 0102)."""

    model: ModelUsage
    browser_time: timedelta
    proxy_bytes: int
    paid_solves: int
    paid_fetches: int
    price: Money | None
    """The price of the Usage third parties charge for: model calls, metered proxies, paid solves and Paid fetches;
    `None` as `TokenCounts.price` when any of it has no price or rate. Browser time and an unmetered proxy carry none."""
    limits: Sequence[AppliedLimit]
    """The Limits that apply to the Run: what stops it."""
    model_allowance: Sequence[AppliedModelAllowance]
    """The Run's Model allowance, one entry per kind set, empty when none is (ADR 0107): not a Limit, so not in
    `limits`."""
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
    missing: Literal["price", "rate"] | str
    """What the price table lacks: the model's price, or a fixed rate to the Limit's currency."""

@final
@dataclass(frozen=True, kw_only=True)
class ModelUnavailableDetails:
    model: ModelReference
    status: int | None
    request_id: str | None
    reason: Literal["unreachable", "server_error", "rate_limited"] | str
    retry_after: timedelta | None
    """How long the provider asked to wait, when it said."""

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
    strategy: StructuredOutputStrategy | str
    """A newer engine's value is kept as received (G3)."""
    attempts: int
    problem: str
    """The last attempt's problem."""

@final
@dataclass(frozen=True, kw_only=True)
class ModelRequestRejectedDetails:
    model: ModelReference
    status: int | None
    request_id: str | None
    provider_message: str | None

@final
class ModelRequestRejectedError(ProgramError):
    """The provider rejected the request for another reason (a 400 or 422), such as a strategy `ai.structured_output`
    forces, or the last rung of `auto`'s ladder."""

    @property
    def details(self) -> ModelRequestRejectedDetails: ...

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
class ModelFundsExhaustedError(RefusedError):
    """The provider refused for the account's funds or a cap set at the provider: no credit left, an unpaid account, a
    used-up free tier, an organization's or workspace's spend limit, a tier's monthly cap. Not retryable."""

    @property
    def details(self) -> ModelFailureDetails: ...

@final
class ModelPriceUnknownError(RefusedError):
    """A currency Limit applies and the model's price cannot be stated in its currency: no price, or no fixed rate.
    `fix` names the configuration key."""

    @property
    def details(self) -> ModelPriceUnknownDetails: ...

@final
@dataclass(frozen=True, kw_only=True)
class ModelAllowanceReachedDetails:
    kind: Literal["price", "tokens", "calls"] | str
    value: int | Money
    spent: int | Money
    """What the Run had spent of that kind when the allowance was reached."""
    reached_at: datetime

@final
class ModelAllowanceReachedError(RefusedError):
    """The Run's Model allowance is reached (ADR 0107): every later model call of the Run is refused while the Run goes
    on. A step with no Plan entry, a compile or a Repair raises it; a step replayed from the Plan does not. Escaping a
    crawl handler, it ends the request `skipped` with the reason `model_allowance`. `limit` names `ai.allowance.<kind>`,
    its value and its layer; `usage` carries the call's model Usage. Not retryable within the Run."""

    @property
    def details(self) -> ModelAllowanceReachedDetails: ...

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
