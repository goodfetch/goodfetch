"""The testing kit's shared part, `goodfetch_collie_client.testing` (ADRs 0069 and 0105, decided in #482): public and
under semver.

Each Product ships its own kit, re-exporting this one and adding its test engine (ADRs 0090, 0095):
`goodfetch_collie.testing` and `goodfetch_foxhound.testing`, whose engine adds `run_handler` and `run_hook`, since crawl
handlers and Hooks are Foxhound's (ADR 0084). It fits any test runner: plain values closed with `async with` or `with`,
failures raised as `GoodfetchError`s, and assertions left to `assert` or the runner's own. No pytest plugin is needed: a
fixture that yields a started `AsyncTestEngine` is three lines.

- `ScriptedModel`: ADR 0069's Scripted model, a Model provider the test registers.
- `fake_error`: any `GoodfetchError`, for a test of the program's own error handling. Result values are generated
  models (ADRs 0038, 0082), so a test builds them with their constructors.
"""

from collections.abc import Sequence
from dataclasses import dataclass
from typing import TypeVar, final

from goodfetch_collie_client._models import ModelRequest, ModelResponse
from goodfetch_collie_client._results import GoodfetchError, ProgramError

E = TypeVar("E", bound=GoodfetchError)

@final
class ScriptedModel:
    """ADR 0069's Scripted model: answers call n with reply n, records every request, and reports Usage from its replies.
    A retryable error reply is retried as a provider's (ADR 0101), the retry taking the next reply; a call beyond its
    replies raises `ScriptedModelExhaustedError`. Register it in `model_providers`, sync or async."""

    def __init__(self, replies: Sequence[ModelResponse | GoodfetchError]) -> None: ...
    def __call__(self, request: ModelRequest) -> ModelResponse: ...
    @property
    def requests(self) -> Sequence[ModelRequest]:
        """Every request it answered, in order, for a test to check what the engine asked."""
    @property
    def remaining(self) -> int: ...

def fake_error(error_class: type[E], *, message: str = ..., details: object = ...) -> E:
    """Builds the error a call would raise: its class, group, retryable flag and details, for a test of the program's own
    error handling."""

@final
@dataclass(frozen=True, kw_only=True)
class ScriptedModelDetails:
    provider: str
    calls: int
    remaining: int

@final
class ScriptedModelExhaustedError(ProgramError):
    """A call went beyond the Scripted model's replies: the AI call that made it fails with this code (ADR 0101)."""

    @property
    def details(self) -> ScriptedModelDetails: ...

@final
class ScriptedModelUnusedError(ProgramError):
    """The test engine closed with replies of a Scripted model unused: the code under test called the model less than the
    test expected."""

    @property
    def details(self) -> ScriptedModelDetails: ...
