"""Collie's AI verbs and Plans (ADR 0091, decided in #473).

- `act`, `observe` and `extract` live on the page, and on a locator, whose one element bounds what the model sees and
  what a step may target. There is no AI runtime object and no active page (G2).
- Each call is one step of the Run's Plan (ADR 0009): replayed without a model when the Plan holds it and its Target
  fingerprint still matches, compiled by the model when it is missing, repaired once on Drift. Starting a Run with
  `plan=` names the file; the setting `plan.frozen` forbids any model call.
- Every verb returns a `StepResult`: its `output`, how the `step` was resolved, and its `usage`. Failures raise
  (ADR 0079); single steps carry no Verdict, which the Run's checks give (ADR 0029).
- `expect` runs a Success check under Playwright for Python's assertion names, evaluated live by the engine and read by
  the Run's Verdict; a condition still false at its timeout raises `SuccessCheckFailedError`.
- `aria_snapshot` and `get_by_ref` are the observation an agent loop reads, under Playwright's names.

Model references and `ModelUsage` are ADR 0101's (`_models`). Secret placeholders in instructions are ADR 0092's.
"""

import re
from collections.abc import Mapping, Sequence
from dataclasses import dataclass
from datetime import timedelta
from typing import Generic, Literal, TypeAlias, TypeVar, final, overload

from goodfetch_collie_client._data import JSONSchema, JSONValue
from goodfetch_collie_client._effects import UncertainEffect
from goodfetch_collie_client._lifecycle import AsyncPage, Page
from goodfetch_collie_client._models import ModelReference, ModelUsage, StructuredOutputStrategy
from goodfetch_collie_client._page import AsyncLocator, AsyncResponse, Locator, Response
from goodfetch_collie_client._results import ModelError, PageError, ProgramError, RefusedError
from goodfetch_collie_client._time import Count, NotGiven, Timeout

T = TypeVar("T")
T_co = TypeVar("T_co", covariant=True)
L = TypeVar("L")

StepSource: TypeAlias = Literal["plan", "model", "repair", "journal"] | str
"""How a step was resolved. `plan`: replayed from the Plan, no model call. `model`: compiled now, since the Plan held
no entry for it. `repair`: its entry drifted and one Repair recompiled it. `journal`: the Effect journal already held
this step's Effect, so nothing was done again (ADR 0010). A newer engine's value is kept as received (G3)."""

@final
@dataclass(frozen=True, kw_only=True)
class Drift:
    """What drifted, when a step was repaired."""

    expected: str
    """The Target fingerprint the Plan kept, rendered for a person."""
    observed: str | None
    """What stood there instead, or `None` when nothing matched."""

@final
@dataclass(frozen=True, kw_only=True)
class StepReport:
    source: StepSource
    instruction: str
    """The instruction as the Plan keys it, with `%name%` placeholders and never their values."""
    drift: Drift | None
    """Present when `source` is `repair`."""

@final
@dataclass(frozen=True, kw_only=True)
class StepResult(Generic[T_co]):
    """What every AI verb returns. A value of `output` is always complete; a failure raises."""

    output: T_co
    step: StepReport
    usage: ModelUsage
    """The model calls this step made and their price (ADR 0101): none for a step replayed from the Plan."""

@final
@dataclass(frozen=True, kw_only=True)
class PerformedAction(Generic[L]):
    """The action `act` performed, through the guard like any locator action (ADR 0019)."""

    method: str
    """The page API method, such as `click` or `fill`."""
    arguments: Sequence[JSONValue]
    """Its arguments, `%name%` placeholders where a variable was typed."""
    target: L
    """The target, an ordinary locator resolved again at each use."""
    code: str
    """The same call in Python, as the engine renders it: `await page.get_by_role("link", name="love").click()`."""

@final
@dataclass(frozen=True, kw_only=True)
class Observed(Generic[L]):
    """One element `observe` found for its instruction."""

    locator: L
    description: str
    """The model's description of the element: a signal, never proof."""
    code: str
    """The locator in Python, as the engine renders it: `page.get_by_role("button", name="Sign in")`."""

ExtractMode: TypeAlias = Literal["compiled", "model"]
"""`compiled`: the model writes each field's locators and transform once, kept in the Plan, and values are copied from
the page at every call (ADR 0013). `model`: the model reads the page at every call, every value checked by Grounding."""

AriaSnapshotMode: TypeAlias = Literal["default", "ai"]

class AsyncAIVerbs:
    """The AI verbs, on an async page and an async locator. Each call's timeout is `timeout.ai`.

    A call that would send the page's content to a model while `site_preferences.respect_use_signals` is on and the
    page's AI-use signal refuses the Run's Purpose raises `SitePreferencesRefusedError` (`use_signals`) before anything
    is sent; a step replayed from the Plan is not refused (ADRs 0085, 0091)."""

    async def act(
        self,
        instruction: str,
        *,
        variables: Mapping[str, str] | None = None,
        model: ModelReference | NotGiven = ...,
        timeout: Timeout = ...,
    ) -> StepResult[PerformedAction[AsyncLocator]]:
        """Needs a model unless its Plan entry replays: pass `model` or set `ai.model`.

        Performs one action described in words; raises `TargetNotFoundError` when nothing on the page fits it.

        `variables` are plain values for `%name%` placeholders; a placeholder no variable fills is a Secret placeholder,
        bound by the setting `secrets.<name>` (ADR 0092). The model, the Plan and the logs see only `%name%`. A
        placeholder neither fills, a variable no placeholder uses, or a name that is both raises
        `VariablesMismatchError`."""

    async def observe(
        self, instruction: str, *, model: ModelReference | NotGiven = ..., timeout: Timeout = ...
    ) -> StepResult[list[Observed[AsyncLocator]]]:
        """Needs a model unless its Plan entry replays: pass `model` or set `ai.model`.

        Finds the elements an instruction describes, without acting. None is an answer: an empty list."""

    @overload
    async def extract(
        self,
        instruction: str,
        schema: type[T],
        *,
        mode: ExtractMode | NotGiven = ...,
        model: ModelReference | NotGiven = ...,
        timeout: Timeout = ...,
    ) -> StepResult[T]:
        """Needs a model unless its Plan entry replays: pass `model` or set `ai.model`.

        Reads what the instruction describes as a value of `schema`, a type or a JSON Schema, in `extract.mode`
        (`mode` for this call); a field not on the page raises `ExtractionIncompleteError`, naming it."""
    @overload
    async def extract(
        self,
        instruction: str,
        schema: JSONSchema,
        *,
        mode: ExtractMode | NotGiven = ...,
        model: ModelReference | NotGiven = ...,
        timeout: Timeout = ...,
    ) -> StepResult[JSONValue]: ...
    async def aria_snapshot(
        self, *, mode: AriaSnapshotMode = "default", depth: int | None = None, boxes: bool = False, timeout: Timeout = ...
    ) -> str:
        """Playwright's accessibility snapshot. In `ai` mode it carries element references, marks page text untrusted
        (ADR 0014) and is bounded by `observation.max_size`, past which the deepest subtrees collapse into one line
        keeping their reference. Secret values are masked and cookies never appear, in both modes."""

class AIVerbs:
    """The AI verbs, on a sync page and a sync locator."""

    def act(
        self,
        instruction: str,
        *,
        variables: Mapping[str, str] | None = None,
        model: ModelReference | NotGiven = ...,
        timeout: Timeout = ...,
    ) -> StepResult[PerformedAction[Locator]]: ...
    def observe(
        self, instruction: str, *, model: ModelReference | NotGiven = ..., timeout: Timeout = ...
    ) -> StepResult[list[Observed[Locator]]]: ...
    @overload
    def extract(
        self,
        instruction: str,
        schema: type[T],
        *,
        mode: ExtractMode | NotGiven = ...,
        model: ModelReference | NotGiven = ...,
        timeout: Timeout = ...,
    ) -> StepResult[T]: ...
    @overload
    def extract(
        self,
        instruction: str,
        schema: JSONSchema,
        *,
        mode: ExtractMode | NotGiven = ...,
        model: ModelReference | NotGiven = ...,
        timeout: Timeout = ...,
    ) -> StepResult[JSONValue]: ...
    def aria_snapshot(
        self, *, mode: AriaSnapshotMode = "default", depth: int | None = None, boxes: bool = False, timeout: Timeout = ...
    ) -> str: ...

class AsyncElementReferences:
    def get_by_ref(self, ref: str) -> AsyncLocator:
        """The element an `ai` snapshot gave this reference, such as `e12`. References last as long as their element;
        one whose element is gone raises `TargetNotFoundError` at once."""

class ElementReferences:
    def get_by_ref(self, ref: str) -> Locator: ...

@final
@dataclass(frozen=True, kw_only=True)
class PlanReport:
    """What became of the Run's Plan: the Run report's `plan` (ADR 0102)."""

    file: str | None
    """The file the Run named, or `None` when the Plan lives only in its Run trace."""
    version_used: str | None
    status: Literal["unchanged", "promoted", "pending_review", "frozen"] | str
    """`promoted`: written after a `verified` Run; `pending_review`: a new version a person must accept, from a Run that
    was not `verified` or a Plan with Effects (ADR 0009); `frozen`: `plan.frozen` was set."""
    changed_steps: Sequence[str]

TextExpected: TypeAlias = str | re.Pattern[str]
"""As in Playwright's assertions: a `str` matches the whole value (`to_have_url`'s exactly, text after whitespace is
normalized), except in `to_contain_text`, which matches a part; a pattern matches a pattern."""


@final
class AsyncPageExpectations:
    """Playwright's page assertions. Each call is a Success check the Run's Verdict reads; its timeout is
    `timeout.wait` (30 s), not Playwright's 5 s (G1)."""

    async def to_have_url(self, url: TextExpected, *, timeout: Timeout = ...) -> None: ...
    async def not_to_have_url(self, url: TextExpected, *, timeout: Timeout = ...) -> None: ...
    async def to_have_title(self, title: TextExpected, *, timeout: Timeout = ...) -> None: ...
    async def not_to_have_title(self, title: TextExpected, *, timeout: Timeout = ...) -> None: ...
    async def to_match_aria_snapshot(self, expected: str, *, timeout: Timeout = ...) -> None: ...
    async def not_to_match_aria_snapshot(self, expected: str, *, timeout: Timeout = ...) -> None: ...

@final
class PageExpectations:
    def to_have_url(self, url: TextExpected, *, timeout: Timeout = ...) -> None: ...
    def not_to_have_url(self, url: TextExpected, *, timeout: Timeout = ...) -> None: ...
    def to_have_title(self, title: TextExpected, *, timeout: Timeout = ...) -> None: ...
    def not_to_have_title(self, title: TextExpected, *, timeout: Timeout = ...) -> None: ...
    def to_match_aria_snapshot(self, expected: str, *, timeout: Timeout = ...) -> None: ...
    def not_to_match_aria_snapshot(self, expected: str, *, timeout: Timeout = ...) -> None: ...

@final
class AsyncLocatorExpectations:
    """Playwright's locator assertions, all but `to_have_screenshot`."""

    async def to_be_attached(self, *, timeout: Timeout = ...) -> None: ...
    async def not_to_be_attached(self, *, timeout: Timeout = ...) -> None: ...
    async def to_be_checked(self, *, checked: bool = True, timeout: Timeout = ...) -> None: ...
    async def not_to_be_checked(self, *, checked: bool = True, timeout: Timeout = ...) -> None: ...
    async def to_be_disabled(self, *, timeout: Timeout = ...) -> None: ...
    async def not_to_be_disabled(self, *, timeout: Timeout = ...) -> None: ...
    async def to_be_editable(self, *, timeout: Timeout = ...) -> None: ...
    async def not_to_be_editable(self, *, timeout: Timeout = ...) -> None: ...
    async def to_be_empty(self, *, timeout: Timeout = ...) -> None: ...
    async def not_to_be_empty(self, *, timeout: Timeout = ...) -> None: ...
    async def to_be_enabled(self, *, timeout: Timeout = ...) -> None: ...
    async def not_to_be_enabled(self, *, timeout: Timeout = ...) -> None: ...
    async def to_be_focused(self, *, timeout: Timeout = ...) -> None: ...
    async def not_to_be_focused(self, *, timeout: Timeout = ...) -> None: ...
    async def to_be_hidden(self, *, timeout: Timeout = ...) -> None: ...
    async def not_to_be_hidden(self, *, timeout: Timeout = ...) -> None: ...
    async def to_be_in_viewport(self, *, ratio: float | None = None, timeout: Timeout = ...) -> None: ...
    async def not_to_be_in_viewport(self, *, ratio: float | None = None, timeout: Timeout = ...) -> None: ...
    async def to_be_visible(self, *, timeout: Timeout = ...) -> None: ...
    async def not_to_be_visible(self, *, timeout: Timeout = ...) -> None: ...
    async def to_contain_text(self, expected: TextExpected | Sequence[TextExpected], *, timeout: Timeout = ...) -> None: ...
    async def not_to_contain_text(self, expected: TextExpected | Sequence[TextExpected], *, timeout: Timeout = ...) -> None: ...
    async def to_contain_class(self, expected: str | Sequence[str], *, timeout: Timeout = ...) -> None: ...
    async def not_to_contain_class(self, expected: str | Sequence[str], *, timeout: Timeout = ...) -> None: ...
    async def to_have_accessible_description(self, expected: TextExpected, *, timeout: Timeout = ...) -> None: ...
    async def not_to_have_accessible_description(self, expected: TextExpected, *, timeout: Timeout = ...) -> None: ...
    async def to_have_accessible_error_message(self, expected: TextExpected, *, timeout: Timeout = ...) -> None: ...
    async def not_to_have_accessible_error_message(self, expected: TextExpected, *, timeout: Timeout = ...) -> None: ...
    async def to_have_accessible_name(self, expected: TextExpected, *, timeout: Timeout = ...) -> None: ...
    async def not_to_have_accessible_name(self, expected: TextExpected, *, timeout: Timeout = ...) -> None: ...
    async def to_have_attribute(self, name: str, value: TextExpected | NotGiven = ..., *, timeout: Timeout = ...) -> None: ...
    async def not_to_have_attribute(self, name: str, value: TextExpected | NotGiven = ..., *, timeout: Timeout = ...) -> None: ...
    async def to_have_class(self, expected: TextExpected | Sequence[TextExpected], *, timeout: Timeout = ...) -> None: ...
    async def not_to_have_class(self, expected: TextExpected | Sequence[TextExpected], *, timeout: Timeout = ...) -> None: ...
    async def to_have_count(self, count: int | Count, *, timeout: Timeout = ...) -> None: ...
    async def not_to_have_count(self, count: int | Count, *, timeout: Timeout = ...) -> None: ...
    async def to_have_css(self, name: str, value: TextExpected, *, timeout: Timeout = ...) -> None: ...
    async def not_to_have_css(self, name: str, value: TextExpected, *, timeout: Timeout = ...) -> None: ...
    async def to_have_id(self, id: TextExpected, *, timeout: Timeout = ...) -> None: ...
    async def not_to_have_id(self, id: TextExpected, *, timeout: Timeout = ...) -> None: ...
    async def to_have_js_property(self, name: str, value: JSONValue, *, timeout: Timeout = ...) -> None: ...
    async def not_to_have_js_property(self, name: str, value: JSONValue, *, timeout: Timeout = ...) -> None: ...
    async def to_have_role(self, role: str, *, timeout: Timeout = ...) -> None: ...
    async def not_to_have_role(self, role: str, *, timeout: Timeout = ...) -> None: ...
    async def to_have_text(self, expected: TextExpected | Sequence[TextExpected], *, timeout: Timeout = ...) -> None: ...
    async def not_to_have_text(self, expected: TextExpected | Sequence[TextExpected], *, timeout: Timeout = ...) -> None: ...
    async def to_have_value(self, value: TextExpected, *, timeout: Timeout = ...) -> None: ...
    async def not_to_have_value(self, value: TextExpected, *, timeout: Timeout = ...) -> None: ...
    async def to_have_values(self, values: Sequence[TextExpected], *, timeout: Timeout = ...) -> None: ...
    async def not_to_have_values(self, values: Sequence[TextExpected], *, timeout: Timeout = ...) -> None: ...
    async def to_match_aria_snapshot(self, expected: str, *, timeout: Timeout = ...) -> None: ...
    async def not_to_match_aria_snapshot(self, expected: str, *, timeout: Timeout = ...) -> None: ...

@final
class LocatorExpectations:
    def to_be_attached(self, *, timeout: Timeout = ...) -> None: ...
    def not_to_be_attached(self, *, timeout: Timeout = ...) -> None: ...
    def to_be_checked(self, *, checked: bool = True, timeout: Timeout = ...) -> None: ...
    def not_to_be_checked(self, *, checked: bool = True, timeout: Timeout = ...) -> None: ...
    def to_be_disabled(self, *, timeout: Timeout = ...) -> None: ...
    def not_to_be_disabled(self, *, timeout: Timeout = ...) -> None: ...
    def to_be_editable(self, *, timeout: Timeout = ...) -> None: ...
    def not_to_be_editable(self, *, timeout: Timeout = ...) -> None: ...
    def to_be_empty(self, *, timeout: Timeout = ...) -> None: ...
    def not_to_be_empty(self, *, timeout: Timeout = ...) -> None: ...
    def to_be_enabled(self, *, timeout: Timeout = ...) -> None: ...
    def not_to_be_enabled(self, *, timeout: Timeout = ...) -> None: ...
    def to_be_focused(self, *, timeout: Timeout = ...) -> None: ...
    def not_to_be_focused(self, *, timeout: Timeout = ...) -> None: ...
    def to_be_hidden(self, *, timeout: Timeout = ...) -> None: ...
    def not_to_be_hidden(self, *, timeout: Timeout = ...) -> None: ...
    def to_be_in_viewport(self, *, ratio: float | None = None, timeout: Timeout = ...) -> None: ...
    def not_to_be_in_viewport(self, *, ratio: float | None = None, timeout: Timeout = ...) -> None: ...
    def to_be_visible(self, *, timeout: Timeout = ...) -> None: ...
    def not_to_be_visible(self, *, timeout: Timeout = ...) -> None: ...
    def to_contain_text(self, expected: TextExpected | Sequence[TextExpected], *, timeout: Timeout = ...) -> None: ...
    def not_to_contain_text(self, expected: TextExpected | Sequence[TextExpected], *, timeout: Timeout = ...) -> None: ...
    def to_contain_class(self, expected: str | Sequence[str], *, timeout: Timeout = ...) -> None: ...
    def not_to_contain_class(self, expected: str | Sequence[str], *, timeout: Timeout = ...) -> None: ...
    def to_have_accessible_description(self, expected: TextExpected, *, timeout: Timeout = ...) -> None: ...
    def not_to_have_accessible_description(self, expected: TextExpected, *, timeout: Timeout = ...) -> None: ...
    def to_have_accessible_error_message(self, expected: TextExpected, *, timeout: Timeout = ...) -> None: ...
    def not_to_have_accessible_error_message(self, expected: TextExpected, *, timeout: Timeout = ...) -> None: ...
    def to_have_accessible_name(self, expected: TextExpected, *, timeout: Timeout = ...) -> None: ...
    def not_to_have_accessible_name(self, expected: TextExpected, *, timeout: Timeout = ...) -> None: ...
    def to_have_attribute(self, name: str, value: TextExpected | NotGiven = ..., *, timeout: Timeout = ...) -> None: ...
    def not_to_have_attribute(self, name: str, value: TextExpected | NotGiven = ..., *, timeout: Timeout = ...) -> None: ...
    def to_have_class(self, expected: TextExpected | Sequence[TextExpected], *, timeout: Timeout = ...) -> None: ...
    def not_to_have_class(self, expected: TextExpected | Sequence[TextExpected], *, timeout: Timeout = ...) -> None: ...
    def to_have_count(self, count: int | Count, *, timeout: Timeout = ...) -> None: ...
    def not_to_have_count(self, count: int | Count, *, timeout: Timeout = ...) -> None: ...
    def to_have_css(self, name: str, value: TextExpected, *, timeout: Timeout = ...) -> None: ...
    def not_to_have_css(self, name: str, value: TextExpected, *, timeout: Timeout = ...) -> None: ...
    def to_have_id(self, id: TextExpected, *, timeout: Timeout = ...) -> None: ...
    def not_to_have_id(self, id: TextExpected, *, timeout: Timeout = ...) -> None: ...
    def to_have_js_property(self, name: str, value: JSONValue, *, timeout: Timeout = ...) -> None: ...
    def not_to_have_js_property(self, name: str, value: JSONValue, *, timeout: Timeout = ...) -> None: ...
    def to_have_role(self, role: str, *, timeout: Timeout = ...) -> None: ...
    def not_to_have_role(self, role: str, *, timeout: Timeout = ...) -> None: ...
    def to_have_text(self, expected: TextExpected | Sequence[TextExpected], *, timeout: Timeout = ...) -> None: ...
    def not_to_have_text(self, expected: TextExpected | Sequence[TextExpected], *, timeout: Timeout = ...) -> None: ...
    def to_have_value(self, value: TextExpected, *, timeout: Timeout = ...) -> None: ...
    def not_to_have_value(self, value: TextExpected, *, timeout: Timeout = ...) -> None: ...
    def to_have_values(self, values: Sequence[TextExpected], *, timeout: Timeout = ...) -> None: ...
    def not_to_have_values(self, values: Sequence[TextExpected], *, timeout: Timeout = ...) -> None: ...
    def to_match_aria_snapshot(self, expected: str, *, timeout: Timeout = ...) -> None: ...
    def not_to_match_aria_snapshot(self, expected: str, *, timeout: Timeout = ...) -> None: ...

@final
class AsyncResponseExpectations:
    async def to_be_ok(self) -> None: ...
    async def not_to_be_ok(self) -> None: ...

@final
class ResponseExpectations:
    def to_be_ok(self) -> None: ...
    def not_to_be_ok(self) -> None: ...

@overload
def expect(target: AsyncPage) -> AsyncPageExpectations: ...
@overload
def expect(target: Page) -> PageExpectations: ...
@overload
def expect(target: AsyncLocator) -> AsyncLocatorExpectations: ...
@overload
def expect(target: Locator) -> LocatorExpectations: ...
@overload
def expect(target: AsyncResponse) -> AsyncResponseExpectations: ...
@overload
def expect(target: Response) -> ResponseExpectations: ...

# The AI verbs' Error codes (ADR 0079's naming rule).

@final
@dataclass(frozen=True, kw_only=True)
class TargetNotFoundDetails:
    instruction: str | None
    ref: str | None

@final
class TargetNotFoundError(PageError):
    """`act` found no element for its instruction, or `get_by_ref` named an element that is gone."""

    @property
    def details(self) -> TargetNotFoundDetails: ...

@final
@dataclass(frozen=True, kw_only=True)
class PlanDriftDetails:
    instruction: str
    drift: Drift
    why_no_repair: Literal["frozen", "no_model", "repair_drifted"] | str

@final
class PlanDriftError(PageError):
    """A step drifted and could not be repaired: `plan.frozen`, no model configured, or its one Repair drifted too."""

    @property
    def details(self) -> PlanDriftDetails: ...

@final
@dataclass(frozen=True, kw_only=True)
class PlanStepMissingDetails:
    instruction: str
    file: str | None

@final
class PlanStepMissingError(ProgramError):
    """`plan.frozen` is set and the Plan holds no entry for this call."""

    @property
    def details(self) -> PlanStepMissingDetails: ...

@final
@dataclass(frozen=True, kw_only=True)
class PlanInvalidDetails:
    file: str
    problem: str

@final
class PlanInvalidError(ProgramError):
    """The Plan file cannot be read or does not match its format."""

    @property
    def details(self) -> PlanInvalidDetails: ...

@final
@dataclass(frozen=True, kw_only=True)
class ModelNotConfiguredDetails:
    instruction: str

@final
class ModelNotConfiguredError(ProgramError):
    """A call the Plan holds no entry for needs a model, and none is configured; `fix` names `ai.model` (ADR 0101)."""

    @property
    def details(self) -> ModelNotConfiguredDetails: ...

@final
@dataclass(frozen=True, kw_only=True)
class MissingField:
    path: Sequence[str | int]
    reason: Literal["absent", "no_match"] | str

@final
@dataclass(frozen=True, kw_only=True)
class ExtractionIncompleteDetails:
    found: JSONValue
    """The fields that were found, never typed as complete."""
    missing: Sequence[MissingField]

@final
class ExtractionIncompleteError(PageError):
    """Required fields were not found on the page."""

    @property
    def details(self) -> ExtractionIncompleteDetails: ...

@final
@dataclass(frozen=True, kw_only=True)
class ExtractionIssue:
    path: Sequence[str | int]
    message: str

@final
@dataclass(frozen=True, kw_only=True)
class ExtractionInvalidDetails:
    attempts: int
    model: ModelReference
    strategy: StructuredOutputStrategy | str
    """The strategy of the last attempt (ADR 0101); a newer engine's value is kept as received."""
    issues: Sequence[ExtractionIssue]
    """The last attempt's problems, per field."""

@final
class ExtractionInvalidError(ModelError):
    """The model's answers never passed the JSON Schema, the program's validators (ADR 0082) or Grounding within
    ADR 0012's retries."""

    @property
    def details(self) -> ExtractionInvalidDetails: ...

EffectUncertainDetails: TypeAlias = UncertainEffect
"""The uncertain Effect: an AI action's instruction, or a declared Effect's key (ADR 0106)."""

@final
class EffectUncertainError(PageError):
    """An Effect may have happened and Goodfetch cannot tell: an `act` whose action the engine classed an Effect failed
    after the action was sent, or a declared Effect's body ended after a send (ADR 0106), now or in an earlier attempt.
    Never retry: a person reconciles it (ADR 0029). `__cause__` holds the original, when it was raised in this
    process."""

    @property
    def details(self) -> EffectUncertainDetails: ...

@final
@dataclass(frozen=True, kw_only=True)
class EffectNotConfirmedDetails:
    instruction: str
    category: str
    by: Literal["person", "timeout"] | str

@final
class EffectNotConfirmedError(RefusedError):
    """An AI Effect in one of ADR 0014's categories was not confirmed: a person declined its Human pause, or the pause
    expired (ADR 0092)."""

    @property
    def details(self) -> EffectNotConfirmedDetails: ...

@final
@dataclass(frozen=True, kw_only=True)
class SuccessCheckFailedDetails:
    check: str
    """The Success check in Python: `expect(page).to_have_url(re.compile(r"^https://quotes\\.toscrape\\.com/author/"))`."""
    expected: str
    observed: str | None
    """The last value seen, or `None` when no element matched."""
    timeout: timedelta

@final
class SuccessCheckFailedError(PageError):
    """`expect`'s condition was still false at its timeout; `limit` names `timeout.wait` and its layer."""

    @property
    def details(self) -> SuccessCheckFailedDetails: ...

@final
@dataclass(frozen=True, kw_only=True)
class VariablesMismatchDetails:
    missing: Sequence[str]
    unused: Sequence[str]
    conflicting: Sequence[str]

@final
class VariablesMismatchError(ProgramError):
    """A `%name%` placeholder neither `variables` nor a bound Secret placeholder fills, a variable no placeholder uses,
    or a name that is both."""

    @property
    def details(self) -> VariablesMismatchDetails: ...
