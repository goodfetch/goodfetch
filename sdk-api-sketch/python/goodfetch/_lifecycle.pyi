"""Entry points and lifecycle (ADR 0078, decided in #465).

Logs go to `logging` loggers under `goodfetch` (`goodfetch.client`, `goodfetch.engine.<area>`); the SDK adds only a
`NullHandler`. The Run's start verbs are a placeholder owned by #478; Sessions, logins and secrets are ADR 0092's
(`_sessions`); the page API is ADR 0085's (`_page`).
Timeouts, waits and Limits come from #467 (`_time`); events and user code from #468 (`_concurrency`).
"""

from collections.abc import Awaitable, Callable, Generator, Mapping, Sequence
from os import PathLike
from dataclasses import dataclass
from types import TracebackType
from typing import Any, Literal, Never, Protocol, Self, TypeAlias, TypeVar, final, overload

from goodfetch._agent import AgentApi, AsyncAgentApi
from goodfetch._recording import Recording
from goodfetch._ai import AiVerbs, AsyncAiVerbs, AsyncElementReferences, ElementReferences
from goodfetch._catalog import AsyncRawCall, RawCall
from goodfetch._concurrency import (
    AsyncEventIterator,
    AsyncListener,
    AsyncPageEvents,
    AsyncRunEvents,
    EventIterator,
    Listener,
    PageEvents,
    RunEvents,
    Subscription,
)
from goodfetch._data import JsonValue
from goodfetch._effects import AsyncDeclaresEffects, DeclaresEffects
from goodfetch._models import AsyncModelProviderHandler, ModelProviderHandler, RunUsage
from goodfetch._output import AsyncExport, AsyncKeyValueStore, DatasetRecord, Export, ExportFormat, KeyValueStore, RejectedRecord, StoredBytes
from goodfetch._hooks import (
    AsyncFetchBackendHandler,
    AsyncProxyProviderHandler,
    AsyncSinkHandler,
    FetchBackendHandler,
    ProxyProviderHandler,
    SinkHandler,
)
from goodfetch._page import AsyncPageApi, AsyncRouteHandler, PageApi, RouteHandler
from goodfetch._sessions import (
    AsyncIdentities,
    AsyncPageStorage,
    AsyncPausesForHuman,
    AsyncResolveSecret,
    AsyncSessionStateApi,
    Identities,
    PageStorage,
    PausesForHuman,
    ResolveSecret,
    SessionStateApi,
    SessionSource,
)
from goodfetch._crawl import AsyncCrawl, Crawl, Enqueued, Seed
from goodfetch._data import Dataset, Label
from goodfetch._results import CrawlRequestOutcome
from goodfetch._run import AsyncRunControl, AsyncRunView, CrawlRunProgress, CrawlRunReport, RunControl, RunProgress, RunReport, RunView
from goodfetch._settings import CrawlRunSettings, ExplainedSetting, PageSettings, RunSettings, SessionSettings, SettingKey, Settings
from goodfetch._time import AsyncPageWaits, Limits, NotGiven, PageWaits, Timeout, UrlMatcher

_P = TypeVar("_P")
_T = TypeVar("_T")

@final
@dataclass(frozen=True, kw_only=True)
class EngineAddress:
    """An Exposed Daemon or Goodfetch Cloud, reached by address and token."""

    url: str
    token: str

EngineChoice: TypeAlias = Literal["auto", "attached", "daemon"] | EngineAddress
"""`auto` follows ADR 0024: a Daemon when one is configured or running, otherwise an Attached engine."""

@final
@dataclass(frozen=True, kw_only=True)
class EngineInfo:
    """The engine a connection reached, reported for diagnosis (G6)."""

    kind: Literal["attached", "daemon", "remote"]
    version: str
    protocol_version: str
    project_file: str | None
    """The project configuration file read, or `None` when none was found or `project` was `None` (ADR 0083)."""
    home: str | None
    """The Goodfetch home the engine uses (`GOODFETCH_HOME`), or `None` for an Exposed Daemon or Goodfetch Cloud."""
    engine_path: str | None
    """The engine binary started (`GOODFETCH_ENGINE_PATH`), or `None` when none was started."""
    url: str | None
    """The address reached (`GOODFETCH_URL` or `engine`), or `None` for an engine on this machine. `GOODFETCH_TOKEN` never
    appears."""

# The user's own Playwright objects, matched by shape so no third-party type is required (G7).

class _CDPContext(Protocol):
    def new_cdp_session(self, page: Never, /) -> object: ...

class _AsyncCDPContext(Protocol):
    async def new_cdp_session(self, page: Never, /) -> object: ...

class BorrowablePage(Protocol):
    """A page of Playwright's sync API."""

    @property
    def context(self) -> _CDPContext: ...

class BorrowableAsyncPage(Protocol):
    """A page of Playwright's async API."""

    @property
    def context(self) -> _AsyncCDPContext: ...

class BorrowableBrowser(Protocol):
    """A browser of Playwright's sync API."""

    def new_browser_cdp_session(self) -> object: ...

class BorrowableAsyncBrowser(Protocol):
    """A browser of Playwright's async API."""

    async def new_browser_cdp_session(self) -> object: ...

# Async API.

class _AsyncExplainsSettings:
    async def explain_settings(self, keys: Sequence[SettingKey] = ..., *, timeout: Timeout = ...) -> list[ExplainedSetting]:
        """The scope's effective settings, with the layer that set each one; all of them when `keys` is left out."""

class _AsyncCloseable:
    async def close(self, *, timeout: Timeout = ...) -> None:
        """Idempotent. Its timeout is the close phase's. Closes children first; any later call on this value fails with the Error code `closed`."""

    async def __aenter__(self) -> Self: ...
    async def __aexit__(
        self, exc_type: type[BaseException] | None, exc: BaseException | None, tb: TracebackType | None
    ) -> None: ...

@final
class AsyncPage(
    _AsyncCloseable,
    _AsyncExplainsSettings,
    AsyncPageApi,
    AsyncPageWaits,
    AsyncPageEvents,
    AsyncPageStorage,
    AsyncPausesForHuman,
    AsyncAiVerbs,
    AsyncElementReferences,
    AsyncRawCall,
    AsyncAgentApi,
):
    """A page Goodfetch drives: a tab of a Session, opened by `session.new_page()` or by the page itself (a popup).
    Closing it closes only a tab Goodfetch opened; on a Borrowed browser it only releases it."""

    async def close(self, *, run_before_unload: bool = False, timeout: Timeout = ...) -> None:
        """Idempotent. `run_before_unload` runs the page's `beforeunload` handlers, which closing otherwise skips."""

@final
class AsyncSession(_AsyncCloseable, _AsyncExplainsSettings, AsyncSessionStateApi, AsyncRawCall):
    """A Session of a Run: Playwright's context, under the glossary's name (ADR 0085), with its cookies and login
    (ADR 0092). Closing it never closes a Borrowed browser. There is no active page."""

    async def new_page(self, *, settings: PageSettings = ..., timeout: Timeout = ...) -> AsyncPage: ...
    def pages(self) -> list[AsyncPage]:
        """The Session's open pages, read without a round trip."""

    def on(self, event: Literal["page"], listener: AsyncListener[AsyncPage]) -> Subscription:
        """A new page in the Session: one it opened, a popup, or a tab a link opened."""

    def route(self, url: UrlMatcher, handler: AsyncRouteHandler) -> Subscription:
        """Answers requests of every page of the Session, a popup's first request included; page routes run first."""

    def add_init_script(self, script: str, arg: JsonValue | NotGiven = ...) -> Subscription: ...

@final
class AsyncRun(_AsyncCloseable, _AsyncExplainsSettings, AsyncRunEvents, AsyncRunControl[RunReport, RunProgress], AsyncRawCall, AsyncDeclaresEffects):
    """A Run, always opened explicitly. `finish()` ends it `finished`; closing an unfinished Run stops it with a
    Checkpoint, resumable by starting its ID again (ADR 0102)."""

    @property
    def id(self) -> str:
        """Chosen by the program at the start, or generated: it names this Run for good."""
    async def usage(self, *, timeout: Timeout = ...) -> RunUsage:
        """The Run's Usage so far, priced, with what remains of each Limit and the uncapped mark (ADR 0101)."""
    @property
    def key_value_store(self) -> AsyncKeyValueStore:
        """The Run's Key-value store, Collie's included (ADR 0103). Writes commit at once, or, through a handler's
        `ctx.run`, are staged with its attempt."""
    async def new_page(self, *, settings: PageSettings = ..., timeout: Timeout = ...) -> AsyncPage:
        """Opens a page in the Run's default Session, which the first call opens with the Run's settings and every later
        call shares; `page.session` reaches it. Once it is closed, this raises `ClosedError`."""
    async def new_session(
        self,
        *,
        source: SessionSource = ...,
        login: Callable[[AsyncPage], Awaitable[None]] | None = None,
        settings: SessionSettings = ...,
        timeout: Timeout = ...,
    ) -> AsyncSession:
        """Opens a Session of its own (ADR 0092).

        `source` is `session.source` for this Session alone (ADR 0056); left out, the Run's or the connection's. `login`
        is ADR 0006's scripted login step: called with a page of this Session on its login page, on the program's own
        loop (ADR 0081); the Page check judges the page it leaves, it counts toward the two recoveries a page gets, and
        an exception is chained to `LoggedOutError`.
        """
    @overload
    async def borrow(self, page: BorrowableAsyncPage, /) -> AsyncPage: ...
    @overload
    async def borrow(self, browser: BorrowableAsyncBrowser, /) -> AsyncSession: ...

@final
class AsyncCollie:
    """Collie, reached from the connection without I/O."""

    async def start(
        self,
        *,
        id: str | None = None,
        settings: RunSettings = ...,
        timeout: Timeout = ...,
        limits: Limits | None = None,
        plan: str | PathLike[str] | None = None,
        recording: Recording | None = None,
    ) -> AsyncRun:
        """Starts a Run and returns once it is started (ADR 0102).

        `id` names the Run for good in the Run index; left out, the engine generates one. A start whose `id` names an
        existing Run with the same inputs resumes it if interrupted (`started_as` is `resumed`) or returns it as it is if
        it finished (`ended`); it raises `RunInUseError` if the Run is live, `RunConflictError` for other inputs,
        `LimitReachedError` if a used-up Limit stopped it, and `RunStateUnavailableError` if its state is gone.

        `timeout` caps the whole Run. Omitted: the setting's value, which has no built-in default; `None`: no timeout.
        On expiry the Run stops with a Checkpoint, resumable; a resume's clock starts afresh. On a resume, `limits` are
        compared with what the Run began with: equal leaves each Limit as it stands, lower lowers it, higher raises
        `LimitRaiseRefusedError` (ADR 0041); a changed setting applies from then on, and one fixed for a Run's life, such
        as `crawl.scope.*`, raises `SettingNotApplicableError`.

        `plan` is the Run's Plan file, relative to the working directory: an input of the Run (ADR 0091), read here and
        written back here when a new version is promoted. Left out, the Plan lives only in the Run trace.

        `recording` is the Run's Response recording (ADR 0105), an input like `plan`, read and written here: `replay`
        answers every request from it and sends none, `record` writes what the Run received when it ends.
        """
    async def open_run(self, id: str, *, timeout: Timeout = ...) -> AsyncRunView[RunReport, RunProgress]:
        """Opens a Run by its ID to read it, never to drive it: a detached Run, another program's, or one that ended.
        Raises `RunNotFoundError`."""

@final
class AsyncCrawlRun(_AsyncCloseable, _AsyncExplainsSettings, AsyncRunEvents, AsyncRunControl[CrawlRunReport, CrawlRunProgress], AsyncDeclaresEffects):
    """A crawl's Run (ADRs 0093, 0102); its Records are ADR 0103's. Its handlers run from its start until it ends."""

    @property
    def id(self) -> str: ...
    @property
    def seeds(self) -> Sequence[Enqueued]:
        """Each seed's answer, given once at the start: added with its ID, or not added with its reason. On a resume, the
        answers given when the Run began."""
    def outcomes(
        self, *, kinds: Sequence[Literal["succeeded", "failed", "skipped", "blocked"]] | None = None, labels: Sequence[Label[Any]] | None = None
    ) -> AsyncEventIterator[CrawlRequestOutcome]:
        """The crawl requests' outcomes, read from the Run's committed state in commit order from its first, then following
        new commits until the Run ends; the filters run in the engine (ADR 0081). A reader of committed state, it never
        slows the Run. A request `retry_failed` re-admits ends again, and its new outcome supersedes the old: key
        outcomes by `request.id`."""
    async def retry_failed(self, *, blocked: bool = False, timeout: Timeout = ...) -> int:
        """Re-admits, in this same Run, every request that ended `failed`, and with `blocked=True` those that ended
        `blocked`, each keeping its ID, Label, Payload and depth, its attempts counting on with a fresh `crawl.retry.max`.
        A skip is a decision and is never re-admitted. On an ended Run it returns the Run to `running`, its Verdict and
        Completeness given anew when it ends (ADR 0029). Returns how many it re-admitted."""
    async def usage(self, *, timeout: Timeout = ...) -> RunUsage:
        """The Run's Usage so far, priced, with what remains of each Limit and the uncapped mark (ADR 0101)."""
    @property
    def key_value_store(self) -> AsyncKeyValueStore:
        """Writes commit at once, or, through a handler's `ctx.run`, are staged with its attempt."""
    def records(self, dataset: Dataset[_T], *, after: int = 0) -> AsyncEventIterator[DatasetRecord[_T]]:
        """The Dataset's committed Records, from its first (or after `after`), following the Run live until it ends and
        every Record is read (ADR 0103). A cursor over run state: reading slowly, or not at all, never slows the crawl.
        One the native parse rejects ends the loop with `RecordInvalidError` carrying its number. Through a handler's
        `ctx.run`, it reads what has committed and ends, since the Run cannot end while the handler holds its request."""
    def rejected_records(self, dataset: Dataset[Any], *, after: int = 0) -> AsyncEventIterator[RejectedRecord]:
        """The Dataset's rejection table, each entry numbered in commit order, following the Run as `records` does."""
    async def export(
        self, dataset: Dataset[Any], *, format: ExportFormat, compression: Literal["gzip", "zstd"] | None = None, timeout: Timeout = ...
    ) -> AsyncExport:
        """Writes an Export of the Records committed so far (ADR 0026), to save or stream wherever the engine runs."""
    @overload
    async def add_seed(
        self, url: str, label: Label[None], *, priority: int = 0, forefront: bool = False, key: str | None = None, timeout: Timeout = ...
    ) -> Enqueued: ...
    @overload
    async def add_seed(
        self,
        url: str,
        label: Label[_P],
        payload: _P,
        *,
        priority: int = 0,
        forefront: bool = False,
        key: str | None = None,
        timeout: Timeout = ...,
    ) -> Enqueued:
        """Adds a seed to the running crawl (ADR 0037's CO-03), committed at once, or staged with the attempt when called
        through a handler's `ctx.run`. A seed widens the scope by its host, where a handler's `enqueue` is checked against
        it; one the domain policy refuses is not added (`domain_policy`)."""

class AsyncKeyValueStoreReader:
    """A Key-value store's reads, for a Run view: `AsyncKeyValueStore` without its writes."""

    @overload
    async def get(self, key: str, *, timeout: Timeout = ...) -> JsonValue | None: ...
    @overload
    async def get(self, key: str, schema: type[_T], *, timeout: Timeout = ...) -> _T | None: ...
    async def get_bytes(self, key: str, *, timeout: Timeout = ...) -> StoredBytes | None: ...
    def keys(self, *, prefix: str = "") -> AsyncEventIterator[str]: ...

@final
class AsyncCrawlRunView(AsyncRunView[CrawlRunReport, CrawlRunProgress]):
    """A crawl's Run read by a program that does not drive it, opened with `open_crawl_run` (ADR 0102). Its Records are
    read with the program's own Dataset handles."""

    def outcomes(
        self, *, kinds: Sequence[Literal["succeeded", "failed", "skipped", "blocked"]] | None = None, labels: Sequence[Label[Any]] | None = None
    ) -> AsyncEventIterator[CrawlRequestOutcome]: ...
    def records(self, dataset: Dataset[_T], *, after: int = 0) -> AsyncEventIterator[DatasetRecord[_T]]: ...
    def rejected_records(self, dataset: Dataset[Any], *, after: int = 0) -> AsyncEventIterator[RejectedRecord]: ...
    async def export(
        self, dataset: Dataset[Any], *, format: ExportFormat, compression: Literal["gzip", "zstd"] | None = None, timeout: Timeout = ...
    ) -> AsyncExport: ...
    @property
    def key_value_store(self) -> AsyncKeyValueStoreReader: ...

@final
class AsyncFoxhound:
    """Foxhound, reached from the connection without I/O."""

    async def start_crawl(
        self,
        crawl: AsyncCrawl,
        *,
        seeds: Sequence[Seed[Any]],
        id: str | None = None,
        settings: CrawlRunSettings = ...,
        timeout: Timeout = ...,
        limits: Limits | None = None,
        plan: str | PathLike[str] | None = None,
        recording: Recording | None = None,
    ) -> AsyncCrawlRun:
        """Starts a Run of the crawl from its seeds, which define its scope, and returns once it is started; its handlers
        run from then on (ADR 0102, Firecrawl's `start_crawl`). An `id` naming an existing Run behaves as
        `AsyncCollie.start`'s does, the seeds and the Plan file being inputs that must match. Raises `LabelUnhandledError`
        when a Label has no handler, and `LabelUnknownError` when a resumed Run has pending requests of a Label no longer
        declared (ADR 0093). A Foxhound connection also carries Collie's `start` (ADR 0095)."""
    async def open_crawl_run(self, id: str, *, timeout: Timeout = ...) -> AsyncCrawlRunView:
        """Opens a crawl's Run by its ID to read it, as `AsyncCollie.open_run` does, with its outcomes, Records and
        Exports."""

@final
class _AsyncConnecting:
    """What `AsyncGoodfetch.connect()` returns: awaited, or used with `async with`."""

    def __await__(self) -> Generator[None, None, AsyncGoodfetch]: ...
    async def __aenter__(self) -> AsyncGoodfetch: ...
    async def __aexit__(
        self, exc_type: type[BaseException] | None, exc: BaseException | None, tb: TracebackType | None
    ) -> None: ...

class AsyncConnection(_AsyncCloseable, _AsyncExplainsSettings, AsyncRawCall):
    """Any connection: what `AsyncCollie` and `AsyncFoxhound` share, declared once in `goodfetch_collie_client`
    (ADRs 0095, 0099), so a library typed for it accepts either Product's connection. Collie's Run start joins it once
    #478 names it. The sketch's placeholder `AsyncGoodfetch` extends it until #483 replaces it."""

    @property
    def engine(self) -> EngineInfo:
        """The engine reached, fixed when the connection opened."""
    @property
    def identities(self) -> AsyncIdentities:
        """The install's Saved identities (ADR 0092)."""

@final
class AsyncGoodfetch(AsyncConnection):
    """The engine connection: the entry point. Each one is independent, and many may be open in one process.

    It runs on AnyIO, under asyncio or Trio. Its values belong to the event loop that opened it; a call from another
    loop or thread fails with a `ProgramError` naming `Goodfetch` (ADR 0081).
    """

    @classmethod
    def connect(
        cls,
        *,
        engine: EngineChoice = "auto",
        settings: Settings = ...,
        env: Mapping[str, str] = ...,
        project: str | PathLike[str] | None | NotGiven = ...,
        timeout: Timeout = ...,
        limits: Limits | None = None,
        resolve_secret: AsyncResolveSecret | None = None,
        model_providers: Mapping[str, AsyncModelProviderHandler] | None = None,
        fetch_backends: Mapping[str, AsyncFetchBackendHandler] | None = None,
        proxy_providers: Mapping[str, AsyncProxyProviderHandler] | None = None,
        sinks: Mapping[str, AsyncSinkHandler] | None = None,
    ) -> _AsyncConnecting:
        """Starts or finds the engine and completes `hello` before the connection is returned.

        Left at `auto`, `engine` is filled by `GOODFETCH_URL` when set; a value written here wins over it. `settings` holds
        every other setting for this connection and everything beneath it. `env` is the environment map, read once here
        and never at import, by default the process's own (ADRs 0062, 0069): only the deployment variables are read, an
        unknown `GOODFETCH_` name fails with `SettingUnknownError`, and an Attached engine gets exactly this map.
        `project` is the project configuration file: left out, found upward from the working directory as the CLI does;
        `None`, none (ADR 0083). `resolve_secret` answers `caller:` Secret references from the program's own store, at
        the moment of use (ADR 0092). `model_providers` are Model providers answered by the program's own code, by the
        name Model references use: `"mine/any-model"` for `{"mine": handler}` (ADR 0101).
        `fetch_backends`, `proxy_providers` and `sinks` are the program's own Plugins of those kinds, SDK handlers by
        the name settings choose them by (ADR 0104): the only Fetch backend or Proxy provider stands in for its
        choosing setting's default, which any explicit value replaces, and a Sink handler serves the `sinks.<name>`
        entry of its name.
        """

    @property
    def engine(self) -> EngineInfo: ...
    @property
    def collie(self) -> AsyncCollie: ...
    @property
    def foxhound(self) -> AsyncFoxhound: ...
    @property
    def identities(self) -> AsyncIdentities:
        """The install's Saved identities (ADR 0092)."""

# Sync API: the same concepts, generated from the same catalog (ADR 0038).

class _ExplainsSettings:
    def explain_settings(self, keys: Sequence[SettingKey] = ..., *, timeout: Timeout = ...) -> list[ExplainedSetting]:
        """The scope's effective settings, with the layer that set each one; all of them when `keys` is left out."""

class _Closeable:
    def close(self, *, timeout: Timeout = ...) -> None:
        """Idempotent. Its timeout is the close phase's. Closes children first; any later call on this value fails with the Error code `closed`."""

    def __enter__(self) -> Self: ...
    def __exit__(
        self, exc_type: type[BaseException] | None, exc: BaseException | None, tb: TracebackType | None
    ) -> None: ...

@final
class Page(_Closeable, _ExplainsSettings, PageApi, PageWaits, PageEvents, PageStorage, PausesForHuman, AiVerbs, ElementReferences, RawCall, AgentApi):
    """A page Goodfetch drives: a tab of a Session, opened by `session.new_page()` or by the page itself (a popup).
    Closing it closes only a tab Goodfetch opened; on a Borrowed browser it only releases it."""

    def close(self, *, run_before_unload: bool = False, timeout: Timeout = ...) -> None: ...

@final
class Session(_Closeable, _ExplainsSettings, SessionStateApi, RawCall):
    """A Session of a Run: Playwright's context, under the glossary's name (ADR 0085), with its cookies and login
    (ADR 0092). Closing it never closes a Borrowed browser. There is no active page."""

    def new_page(self, *, settings: PageSettings = ..., timeout: Timeout = ...) -> Page: ...
    def pages(self) -> list[Page]: ...
    def on(self, event: Literal["page"], listener: Listener[Page]) -> Subscription: ...
    def route(self, url: UrlMatcher, handler: RouteHandler) -> Subscription: ...
    def add_init_script(self, script: str, arg: JsonValue | NotGiven = ...) -> Subscription: ...

@final
class Run(_Closeable, _ExplainsSettings, RunEvents, RunControl[RunReport, RunProgress], RawCall, DeclaresEffects):
    """A Run, always opened explicitly. `finish()` ends it `finished`; closing an unfinished Run stops it with a
    Checkpoint, resumable by starting its ID again (ADR 0102)."""

    @property
    def id(self) -> str: ...
    def usage(self, *, timeout: Timeout = ...) -> RunUsage:
        """The Run's Usage so far, priced, with what remains of each Limit and the uncapped mark (ADR 0101)."""
    @property
    def key_value_store(self) -> KeyValueStore: ...
    def new_page(self, *, settings: PageSettings = ..., timeout: Timeout = ...) -> Page:
        """Opens a page in the Run's default Session, shared by every later call; `page.session` reaches it."""
    def new_session(
        self,
        *,
        source: SessionSource = ...,
        login: Callable[[Page], None] | None = None,
        settings: SessionSettings = ...,
        timeout: Timeout = ...,
    ) -> Session:
        """Opens a Session of its own; `login` runs on the connection's thread pool (ADR 0081)."""
    @overload
    def borrow(self, page: BorrowablePage, /) -> Page: ...
    @overload
    def borrow(self, browser: BorrowableBrowser, /) -> Session: ...

@final
class Collie:
    """Collie, reached from the connection without I/O."""

    def start(
        self,
        *,
        id: str | None = None,
        settings: RunSettings = ...,
        timeout: Timeout = ...,
        limits: Limits | None = None,
        plan: str | PathLike[str] | None = None,
        recording: Recording | None = None,
    ) -> Run:
        """Starts a Run and returns once it is started (ADR 0102).

        `id` names the Run for good in the Run index; left out, the engine generates one. A start whose `id` names an
        existing Run with the same inputs resumes it if interrupted (`started_as` is `resumed`) or returns it as it is if
        it finished (`ended`); it raises `RunInUseError` if the Run is live, `RunConflictError` for other inputs,
        `LimitReachedError` if a used-up Limit stopped it, and `RunStateUnavailableError` if its state is gone.

        `timeout` caps the whole Run. Omitted: the setting's value, which has no built-in default; `None`: no timeout.
        On expiry the Run stops with a Checkpoint, resumable; a resume's clock starts afresh. On a resume, `limits` are
        compared with what the Run began with: equal leaves each Limit as it stands, lower lowers it, higher raises
        `LimitRaiseRefusedError` (ADR 0041); a changed setting applies from then on, and one fixed for a Run's life, such
        as `crawl.scope.*`, raises `SettingNotApplicableError`.

        `plan` is the Run's Plan file, relative to the working directory: an input of the Run (ADR 0091), read here and
        written back here when a new version is promoted. Left out, the Plan lives only in the Run trace.
        """
    def open_run(self, id: str, *, timeout: Timeout = ...) -> RunView[RunReport, RunProgress]: ...

@final
class CrawlRun(_Closeable, _ExplainsSettings, RunEvents, RunControl[CrawlRunReport, CrawlRunProgress], DeclaresEffects):
    """A crawl's Run (ADRs 0093, 0102); its Records are ADR 0103's. Its handlers run on the connection's thread pool from
    its start until it ends."""

    @property
    def id(self) -> str: ...
    @property
    def seeds(self) -> Sequence[Enqueued]:
        """Each seed's answer, given once at the start: added with its ID, or not added with its reason. On a resume, the
        answers given when the Run began."""
    def outcomes(
        self, *, kinds: Sequence[Literal["succeeded", "failed", "skipped", "blocked"]] | None = None, labels: Sequence[Label[Any]] | None = None
    ) -> EventIterator[CrawlRequestOutcome]: ...
    def retry_failed(self, *, blocked: bool = False, timeout: Timeout = ...) -> int: ...
    def usage(self, *, timeout: Timeout = ...) -> RunUsage:
        """The Run's Usage so far, priced, with what remains of each Limit and the uncapped mark (ADR 0101)."""
    @property
    def key_value_store(self) -> KeyValueStore: ...
    def records(self, dataset: Dataset[_T], *, after: int = 0) -> EventIterator[DatasetRecord[_T]]: ...
    def rejected_records(self, dataset: Dataset[Any], *, after: int = 0) -> EventIterator[RejectedRecord]: ...
    def export(
        self, dataset: Dataset[Any], *, format: ExportFormat, compression: Literal["gzip", "zstd"] | None = None, timeout: Timeout = ...
    ) -> Export: ...
    @overload
    def add_seed(
        self, url: str, label: Label[None], *, priority: int = 0, forefront: bool = False, key: str | None = None, timeout: Timeout = ...
    ) -> Enqueued: ...
    @overload
    def add_seed(
        self,
        url: str,
        label: Label[_P],
        payload: _P,
        *,
        priority: int = 0,
        forefront: bool = False,
        key: str | None = None,
        timeout: Timeout = ...,
    ) -> Enqueued: ...

class KeyValueStoreReader:
    """`AsyncKeyValueStoreReader` in the sync API."""

    @overload
    def get(self, key: str, *, timeout: Timeout = ...) -> JsonValue | None: ...
    @overload
    def get(self, key: str, schema: type[_T], *, timeout: Timeout = ...) -> _T | None: ...
    def get_bytes(self, key: str, *, timeout: Timeout = ...) -> StoredBytes | None: ...
    def keys(self, *, prefix: str = "") -> EventIterator[str]: ...

@final
class CrawlRunView(RunView[CrawlRunReport, CrawlRunProgress]):
    """`AsyncCrawlRunView` in the sync API."""

    def outcomes(
        self, *, kinds: Sequence[Literal["succeeded", "failed", "skipped", "blocked"]] | None = None, labels: Sequence[Label[Any]] | None = None
    ) -> EventIterator[CrawlRequestOutcome]: ...
    def records(self, dataset: Dataset[_T], *, after: int = 0) -> EventIterator[DatasetRecord[_T]]: ...
    def rejected_records(self, dataset: Dataset[Any], *, after: int = 0) -> EventIterator[RejectedRecord]: ...
    def export(
        self, dataset: Dataset[Any], *, format: ExportFormat, compression: Literal["gzip", "zstd"] | None = None, timeout: Timeout = ...
    ) -> Export: ...
    @property
    def key_value_store(self) -> KeyValueStoreReader: ...

@final
class Foxhound:
    """Foxhound, reached from the connection without I/O."""

    def start_crawl(
        self,
        crawl: Crawl,
        *,
        seeds: Sequence[Seed[Any]],
        id: str | None = None,
        settings: CrawlRunSettings = ...,
        timeout: Timeout = ...,
        limits: Limits | None = None,
        plan: str | PathLike[str] | None = None,
        recording: Recording | None = None,
    ) -> CrawlRun:
        """`AsyncFoxhound.start_crawl` in the sync API."""
    def open_crawl_run(self, id: str, *, timeout: Timeout = ...) -> CrawlRunView: ...

class Connection(_Closeable, _ExplainsSettings, RawCall):
    """Any connection, as `AsyncConnection`: what `Collie` and `Foxhound` share."""

    @property
    def engine(self) -> EngineInfo: ...
    @property
    def identities(self) -> Identities: ...

@final
class Goodfetch(Connection):
    """The engine connection: the entry point. Each one is independent, and many may be open in one process.

    It runs the async core on a private background thread of its own, so it works on any thread, including one whose event
    loop is running (a notebook cell); its values are safe to call from any thread (ADR 0081).
    """

    @classmethod
    def connect(
        cls,
        *,
        engine: EngineChoice = "auto",
        settings: Settings = ...,
        env: Mapping[str, str] = ...,
        project: str | PathLike[str] | None | NotGiven = ...,
        timeout: Timeout = ...,
        limits: Limits | None = None,
        resolve_secret: ResolveSecret | None = None,
        model_providers: Mapping[str, ModelProviderHandler] | None = None,
        fetch_backends: Mapping[str, FetchBackendHandler] | None = None,
        proxy_providers: Mapping[str, ProxyProviderHandler] | None = None,
        sinks: Mapping[str, SinkHandler] | None = None,
    ) -> Goodfetch:
        """Starts or finds the engine and completes `hello` before returning; use it with `with`.

        Left at `auto`, `engine` is filled by `GOODFETCH_URL` when set; a value written here wins over it. `settings` holds
        every other setting for this connection and everything beneath it. `env` is the environment map, read once here
        and never at import, by default the process's own (ADRs 0062, 0069): only the deployment variables are read, an
        unknown `GOODFETCH_` name fails with `SettingUnknownError`, and an Attached engine gets exactly this map.
        `project` is the project configuration file: left out, found upward from the working directory as the CLI does;
        `None`, none (ADR 0083). `fetch_backends`, `proxy_providers` and `sinks` are the program's own Plugins, SDK
        handlers (ADR 0104), run on the connection's thread pool.
        """

    @property
    def engine(self) -> EngineInfo: ...
    @property
    def collie(self) -> Collie: ...
    @property
    def foxhound(self) -> Foxhound: ...
    @property
    def identities(self) -> Identities: ...
