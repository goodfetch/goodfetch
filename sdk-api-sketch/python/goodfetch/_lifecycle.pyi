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

from goodfetch._ai import AiVerbs, AsyncAiVerbs, AsyncElementReferences, ElementReferences
from goodfetch._concurrency import AsyncListener, AsyncPageEvents, AsyncRunEvents, Listener, PageEvents, RunEvents, Subscription
from goodfetch._data import JsonValue
from goodfetch._models import AsyncModelProviderHandler, ModelProviderHandler, RunUsage
from goodfetch._page import AsyncPageApi, AsyncRouteHandler, PageApi, RouteHandler
from goodfetch._sessions import (
    AsyncIdentities,
    AsyncPausesForHuman,
    AsyncResolveSecret,
    AsyncSessionStateApi,
    Identities,
    PausesForHuman,
    ResolveSecret,
    SessionStateApi,
    SessionSource,
)
from goodfetch._crawl import AsyncCrawl, Crawl, Enqueued, Seed
from goodfetch._data import Label
from goodfetch._settings import CrawlRunSettings, ExplainedSetting, PageSettings, RunSettings, SessionSettings, SettingKey, Settings
from goodfetch._time import AsyncPageWaits, Limits, NotGiven, PageWaits, Timeout, UrlMatcher

_P = TypeVar("_P")

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

class _CdpContext(Protocol):
    def new_cdp_session(self, page: Never, /) -> object: ...

class _AsyncCdpContext(Protocol):
    async def new_cdp_session(self, page: Never, /) -> object: ...

class BorrowablePage(Protocol):
    """A page of Playwright's sync API."""

    @property
    def context(self) -> _CdpContext: ...

class BorrowableAsyncPage(Protocol):
    """A page of Playwright's async API."""

    @property
    def context(self) -> _AsyncCdpContext: ...

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
    AsyncPausesForHuman,
    AsyncAiVerbs,
    AsyncElementReferences,
):
    """A page Goodfetch drives: a tab of a Session, opened by `session.new_page()` or by the page itself (a popup).
    Closing it closes only a tab Goodfetch opened; on a Borrowed browser it only releases it."""

    async def close(self, *, run_before_unload: bool = False, timeout: Timeout = ...) -> None:
        """Idempotent. `run_before_unload` runs the page's `beforeunload` handlers, which closing otherwise skips."""

@final
class AsyncSession(_AsyncCloseable, _AsyncExplainsSettings, AsyncSessionStateApi):
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
class AsyncRun(_AsyncCloseable, _AsyncExplainsSettings, AsyncRunEvents):
    """A Run, always opened explicitly. Closing an unfinished Run stops it with a Checkpoint, resumable by its ID."""

    @property
    def id(self) -> str: ...
    async def usage(self, *, timeout: Timeout = ...) -> RunUsage:
        """The Run's Usage so far, priced, with what remains of each Limit and the uncapped mark (ADR 0101)."""
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
        settings: RunSettings = ...,
        timeout: Timeout = ...,
        limits: Limits | None = None,
        plan: str | PathLike[str] | None = None,
    ) -> AsyncRun:
        """Placeholder start verb: its spelling is #478's; `timeout` and `limits` are #467's, `settings` #470's.

        `timeout` caps the whole Run. Omitted: the setting's value, which has no built-in default; `None`: no timeout.
        On expiry the Run stops with a Checkpoint, resumable.

        `plan` is the Run's Plan file, relative to the working directory: an input of the Run (ADR 0091), read here and
        written back here when a new version is promoted. Left out, the Plan lives only in the Run trace.
        """

@final
class AsyncCrawlRun(_AsyncCloseable, _AsyncExplainsSettings, AsyncRunEvents):
    """A crawl's Run (ADR 0093). Its outcomes, progress and resume are #478's."""

    @property
    def id(self) -> str: ...
    async def usage(self, *, timeout: Timeout = ...) -> RunUsage:
        """The Run's Usage so far, priced, with what remains of each Limit and the uncapped mark (ADR 0101)."""
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

@final
class AsyncFoxhound:
    """Foxhound, reached from the connection without I/O."""

    async def start(
        self,
        crawl: AsyncCrawl,
        *,
        seeds: Sequence[Seed[Any]],
        settings: CrawlRunSettings = ...,
        timeout: Timeout = ...,
        limits: Limits | None = None,
    ) -> AsyncCrawlRun:
        """Starts a Run of the crawl from its seeds, which define its scope. Provisional: the start and resume verbs are
        #478's (ADR 0093). Raises `LabelUnhandledError` when a Label has no handler."""

@final
class _AsyncConnecting:
    """What `AsyncGoodfetch.connect()` returns: awaited, or used with `async with`."""

    def __await__(self) -> Generator[None, None, AsyncGoodfetch]: ...
    async def __aenter__(self) -> AsyncGoodfetch: ...
    async def __aexit__(
        self, exc_type: type[BaseException] | None, exc: BaseException | None, tb: TracebackType | None
    ) -> None: ...

@final
class AsyncGoodfetch(_AsyncCloseable, _AsyncExplainsSettings):
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
class Page(_Closeable, _ExplainsSettings, PageApi, PageWaits, PageEvents, PausesForHuman, AiVerbs, ElementReferences):
    """A page Goodfetch drives: a tab of a Session, opened by `session.new_page()` or by the page itself (a popup).
    Closing it closes only a tab Goodfetch opened; on a Borrowed browser it only releases it."""

    def close(self, *, run_before_unload: bool = False, timeout: Timeout = ...) -> None: ...

@final
class Session(_Closeable, _ExplainsSettings, SessionStateApi):
    """A Session of a Run: Playwright's context, under the glossary's name (ADR 0085), with its cookies and login
    (ADR 0092). Closing it never closes a Borrowed browser. There is no active page."""

    def new_page(self, *, settings: PageSettings = ..., timeout: Timeout = ...) -> Page: ...
    def pages(self) -> list[Page]: ...
    def on(self, event: Literal["page"], listener: Listener[Page]) -> Subscription: ...
    def route(self, url: UrlMatcher, handler: RouteHandler) -> Subscription: ...
    def add_init_script(self, script: str, arg: JsonValue | NotGiven = ...) -> Subscription: ...

@final
class Run(_Closeable, _ExplainsSettings, RunEvents):
    """A Run, always opened explicitly. Closing an unfinished Run stops it with a Checkpoint, resumable by its ID."""

    @property
    def id(self) -> str: ...
    def usage(self, *, timeout: Timeout = ...) -> RunUsage:
        """The Run's Usage so far, priced, with what remains of each Limit and the uncapped mark (ADR 0101)."""
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
        settings: RunSettings = ...,
        timeout: Timeout = ...,
        limits: Limits | None = None,
        plan: str | PathLike[str] | None = None,
    ) -> Run:
        """Placeholder start verb: its spelling is #478's; `timeout` and `limits` are #467's, `settings` #470's.

        `timeout` caps the whole Run. Omitted: the setting's value, which has no built-in default; `None`: no timeout.
        On expiry the Run stops with a Checkpoint, resumable.

        `plan` is the Run's Plan file, relative to the working directory: an input of the Run (ADR 0091), read here and
        written back here when a new version is promoted. Left out, the Plan lives only in the Run trace.
        """

@final
class CrawlRun(_Closeable, _ExplainsSettings, RunEvents):
    """A crawl's Run (ADR 0093). Its outcomes, progress and resume are #478's."""

    @property
    def id(self) -> str: ...
    def usage(self, *, timeout: Timeout = ...) -> RunUsage:
        """The Run's Usage so far, priced, with what remains of each Limit and the uncapped mark (ADR 0101)."""
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

@final
class Foxhound:
    """Foxhound, reached from the connection without I/O."""

    def start(
        self,
        crawl: Crawl,
        *,
        seeds: Sequence[Seed[Any]],
        settings: CrawlRunSettings = ...,
        timeout: Timeout = ...,
        limits: Limits | None = None,
    ) -> CrawlRun:
        """Provisional: the start and resume verbs are #478's (ADR 0093)."""

@final
class Goodfetch(_Closeable, _ExplainsSettings):
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
    ) -> Goodfetch:
        """Starts or finds the engine and completes `hello` before returning; use it with `with`.

        Left at `auto`, `engine` is filled by `GOODFETCH_URL` when set; a value written here wins over it. `settings` holds
        every other setting for this connection and everything beneath it. `env` is the environment map, read once here
        and never at import, by default the process's own (ADRs 0062, 0069): only the deployment variables are read, an
        unknown `GOODFETCH_` name fails with `SettingUnknownError`, and an Attached engine gets exactly this map.
        `project` is the project configuration file: left out, found upward from the working directory as the CLI does;
        `None`, none (ADR 0083).
        """

    @property
    def engine(self) -> EngineInfo: ...
    @property
    def collie(self) -> Collie: ...
    @property
    def foxhound(self) -> Foxhound: ...
    @property
    def identities(self) -> Identities: ...
