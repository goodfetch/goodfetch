"""The Agent in Collie's SDK API (ADR 0100, decided in #474).

- `page.agent(task, ...)` runs ADR 0013's built-in Agent on one task, to its end: a thin, journaled loop over the
  Operation catalog's page operations, the program's Agent tools and a `note` action. It starts on the page, may use
  the tabs it opens, and carries nothing into the next call (G2). The whole task is one gesture of the page (ADR 0085);
  it gives up the page's turn while an Agent tool or its Success check runs.
- The program states the Success check with `expect` (ADR 0091) in `success_check`. The engine runs it each time the
  model reports done; a failure goes back to the model, and the Verdict reads its latest evaluation (ADR 0029).
- The result is ADR 0079's `AiWorkResult` plus the steps, their `ModelUsage` (ADR 0101) and the page the task ended
  on. A step cap, a stall rule, the task's timeout, a Limit and an abort are values: `ended` is `Stopped` with its
  reason, and `partial` holds what was Grounded.
- `page.agent(...)` returns a task handle with `result()` and `steps()`, the same in the sync and async APIs. It starts
  at the first of `result()`, `steps()` or entering its `with` block; leaving the block before it ends cancels it.
- The steps are a lossless sequence that starts at the task's first step (ADR 0081).
- With the Run's `plan=` (ADR 0091), the task is one Plan entry, replayed whole with no model call (before each
  Effect, every target reachable without it is checked). On Drift, or a failed Success check after the replay, the loop
  takes over live from there: no Repair (amends ADR 0009).
"""

from collections.abc import Awaitable, Callable, Mapping, Sequence
from dataclasses import dataclass
from types import TracebackType
from typing import Generic, Literal, Protocol, Self, TypeAlias, TypeVar, final, overload

from goodfetch._concurrency import AsyncEventIterator, EventIterator
from goodfetch._ai import Drift
from goodfetch._data import JsonSchema, JsonValue
from goodfetch._lifecycle import AsyncPage, Page
from goodfetch._models import ModelReference, ModelUsage
from goodfetch._page import AsyncLocator, Locator
from goodfetch._results import AiWorkResult, GoodfetchError, ProgramError
from goodfetch._time import NotGiven, Timeout

T = TypeVar("T")
L = TypeVar("L")
M = TypeVar("M")

# Steps.

AgentStepOrigin: TypeAlias = Literal["model", "plan"] | str
"""Where a step came from. `model`: decided by the model now. `plan`: replayed from the Plan, no model call. A newer
engine's value is kept (G3)."""

@final
@dataclass(frozen=True, kw_only=True)
class ActionDone: ...

@final
@dataclass(frozen=True, kw_only=True)
class ActionFailed:
    """The action failed and the failure went back to the model; it never raises from `page.agent`."""

    error: GoodfetchError

@final
@dataclass(frozen=True, kw_only=True)
class ActionJournaled:
    """The Effect journal already held this Effect, so nothing was done again (ADR 0010)."""

@final
@dataclass(frozen=True, kw_only=True)
class ActionUnknown:
    """An outcome this SDK does not know, kept as received (G3)."""

    raw: str

AgentActionOutcome: TypeAlias = ActionDone | ActionFailed | ActionJournaled | ActionUnknown

@final
@dataclass(frozen=True, kw_only=True)
class AgentAction(Generic[L]):
    """One action a step took."""

    operation: str
    """The catalog operation, such as `locator.click` or `page.goto`; `tool:<name>` for an Agent tool; `note` for the
    built-in action that notes the answer so far."""
    arguments: Sequence[JsonValue]
    """`%name%` placeholders where a variable or a secret was typed, never the value."""
    target: L | None
    """The target of a page action, an ordinary locator resolved again at each use."""
    code: str | None
    """A page action in Python, as the engine renders it (ADR 0091), so a person can turn a task into plain page API
    calls: `await page.get_by_role("link", name="Travel").click()`."""
    outcome: AgentActionOutcome

@final
@dataclass(frozen=True, kw_only=True)
class AgentStep(Generic[L]):
    """One step: one model decision and the actions it took, or one step replayed from the Plan."""

    index: int
    """From 1."""
    origin: AgentStepOrigin
    drift: Drift | None
    """On the first live step after a replay drifted, or after its Success check failed: what the Plan expected and what
    the page showed (ADR 0091). The replay ended there and the loop took over."""
    message: str
    """The model's own words for this step: a signal, never proof. Empty for a replayed step."""
    actions: Sequence[AgentAction[L]]
    url: str
    """The URL of the page the task is on after the step."""
    stalled: bool
    """Its actions failed, or repeated earlier ones on targets whose page had not changed since."""
    usage: ModelUsage
    """This step's model calls and their price: none for a replayed step."""

# Results and task handles.

class AsyncAgentResult(AiWorkResult[T]):
    """What a task returns in the async API: ADR 0079's three separate fields, the output, and what the task did.
    `output` is a valid `T` whose values Grounding found on the pages the task observed or in its Agent tools' results;
    a value it could not find leaves the Verdict at `needs-review`, naming the field. A task replayed whole ends
    `Finished` once its Success check ran, with no `reported`, since no model claimed anything."""

    @property
    def steps(self) -> Sequence[AgentStep[AsyncLocator]]: ...
    @property
    def usage(self) -> ModelUsage:
        """The whole task's model calls and their price (ADR 0101)."""

    @property
    def page(self) -> AsyncPage:
        """The page the task ended on: its own page, or a tab it opened, which the program now holds. The other tabs it
        opened are closed as `page.close()` closes them: on a Borrowed browser, released."""

class AgentResult(AiWorkResult[T]):
    """What a task returns in the sync API, as `AsyncAgentResult`."""

    @property
    def steps(self) -> Sequence[AgentStep[Locator]]: ...
    @property
    def usage(self) -> ModelUsage: ...
    @property
    def page(self) -> Page: ...

@final
class AsyncAgentTask(Generic[T]):
    """A task, started at the first of `result()`, `steps()` or `async with`. Leaving the block before it ends
    cancels it, as a released task is; a handle released without being started logs a warning, as a coroutine never
    awaited does."""

    def steps(self) -> AsyncEventIterator[AgentStep[AsyncLocator]]:
        """The task's steps, lossless, from step 1 whenever it is called, ending when the task ends and raising the
        task's error when it failed. Each call is a reader of its own; an unread open iterator slows the task (ADR
        0081)."""

    async def result(self) -> AsyncAgentResult[T]:
        """Waits for the task's end. An early end is a value (`ended`); a failure raises, with the steps taken and the
        partial answer in the error's `agent_task`."""

    async def __aenter__(self) -> Self: ...
    async def __aexit__(
        self, exc_type: type[BaseException] | None, exc: BaseException | None, tb: TracebackType | None
    ) -> None: ...

@final
class AgentTask(Generic[T]):
    """A task in the sync API, as `AsyncAgentTask`, with `with`."""

    def steps(self) -> EventIterator[AgentStep[Locator]]: ...
    def result(self) -> AgentResult[T]: ...
    def __enter__(self) -> Self: ...
    def __exit__(
        self, exc_type: type[BaseException] | None, exc: BaseException | None, tb: TracebackType | None
    ) -> None: ...

# Agent tools.

ConfirmationCategory: TypeAlias = (
    Literal["payment", "account_creation", "accepting_terms", "sending_messages", "deletion"] | str
)
"""ADR 0014's categories of Effects that ask a person for confirmation unless the run allows them; area 4's spec spells
them."""

class AsyncAgentTool:
    """A function of the program the Agent may call as an action, in the async API: an Agent tool."""

    @property
    def name(self) -> str: ...
    @property
    def description(self) -> str: ...
    @property
    def read_only(self) -> bool: ...
    @property
    def category(self) -> ConfirmationCategory | None: ...

class AgentTool:
    """An Agent tool in the sync API."""

    @property
    def name(self) -> str: ...
    @property
    def description(self) -> str: ...
    @property
    def read_only(self) -> bool: ...
    @property
    def category(self) -> ConfirmationCategory | None: ...

class _AgentToolDecorator(Protocol):
    @overload
    def __call__(self, function: Callable[[M, AsyncPage], Awaitable[JsonValue]], /) -> AsyncAgentTool: ...
    @overload
    def __call__(self, function: Callable[[M, Page], JsonValue], /) -> AgentTool: ...

def agent_tool(
    *,
    description: str,
    name: str | None = None,
    read_only: bool = False,
    category: ConfirmationCategory | None = None,
    input_schema: JsonSchema | None = None,
) -> _AgentToolDecorator:
    """Makes an Agent tool of a function taking its input and the task's current page, and returning the JSON the model
    reads, bounded by `observation.max_size`.

    - `name` defaults to the function's name; it must be unique within a task and no built-in action's, or the task
      fails with `AgentToolInvalidError`.
    - The input's schema is the first parameter's type, a Pydantic model (ADR 0082), checked before the function runs;
      `input_schema` gives a hand-written JSON Schema instead, and the function then receives `JsonValue`.
    - `read_only=False`, the default, makes the tool an Effect: journaled and never redone (ADR 0010), as a site's WebMCP
      tool (ADR 0011). `category` asks a person for confirmation before each call unless the run allows the category
      (ADR 0014); payment and accepting terms always ask, since their allowances name domains and a tool has none.
    - It runs where the program's code runs (ADR 0081), while the task gives up the page's turn. Its result is a source
      Grounding accepts. A `GoodfetchError` it raises ends the task with its own code, and any other exception with
      `AgentToolFailedError`; a cancellation is never wrapped. An answer the model should act on is a value it returns.
    """

# The Agent, on a page.

SuccessCheck: TypeAlias = Callable[[AsyncPage], Awaitable[None]]
"""The Success check of the async API, stated with `expect` (ADR 0091), run each time the model reports done. Each
`expect` in it is a Success check the engine evaluates live. A `PageError` it raises (`SuccessCheckFailedError`, a target
not unique) is the condition's answer, given back to the model; another `GoodfetchError` ends the task with its own code;
any other exception, with `SuccessCheckThrewError`; a cancellation is never wrapped."""

SyncSuccessCheck: TypeAlias = Callable[[Page], None]

class AsyncAgentApi:
    """The Agent, on an async page. `page.agent(task, ...)` is a plain method returning the task handle.

    - `success_check`: the Success check; without one, the Verdict is `needs-review` (ADR 0013).
    - `max_steps`: `agent.max_steps` (20) for this task; reaching it stops only the task.
    - `domains`: `agent.domains`, by default the registrable domain of the page the task starts on. An entry matches its
      host and their subdomains; `"*"` allows wherever the run's domain policy does. It bounds the task's navigations,
      never the subresources a page loads.
    - `variables`: plain values for `%name%` placeholders, as `act` takes them; a placeholder no variable fills is a
      Secret placeholder bound by `secrets.<name>` (ADR 0092).
    - `model`: `ai.model` for this task (ADR 0101).
    - `timeout`: `timeout.agent` (10 minutes); reaching it cancels the step in flight and stops the task with the reason
      `timeout`, it does not raise. An enclosing call's timeout raises `DeadlineExceededError` naming that call;
      `timeout.ai` does not apply to the task.
    """

    @overload
    def agent(
        self,
        task: str,
        *,
        schema: type[T],
        success_check: SuccessCheck | None = None,
        max_steps: int | NotGiven = ...,
        domains: Sequence[str] | NotGiven = ...,
        tools: Sequence[AsyncAgentTool] = (),
        variables: Mapping[str, str] | None = None,
        model: ModelReference | NotGiven = ...,
        timeout: Timeout = ...,
    ) -> AsyncAgentTask[T]: ...
    @overload
    def agent(
        self,
        task: str,
        *,
        schema: JsonSchema,
        success_check: SuccessCheck | None = None,
        max_steps: int | NotGiven = ...,
        domains: Sequence[str] | NotGiven = ...,
        tools: Sequence[AsyncAgentTool] = (),
        variables: Mapping[str, str] | None = None,
        model: ModelReference | NotGiven = ...,
        timeout: Timeout = ...,
    ) -> AsyncAgentTask[JsonValue]: ...
    @overload
    def agent(
        self,
        task: str,
        *,
        schema: None = None,
        success_check: SuccessCheck | None = None,
        max_steps: int | NotGiven = ...,
        domains: Sequence[str] | NotGiven = ...,
        tools: Sequence[AsyncAgentTool] = (),
        variables: Mapping[str, str] | None = None,
        model: ModelReference | NotGiven = ...,
        timeout: Timeout = ...,
    ) -> AsyncAgentTask[None]: ...

class AgentApi:
    """The Agent, on a sync page, as `AsyncAgentApi`."""

    @overload
    def agent(
        self,
        task: str,
        *,
        schema: type[T],
        success_check: SyncSuccessCheck | None = None,
        max_steps: int | NotGiven = ...,
        domains: Sequence[str] | NotGiven = ...,
        tools: Sequence[AgentTool] = (),
        variables: Mapping[str, str] | None = None,
        model: ModelReference | NotGiven = ...,
        timeout: Timeout = ...,
    ) -> AgentTask[T]: ...
    @overload
    def agent(
        self,
        task: str,
        *,
        schema: JsonSchema,
        success_check: SyncSuccessCheck | None = None,
        max_steps: int | NotGiven = ...,
        domains: Sequence[str] | NotGiven = ...,
        tools: Sequence[AgentTool] = (),
        variables: Mapping[str, str] | None = None,
        model: ModelReference | NotGiven = ...,
        timeout: Timeout = ...,
    ) -> AgentTask[JsonValue]: ...
    @overload
    def agent(
        self,
        task: str,
        *,
        schema: None = None,
        success_check: SyncSuccessCheck | None = None,
        max_steps: int | NotGiven = ...,
        domains: Sequence[str] | NotGiven = ...,
        tools: Sequence[AgentTool] = (),
        variables: Mapping[str, str] | None = None,
        model: ModelReference | NotGiven = ...,
        timeout: Timeout = ...,
    ) -> AgentTask[None]: ...

@final
@dataclass(frozen=True, kw_only=True)
class AgentTaskProgress:
    """On an error `page.agent` raises after the task took steps (ADR 0100)."""

    steps: Sequence[AgentStep[object]]
    partial: JsonValue | None
    """The partial answer as JSON: only values valid against their part of the schema and Grounded."""
    uncertain_effects: Sequence[str]
    """The Agent tools whose Effect the task left uncertain, for a person to reconcile (ADR 0029)."""

# The Agent's Error codes (ADR 0079's naming rule).

@final
@dataclass(frozen=True, kw_only=True)
class AgentToolFailedDetails:
    tool: str
    exception_type: str
    exception_message: str
    effect_uncertain: bool
    """The tool was not read-only, so its Effect is left uncertain for a person to reconcile (ADR 0029)."""

@final
class AgentToolFailedError(ProgramError):
    """An Agent tool raised something other than a `GoodfetchError`: the task ended. `__cause__` holds the original, in
    the process that raised it."""

    @property
    def details(self) -> AgentToolFailedDetails: ...

@final
@dataclass(frozen=True, kw_only=True)
class AgentToolInvalidDetails:
    tool: str
    problem: Literal["duplicate", "built_in_name"] | str

@final
class AgentToolInvalidError(ProgramError):
    """Two tools share a name, or one takes a built-in action's: refused before the task starts."""

    @property
    def details(self) -> AgentToolInvalidDetails: ...

@final
@dataclass(frozen=True, kw_only=True)
class SuccessCheckThrewDetails:
    exception_type: str
    exception_message: str

@final
class SuccessCheckThrewError(ProgramError):
    """The Success check function raised something other than `SuccessCheckFailedError`: a bug, never a rejection.
    `__cause__` holds it."""

    @property
    def details(self) -> SuccessCheckThrewDetails: ...
