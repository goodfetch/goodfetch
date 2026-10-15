"""Foxhound's testing kit, `goodfetch_foxhound.testing` (ADRs 0069 and 0105, decided in #482): public and under semver.

It re-exports the shared kit of `goodfetch_collie_client.testing` and adds a test engine whose connection is
`AsyncFoxhound` or `Foxhound`, with `run_handler` (one crawl handler on one saved response, reporting how its attempt
ended and what it staged, without a Frontier, Hooks or a commit) and `run_hook` (one of the crawl's Hooks, by name, the
same way). It fits any test runner: plain values closed with `async with` or `with`, failures raised as
`GoodfetchError`s, and assertions left to `assert` or the runner's own. No pytest plugin is needed: a fixture that yields
a started `AsyncTestEngine` is three lines.
"""

from collections.abc import Generator, Mapping, Sequence
from dataclasses import dataclass
from os import PathLike
from types import TracebackType
from typing import Any, ClassVar, Literal, TypeAlias, TypeVar, final, overload

from goodfetch_collie_client.testing import *
from goodfetch_collie_client._data import JSONValue
from goodfetch_collie_client._effects import UncertainWhy
from goodfetch_collie_client._models import AsyncModelProviderHandler, ModelProviderHandler, ModelUsage
from goodfetch_collie_client._output import StoredBytes
from goodfetch_collie_client._results import BlockCause, GoodfetchError, ProgramError
from goodfetch_foxhound._connection import AsyncFoxhound, Foxhound
from goodfetch_foxhound._crawl import AsyncCrawl, Crawl, Enqueued, FetchTier
from goodfetch_foxhound._data import Dataset, Label
from goodfetch_foxhound._outcomes import SkipReason
from goodfetch_foxhound._settings import Settings

P = TypeVar("P")
T = TypeVar("T")

@final
@dataclass(frozen=True, kw_only=True)
class SavedResponse:
    """The document a handler runs on, given as `route.fulfill` gives a response, or read from a saved file."""

    status: int = 200
    headers: Mapping[str, str] | None = None
    content_type: str | None = None
    """Default: from `path`'s extension, else `text/html`."""
    body: str | bytes | None = None
    path: str | PathLike[str] | None = None
    """A file on the program's machine, relative to the working directory, such as a page saved from a browser."""

@final
@dataclass(frozen=True, kw_only=True)
class Returned:
    """The handler returned: what it staged would commit."""

    kind: Literal["returned"] = "returned"

@final
@dataclass(frozen=True, kw_only=True)
class Skipped:
    """`ctx.skip(message)` (reason `handler`), or a handler's navigation that Site preferences refused and it let
    escape."""

    reason: SkipReason
    message: str | None
    kind: Literal["skipped"] = "skipped"

@final
@dataclass(frozen=True, kw_only=True)
class RetryRequested:
    """`ctx.retry(message)`."""

    message: str
    kind: Literal["retry_requested"] = "retry_requested"

@final
@dataclass(frozen=True, kw_only=True)
class Escalated:
    """A call the tier cannot serve; in a crawl, the request would run again in the browser."""

    call: str
    """That call, rendered as the program wrote it (ADR 0079)."""
    kind: Literal["escalated"] = "escalated"

@final
@dataclass(frozen=True, kw_only=True)
class Blocked:
    """The document's status is a Block cause (ADRs 0021, 0093): the handler did not run."""

    cause: BlockCause
    kind: Literal["blocked"] = "blocked"

@final
@dataclass(frozen=True, kw_only=True)
class Failed:
    """The handler raised: its own exception as `handler_failed`, or the code a `GoodfetchError` keeps; or, before it
    ran, the document's status failed it (`response_not_ok`, `response_unavailable`)."""

    error: GoodfetchError
    kind: Literal["failed"] = "failed"

@final
@dataclass(frozen=True, kw_only=True)
class UnknownEnd:
    """A kind this SDK does not know, as received (G3)."""

    raw: str
    kind: Literal["unknown"] = "unknown"

AttemptEnd: TypeAlias = Returned | Skipped | RetryRequested | Escalated | Blocked | Failed | UnknownEnd

@final
@dataclass(frozen=True, kw_only=True)
class StagedRequest:
    """An enqueue or a seed the attempt staged, with the answer it got."""

    url: str
    label: str
    payload: JSONValue
    answer: Enqueued

@final
@dataclass(frozen=True, kw_only=True)
class StagedWrite:
    """A Key-value store write the attempt staged; `None` for a delete."""

    key: str
    value: JSONValue | StoredBytes | None

@final
class AttemptResult:
    """One attempt of a handler: how it ended and everything it staged, kept even when it ended early, where a crawl would
    discard it (ADR 0003). Nothing is committed."""

    @property
    def ended(self) -> AttemptEnd: ...
    @property
    def tier(self) -> FetchTier | str: ...
    def records(self, dataset: Dataset[T]) -> Sequence[T]:
        """The Records pushed to a Dataset, as `run.records()` would read them back."""
    @property
    def enqueued(self) -> Sequence[StagedRequest]: ...
    @property
    def seeds(self) -> Sequence[StagedRequest]:
        """`ctx.run.add_seed()` calls."""
    @property
    def key_value_writes(self) -> Sequence[StagedWrite]: ...
    @property
    def effects(self) -> Sequence[DeclaredEffect]:
        """The Effects it declared, in order (ADR 0106)."""
    @property
    def usage(self) -> ModelUsage:
        """Model calls made by AI verbs in the handler (ADR 0101)."""

@final
@dataclass(frozen=True, kw_only=True)
class HookResult:
    """What a Hook did: `answered` with its value, the call that ended the work, or `failed` with its error."""

    kind: Literal["answered", "skipped", "blocked", "retry_requested", "dropped", "rejected", "failed"] | str
    value: JSONValue | None = None
    message: str | None = None
    cause: BlockCause | None = None
    """For `blocked`: the Block cause `block(cause)` gave (ADR 0104)."""
    error: GoodfetchError | None = None

@final
class _AsyncStarting:
    def __await__(self) -> Generator[None, None, AsyncTestEngine]: ...
    async def __aenter__(self) -> AsyncTestEngine: ...
    async def __aexit__(
        self, exc_type: type[BaseException] | None, exc: BaseException | None, tb: TracebackType | None
    ) -> None: ...

@final
@dataclass(frozen=True, kw_only=True)
class PriorJournal:
    """A prior answer the Effect journal holds for a key in a test: done, with its output (ADR 0106)."""

    output: JSONValue

@final
@dataclass(frozen=True)
class PriorReconciled:
    """A person answered "it happened" (ADR 0029)."""

@final
@dataclass(frozen=True, kw_only=True)
class PriorUncertain:
    """Left uncertain, for this reason."""

    why: UncertainWhy

PriorEffect: TypeAlias = PriorJournal | PriorReconciled | PriorUncertain

@final
@dataclass(frozen=True, kw_only=True)
class DeclaredEffect:
    """One declaration an attempt made, and how it was answered."""

    key: str
    answer: Literal["performed", "journal", "reconciled", "uncertain", "not_performed"] | str
    sent: bool
    """Whether its body sent anything; a command a Response recording answered counts as sent."""

@final
class AsyncTestEngine:
    """A real engine for one test, for `AsyncFoxhound`: a temporary Goodfetch home removed when it closes, a cleared
    environment, no project file unless named, `offline` (ADR 0043), short durations, and no Model provider unless the
    test registers or configures one, so a call that needs a model with none raises `ModelNotConfiguredError`. Every Run
    started from `connection` that names no Response recording replays an empty one: a request to a site raises
    `ResponseNotRecordedError`, naming it. Browsers come from the browser cache of the caller's Goodfetch home, read and
    never written, or `GOODFETCH_BROWSERS_PATH` (ADRs 0024, 0059, 0105). An Attached engine (ADR 0069)."""

    @classmethod
    def start(
        cls,
        *,
        settings: Settings = ...,
        env: Mapping[str, str] = ...,
        project: str | PathLike[str] | None = None,
        model_providers: Mapping[str, AsyncModelProviderHandler | ScriptedModel] | None = None,
    ) -> _AsyncStarting:
        """`env` is `connect()`'s environment map (ADRs 0062, 0083), the process's own when left out: only
        `GOODFETCH_HOME` (to read its browser cache, never to write), `GOODFETCH_BROWSERS_PATH` and this Product's
        engine-path variable are read from it, and the engine's own environment stays cleared. Every host its `resolve` setting (ADR 0059) maps to loopback, such as a test's own server, is allowlisted
        (ADR 0017) and reached live even while a Run replays. `model_providers` are named by `ai.model` as
        `"<name>/<any>"` (ADR 0101)."""

    @property
    def connection(self) -> AsyncFoxhound:
        """A connection to this engine, `AsyncFoxhound.connect()`'s type: everything a program does works on it."""

    @overload
    async def run_handler(
        self,
        crawl: AsyncCrawl,
        label: Label[None],
        *,
        url: str,
        response: SavedResponse | None = None,
        recording: str | PathLike[str] | None = None,
        tier: FetchTier | None = None,
        attempt: int = 1,
        depth: int = 0,
        effects: Mapping[str, PriorEffect] | None = None,
    ) -> AttemptResult: ...
    @overload
    async def run_handler(
        self,
        crawl: AsyncCrawl,
        label: Label[P],
        *,
        url: str,
        payload: P,
        response: SavedResponse | None = None,
        recording: str | PathLike[str] | None = None,
        tier: FetchTier | None = None,
        attempt: int = 1,
        depth: int = 0,
        effects: Mapping[str, PriorEffect] | None = None,
    ) -> AttemptResult:
        """Runs the Label's handler, the one the crawl attached, on one request, with the Label's settings, in a Run of its
        own: one attempt, no retry. Nothing else of the crawl runs: no Frontier, no Hooks, no commit. The document is not
        fetched, so neither robots.txt nor Politeness applies to it, but ADR 0093's status rules do, before the handler
        runs. `response` is the document; left out, the
        response `recording` holds for `url`. `recording` is replayed for everything else the handler reaches. `tier` is
        `http` by default, or the one the Label's `fetch.tier` forces. No browser starts on a non-browser tier: a call only
        a browser serves ends the attempt `Escalated` under `fetch.tier: auto`, and `Failed` with `browser_required` under
        a forced tier (ADR 0093); `browser` renders the document. Raises when it cannot run the handler
        (`ResponseNotRecordedError` for the document, `PayloadInvalidError`, `LabelUnknownError`, `LabelUnhandledError`,
        `SettingInvalidError` for a tier other than a forced one);
        once the handler ran, how its attempt ended is a value."""

    async def run_hook(
        self,
        crawl: AsyncCrawl,
        name: str,
        *,
        label: Label[Any],
        url: str,
        payload: JSONValue = None,
        method: str = "GET",
        body: str | bytes | None = None,
        headers: Mapping[str, str] | None = None,
        tier: FetchTier | None = None,
        previous_error: GoodfetchError | None = None,
        attempt: int = 1,
        depth: int = 0,
        response: SavedResponse | None = None,
        recording: str | PathLike[str] | None = None,
        dataset: Dataset[Any] | None = None,
        record: JSONValue = None,
    ) -> HookResult:
        """Runs one of the crawl's Hooks alone, by the name Hooks carry (ADR 0104), its stage's context built as
        `run_handler` builds a handler's: before a request from `method`, `body`, `headers`, `tier` and `previous_error`
        (a first attempt by default); after one from `response`; before a Record's commit from `dataset` and `record`. A
        name the crawl does not hold raises `HookUnknownError`, naming the closest."""

    async def reconcile(self, run_id: str, key: str, answer: Literal["happened", "did_not_happen"]) -> None:
        """Stands for a person's answer about an uncertain Effect of a Run on this engine (ADR 0029): the key's next
        declaration returns `reconciled` after `happened`, and performs the body again after `did_not_happen`
        (ADR 0106). Only the testing kit has it."""

    async def close(self) -> None:
        """Closes the connection and the engine and removes the home. Raises `ScriptedModelUnusedError` if a Scripted model
        registered here has replies left."""

    async def __aenter__(self) -> AsyncTestEngine: ...
    async def __aexit__(
        self, exc_type: type[BaseException] | None, exc: BaseException | None, tb: TracebackType | None
    ) -> None: ...

@final
class TestEngine:
    """`AsyncTestEngine` for the sync `Foxhound` and `Crawl`, used with `with`."""

    __test__: ClassVar[bool] = False
    """pytest never collects it, as Starlette's `TestClient`."""

    @classmethod
    def start(
        cls,
        *,
        settings: Settings = ...,
        env: Mapping[str, str] = ...,
        project: str | PathLike[str] | None = None,
        model_providers: Mapping[str, ModelProviderHandler] | None = None,
    ) -> TestEngine: ...
    @property
    def connection(self) -> Foxhound: ...
    @overload
    def run_handler(
        self,
        crawl: Crawl,
        label: Label[None],
        *,
        url: str,
        response: SavedResponse | None = None,
        recording: str | PathLike[str] | None = None,
        tier: FetchTier | None = None,
        attempt: int = 1,
        depth: int = 0,
        effects: Mapping[str, PriorEffect] | None = None,
    ) -> AttemptResult: ...
    @overload
    def run_handler(
        self,
        crawl: Crawl,
        label: Label[P],
        *,
        url: str,
        payload: P,
        response: SavedResponse | None = None,
        recording: str | PathLike[str] | None = None,
        tier: FetchTier | None = None,
        attempt: int = 1,
        depth: int = 0,
        effects: Mapping[str, PriorEffect] | None = None,
    ) -> AttemptResult: ...
    def run_hook(
        self,
        crawl: Crawl,
        name: str,
        *,
        label: Label[Any],
        url: str,
        payload: JSONValue = None,
        method: str = "GET",
        body: str | bytes | None = None,
        headers: Mapping[str, str] | None = None,
        tier: FetchTier | None = None,
        previous_error: GoodfetchError | None = None,
        attempt: int = 1,
        depth: int = 0,
        response: SavedResponse | None = None,
        recording: str | PathLike[str] | None = None,
        dataset: Dataset[Any] | None = None,
        record: JSONValue = None,
    ) -> HookResult: ...
    def reconcile(self, run_id: str, key: str, answer: Literal["happened", "did_not_happen"]) -> None: ...
    def close(self) -> None: ...
    def __enter__(self) -> TestEngine: ...
    def __exit__(self, exc_type: type[BaseException] | None, exc: BaseException | None, tb: TracebackType | None) -> None: ...

@final
@dataclass(frozen=True, kw_only=True)
class HookUnknownDetails:
    name: str
    closest: str | None

@final
class HookUnknownError(ProgramError):
    """`run_hook` named a Hook the crawl does not hold."""

    @property
    def details(self) -> HookUnknownDetails: ...

