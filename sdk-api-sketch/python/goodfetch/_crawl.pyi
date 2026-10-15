"""Crawl definition and handler context (ADR 0093, decided in #477).

A crawl is declared by a `Crawl` (sync handlers) or an `AsyncCrawl` (`async def` handlers), made with no connection and no
I/O, so a module may declare it at import: each Label is declared first, returning its typed handle, then gets exactly one
handler through the decorator `@crawl.handle(label)`, which returns the function unchanged so a test can call it. A Run
carries the crawl out; its start and resume verbs are #478's, so `Foxhound.start` is provisional. Handlers run as ADR
0081 says: as tasks in the Run's task group (async), or on the connection's thread pool (sync).

A handler receives one context on every fetch tier: Collie's page over the request's document. Every tier but `browser`
serves what it can exactly; a call only a browser can serve, or a wait for something a static document lacks, ends the
attempt and runs the request again in the browser, unless `fetch.tier` forces the tier, when it raises
`BrowserRequiredError`.
"""

import re
from collections.abc import Awaitable, Callable, Sequence
from dataclasses import dataclass
from typing import Generic, Literal, NoReturn, TypeAlias, TypeVar, final, overload

from goodfetch._data import Dataset, JsonObject, JsonSchema, JsonValue, Label
from goodfetch._hooks import AsyncHooks, SyncHooks
from goodfetch._lifecycle import AsyncCrawlRun, AsyncPage, CrawlRun, Page
from goodfetch._page import AsyncLocator, AsyncResponse, Locator, Response
from goodfetch._results import PageError, ProgramError, RefusedError, UnavailableError
from goodfetch._settings import LabelSettings
from goodfetch._time import Timeout

P = TypeVar("P")
T = TypeVar("T")
Q = TypeVar("Q")
L = TypeVar("L")

UrlPattern: TypeAlias = str | re.Pattern[str]
"""A URL pattern matched in the engine: a glob, or a compiled pattern in the Rust `regex` syntax (ADR 0081). No predicate:
every link would cross the connection."""

FetchTier: TypeAlias = Literal["http", "discovered_api", "browser_fetch", "light_browser", "browser"]
"""The fetch tiers of ADR 0020. `fetch.tier` takes these or `auto`, the default, learned per Label on a domain."""

NotAddedReason: TypeAlias = (
    Literal["duplicate", "out_of_scope", "domain_policy", "max_depth", "max_requests", "unsupported_scheme", "no_link"] | str
)
"""Why a URL was not added. A newer engine's reason is kept as received (G3)."""

@final
@dataclass(frozen=True, kw_only=True)
class EnqueueAdded:
    id: str
    url: str

@final
@dataclass(frozen=True, kw_only=True)
class EnqueueNotAdded:
    url: str | None
    """`None` only for a matched element with no link (`no_link`)."""
    reason: NotAddedReason

Enqueued: TypeAlias = EnqueueAdded | EnqueueNotAdded
"""What an enqueue did: an answer, never an error (ADR 0079). From a handler, an addition is staged with the attempt and
committed with it (ADR 0003)."""

@final
@dataclass(frozen=True, kw_only=True)
class EnqueuedLinks:
    added: Sequence[EnqueueAdded]
    not_added: Sequence[EnqueueNotAdded]

@final
@dataclass(frozen=True, kw_only=True)
class Seed(Generic[P]):
    """A seed: a crawl request the program gives at the start, checked as an enqueue is. Provisional: the start verb is
    #478's."""

    url: str
    label: Label[P]
    payload: P

@overload
def seed(url: str, label: Label[None]) -> Seed[None]: ...
@overload
def seed(url: str, label: Label[P], payload: P) -> Seed[P]: ...

@final
class CrawlRequest(Generic[P]):
    """One crawl request, as its handler sees it. The engine's fields are kept apart from the Payload (ADR 0082)."""

    @property
    def id(self) -> str: ...
    @property
    def url(self) -> str:
        """The URL the Frontier admitted, after its URL rules (ADR 0005). The final URL is `page.url`."""
    @property
    def label(self) -> Label[P]:
        """The request's own Label, the handle to enqueue more of the same kind (pagination)."""
    @property
    def payload(self) -> P:
        """Parsed by the Label's schema."""
    @property
    def depth(self) -> int:
        """0 for a seed, one more for each enqueue from a handler."""
    @property
    def attempt(self) -> int:
        """1 for the first attempt. Escalations and sample-comparison runs are not retries and do not count."""

class _ContextBase(Generic[P]):
    @property
    def request(self) -> CrawlRequest[P]: ...
    @property
    def tier(self) -> FetchTier | str:
        """The tier that fetched this attempt; a newer engine's tier is kept as received."""
    def skip(self, message: str) -> NoReturn:
        """Ends the request `skipped`, with the reason `handler` and this message."""
    def retry(self, message: str) -> NoReturn:
        """Ends this attempt with `retry_requested`, retried under `crawl.retry.max`."""

@final
class AsyncCrawlContext(_ContextBase[P]):
    """What an `async def` handler receives, the same on every fetch tier. A context belongs to one attempt: once it has
    ended (`skip`, `retry`, an escalation, the handler's timeout, a lost lease, the Run's close), every call on it raises
    `AttemptEndedError`, and what the attempt staged is discarded."""

    @property
    def page(self) -> AsyncPage:
        """Collie's page (ADR 0085), with its AI verbs, fenced by the work item's lease and journaled."""
    @property
    def response(self) -> AsyncResponse:
        """The main document's final response, after redirects; its body is always kept."""
    @property
    def run(self) -> AsyncCrawlRun:
        """The crawl's Run, bound to this attempt (ADR 0093): `add_seed` and its Key-value store's writes through it are
        staged with the attempt, and its reads see them first. The program's own `run` value writes at once."""
    async def push(self, dataset: Dataset[T], record: T, *, timeout: Timeout = ...) -> None:
        """Stages a Record with this attempt (ADR 0103): it commits with the attempt, numbered in its Dataset, or is
        discarded if the attempt fails, retries, escalates, skips or loses its lease. Returns once staged. A Record
        failing the Dataset's schema, not JSON, or lacking its key's fields raises `RecordInvalidError` and is not
        staged; a Dataset of another crawl raises `DatasetUnknownError`."""
    @overload
    async def enqueue(
        self, url: str, label: Label[None], *, priority: int = 0, forefront: bool = False, key: str | None = None, timeout: Timeout = ...
    ) -> Enqueued: ...
    @overload
    async def enqueue(
        self, url: str, label: Label[Q], payload: Q, *, priority: int = 0, forefront: bool = False, key: str | None = None, timeout: Timeout = ...
    ) -> Enqueued: ...
    @overload
    async def enqueue_links(
        self,
        links: AsyncLocator,
        label: Label[None],
        *,
        include: Sequence[UrlPattern] = (),
        exclude: Sequence[UrlPattern] = (),
        limit: int | None = None,
        priority: int = 0,
        timeout: Timeout = ...,
    ) -> EnqueuedLinks: ...
    @overload
    async def enqueue_links(
        self,
        links: AsyncLocator,
        label: Label[Q],
        payload: Q,
        *,
        include: Sequence[UrlPattern] = (),
        exclude: Sequence[UrlPattern] = (),
        limit: int | None = None,
        priority: int = 0,
        timeout: Timeout = ...,
    ) -> EnqueuedLinks: ...

@final
class CrawlContext(_ContextBase[P]):
    """What a sync handler receives: `AsyncCrawlContext` without `await`."""

    @property
    def page(self) -> Page: ...
    @property
    def response(self) -> Response: ...
    @property
    def run(self) -> CrawlRun: ...
    def push(self, dataset: Dataset[T], record: T, *, timeout: Timeout = ...) -> None: ...
    @overload
    def enqueue(
        self, url: str, label: Label[None], *, priority: int = 0, forefront: bool = False, key: str | None = None, timeout: Timeout = ...
    ) -> Enqueued: ...
    @overload
    def enqueue(
        self, url: str, label: Label[Q], payload: Q, *, priority: int = 0, forefront: bool = False, key: str | None = None, timeout: Timeout = ...
    ) -> Enqueued: ...
    @overload
    def enqueue_links(
        self,
        links: Locator,
        label: Label[None],
        *,
        include: Sequence[UrlPattern] = (),
        exclude: Sequence[UrlPattern] = (),
        limit: int | None = None,
        priority: int = 0,
        timeout: Timeout = ...,
    ) -> EnqueuedLinks: ...
    @overload
    def enqueue_links(
        self,
        links: Locator,
        label: Label[Q],
        payload: Q,
        *,
        include: Sequence[UrlPattern] = (),
        exclude: Sequence[UrlPattern] = (),
        limit: int | None = None,
        priority: int = 0,
        timeout: Timeout = ...,
    ) -> EnqueuedLinks: ...

AsyncHandler: TypeAlias = Callable[[AsyncCrawlContext[P]], Awaitable[None]]
Handler: TypeAlias = Callable[[CrawlContext[P]], None]

class _Declares(Generic[L]):
    @overload
    def label(self, name: str, *, settings: LabelSettings = ..., login: L | None = None) -> Label[None]:
        """Declares a Label and returns its typed handle. A second Label of the same name raises `LabelDuplicateError`.
        `login` is the scripted login step of the Sessions its requests use (ADRs 0006, 0092); setting it gives the
        Label a Session pool of its own."""
    @overload
    def label(self, name: str, *, payload: type[P], settings: LabelSettings = ..., login: L | None = None) -> Label[P]:
        """A Pydantic model, or any class Pydantic's `TypeAdapter` accepts (ADR 0082)."""
    @overload
    def label(self, name: str, *, payload: JsonSchema, settings: LabelSettings = ..., login: L | None = None) -> Label[JsonValue]:
        """A JSON Schema written by hand: the way down."""
    @overload
    def dataset(self, name: str, *, key: Sequence[str] = ()) -> Dataset[JsonObject]:
        """Declares a Dataset and returns its typed handle (ADR 0103): there is no default Dataset, so every push names
        one. A second Dataset of the same name raises `DatasetDuplicateError`. `key` lists the top-level fields that
        identify a Record, which Sinks' keyed write modes and Baseline comparisons use; one the schema does not require
        raises `DatasetKeyInvalidError`."""
    @overload
    def dataset(self, name: str, *, schema: type[T], key: Sequence[str] = ()) -> Dataset[T]:
        """A Pydantic model, or any class Pydantic's `TypeAdapter` accepts (ADR 0082)."""
    @overload
    def dataset(self, name: str, *, schema: JsonSchema, key: Sequence[str] = ()) -> Dataset[JsonValue]: ...

@final
class AsyncCrawl(_Declares[Callable[[AsyncPage], Awaitable[None]]], AsyncHooks):
    """A crawl's declaration, for `AsyncGoodfetch`: its Labels, their `async def` handlers and its Hooks in code (ADR 0104)."""

    def __init__(self, *, login: Callable[[AsyncPage], Awaitable[None]] | None = None) -> None:
        """`login` is the scripted login step of the Run's Session pool (ADRs 0006, 0092); a Label's own wins."""
    def handle(self, label: Label[P]) -> Callable[[AsyncHandler[P]], AsyncHandler[P]]:
        """Attaches the Label's one handler and returns it unchanged. A second one raises `LabelDuplicateError`; a Label
        left without one makes the Run's start raise `LabelUnhandledError`."""

@final
class Crawl(_Declares[Callable[[Page], None]], SyncHooks):
    """A crawl's declaration, for `Goodfetch` (sync): its Labels, their plain-function handlers and its Hooks in code
    (ADR 0104)."""

    def __init__(self, *, login: Callable[[Page], None] | None = None) -> None: ...
    def handle(self, label: Label[P]) -> Callable[[Handler[P]], Handler[P]]: ...

# The crawl's Error codes (ADR 0093).

@final
class LabelDuplicateError(ProgramError):
    """A second Label of one name, or a second handler for one Label. Raised at once, with no I/O."""

@final
class LabelUnhandledError(ProgramError):
    """The Run's start found a Label with no handler."""

@final
class LabelUnknownError(ProgramError):
    """An enqueue named a Label this crawl does not declare, or a resume found pending requests of one it no longer
    declares; nothing ran."""

AttemptEndReason: TypeAlias = Literal["skipped", "retry", "escalated", "timeout", "lease_lost", "run_closed"] | str

@final
class AttemptEndedError(BaseException):
    """Ends a handler's attempt: raised by `skip()`, `retry()`, a call that escalates, and every call on the context of an
    attempt that has already ended. Like `asyncio.CancelledError`, it derives from `BaseException`, so `except
    Exception:` lets it through; the SDK ends the handler with it, and catching it changes nothing, since the decision
    stands. It still carries its Error code, `attempt_ended`, in the `program` group (ADR 0079)."""

    @property
    def code(self) -> Literal["attempt_ended"]: ...
    @property
    def group(self) -> Literal["program"]: ...
    @property
    def reason(self) -> AttemptEndReason: ...

@final
class BrowserRequiredError(ProgramError):
    """A call only a browser can serve while `fetch.tier` forces a tier without one; `limit` names that setting and its
    Label."""

@final
class RetryRequestedError(PageError):
    """An attempt the handler ended with `ctx.retry()`; the request's error once retries run out."""

@final
class ResponseNotOkError(PageError):
    """A final 4xx that is no Block cause and not in `crawl.handle_statuses`. Not retryable."""

@final
class ResponseUnavailableError(PageError):
    """A final 5xx or 408 after the request's retries, not in `crawl.handle_statuses`: the site is failing for now.
    Retryable."""

@final
class HandlerLostError(UnavailableError):
    """The program stopped answering for this attempt with no word: its process ended or its connection dropped.
    Retryable, and counted against `crawl.retry.max`. The program's own stop (`close()`, `KeyboardInterrupt`) counts
    nothing."""

@final
class SitePreferencesRefusedError(RefusedError):
    """A handler's own navigation or fetch that Site preferences refuse, refused before it is sent: a crawl request's
    reasons, `robots_txt`, `crawl_delay`, or `use_signals` while `site_preferences.respect_use_signals` is on (ADR 0086).
    If it escapes the handler, the request ends `skipped` with the same reason."""
