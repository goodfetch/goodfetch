"""Entry points and lifecycle (ADR 0078, decided in #465).

Logs go to `logging` loggers under `goodfetch` (`goodfetch.client`, `goodfetch.engine.<area>`); the SDK adds only a
`NullHandler`. The Run's start verbs, its ID, its end and its report are ADR 0102's (`_run`); Sessions, logins and secrets are ADR 0092's
(`_sessions`); the page API is ADR 0085's (`_page`).
Timeouts, waits and Limits come from #467 (`_time`); events and user code from #468 (`_concurrency`).
"""

from collections.abc import Awaitable, Callable, Sequence
from os import PathLike
from dataclasses import dataclass
from types import TracebackType
from typing import Literal, Never, Protocol, Self, TypeAlias, TypeVar, final, overload

from goodfetch_collie_client._agent import AgentAPI, AsyncAgentAPI
from goodfetch_collie_client._ai import AIVerbs, AsyncAIVerbs, AsyncElementReferences, ElementReferences
from goodfetch_collie_client._catalog import AsyncRawCall, RawCall
from goodfetch_collie_client._concurrency import (
    AsyncListener,
    AsyncPageEvents,
    AsyncRunEvents,
    Listener,
    PageEvents,
    RunEvents,
    Subscription,
)
from goodfetch_collie_client._data import JSONValue
from goodfetch_collie_client._effects import AsyncDeclaresEffects, DeclaresEffects
from goodfetch_collie_client._lending import AsyncLendsPage, LendsPage
from goodfetch_collie_client._models import RunUsage
from goodfetch_collie_client._output import AsyncKeyValueStore, KeyValueStore
from goodfetch_collie_client._page import AsyncPageAPI, AsyncRouteHandler, PageAPI, RouteHandler
from goodfetch_collie_client._recording import Recording
from goodfetch_collie_client._results import ProgramError, RefusedError
from goodfetch_collie_client._run import AsyncRunControl, AsyncRunView, RunControl, RunProgress, RunReport, RunView
from goodfetch_collie_client._sessions import (
    AsyncIdentities,
    AsyncPageStorage,
    AsyncPausesForHuman,
    AsyncSessionStateAPI,
    Identities,
    PageStorage,
    PausesForHuman,
    SessionSource,
    SessionStateAPI,
)
from goodfetch_collie_client._settings import ExplainedSetting, PageSettings, RunSettings, SessionSettings, SettingKey
from goodfetch_collie_client._time import AsyncPageWaits, Limits, NotGiven, PageWaits, Timeout, URLMatcher

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
    product: Literal["collie", "foxhound"] | str
    """The engine's Product (ADR 0095): `foxhound` serves a Collie connection too. A newer one is kept (G3)."""
    version: str
    """The engine's Product version (ADR 0090), reported, never compared."""
    protocol_version: str
    project_file: str | None
    """The project configuration file read, or `None` when none was found or `project` was `None` (ADR 0083)."""
    home: str | None
    """The Goodfetch home the engine uses (`GOODFETCH_HOME`), or `None` for an Exposed Daemon or Goodfetch Cloud."""
    engine_path: str | None
    """The engine executable started (`GOODFETCH_COLLIE_ENGINE_PATH` or `GOODFETCH_FOXHOUND_ENGINE_PATH`, ADR 0090), or `None` when none was started."""
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
    AsyncPageAPI,
    AsyncPageWaits,
    AsyncPageEvents,
    AsyncPageStorage,
    AsyncPausesForHuman,
    AsyncAIVerbs,
    AsyncElementReferences,
    AsyncRawCall,
    AsyncAgentAPI,
    AsyncLendsPage,
):
    """A page Goodfetch drives: a tab of a Session, opened by `session.new_page()` or by the page itself (a popup).
    Closing it closes only a tab Goodfetch opened; on a Borrowed browser it only releases it."""

    async def close(self, *, run_before_unload: bool = False, timeout: Timeout = ...) -> None:
        """Idempotent. `run_before_unload` runs the page's `beforeunload` handlers, which closing otherwise skips."""

@final
class AsyncSession(_AsyncCloseable, _AsyncExplainsSettings, AsyncSessionStateAPI, AsyncRawCall):
    """A Session of a Run: Playwright's context, under the glossary's name (ADR 0085), with its cookies and login
    (ADR 0092). Closing it never closes a Borrowed browser. There is no active page."""

    async def new_page(self, *, settings: PageSettings = ..., timeout: Timeout = ...) -> AsyncPage: ...
    def pages(self) -> list[AsyncPage]:
        """The Session's open pages, read without a round trip."""

    def on(self, event: Literal["page"], listener: AsyncListener[AsyncPage]) -> Subscription:
        """A new page in the Session: one it opened, a popup, or a tab a link opened."""

    def route(self, url: URLMatcher, handler: AsyncRouteHandler) -> Subscription:
        """Answers requests of every page of the Session, a popup's first request included; page routes run first."""

    def add_init_script(self, script: str, arg: JSONValue | NotGiven = ...) -> Subscription: ...

class AsyncRun(_AsyncCloseable, _AsyncExplainsSettings, AsyncRunEvents, AsyncRunControl[RunReport, RunProgress], AsyncRawCall, AsyncDeclaresEffects):
    """A Run, always opened explicitly. `finish()` ends it `finished`; closing an unfinished Run stops it with a
    Checkpoint, resumable by starting its ID again (ADR 0102)."""

    @property
    def id(self) -> str:
        """Chosen by the program at the start, or generated: it names this Run for good."""
    async def close(self, *, timeout: Timeout = ...) -> None:
        """Leaving the scope or `close()` stops the Run with a Checkpoint; `finish()` ends it.

        Idempotent; leaving `async with` calls it. A Run not yet ended stops at its next safe point, `stopped` and
        resumable by starting its ID again (ADRs 0078, 0102); one that ended is only released. Its timeout is the close
        phase's; any later call on the Run fails with the Error code `closed`."""
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
    async def borrow(self, page: BorrowableAsyncPage, /) -> AsyncPage:
        """Borrows the program's own page (ADR 0056's `client_browser`); ending the borrow never closes it.

        The program's own driver keeps the page too, outside the engine's order of Gestures: await each Collie call
        before that driver acts on the page again. An AI verb whose document a navigation it did not start replaces
        raises `PageNavigatedDuringCallError` (ADR 0114); the driver's input within one document is beyond what the
        engine sees.
        """
    @overload
    async def borrow(self, browser: BorrowableAsyncBrowser, /) -> AsyncSession: ...

class AsyncConnection(_AsyncCloseable, _AsyncExplainsSettings, AsyncRawCall):
    """Any connection: what `AsyncCollie` and `AsyncFoxhound` share, declared once in `goodfetch_collie_client`
    (ADRs 0095, 0099), so a library typed for it accepts either Product's connection. Each Product's package holds its
    connection class, with its own `connect()`; a Foxhound connection carries every member of a Collie connection under
    the same names.

    It runs on AnyIO, under asyncio or Trio. Its values belong to the event loop that opened it; a call from another
    loop or thread fails with a `ProgramError` naming the connection (ADR 0081).
    """

    @property
    def engine(self) -> EngineInfo:
        """The engine reached, fixed when the connection opened."""
    @property
    def identities(self) -> AsyncIdentities:
        """The install's Saved identities (ADR 0092)."""
    async def start(
        self,
        *,
        run_id: str | None = None,
        settings: RunSettings = ...,
        timeout: Timeout = ...,
        limits: Limits | None = None,
        plan: str | PathLike[str] | None = None,
        recording: Recording | None = None,
    ) -> AsyncRun:
        """Leaving the scope or `close()` stops the Run with a Checkpoint; `finish()` ends it.

        Starts a Run and returns once it is started (ADR 0102).

        `run_id` names the Run for good: a start with an existing Run ID resumes the Run if it was interrupted, or
        returns it as it ended (ADR 0102); a new ID starts a new Run. Left out, the engine generates one. A start whose
        `run_id` names an existing Run with the same inputs resumes it if interrupted (`started_as` is `resumed`) or
        returns it as it is if it finished (`ended`); it raises `RunInUseError` if the Run is live, `RunConflictError`
        for other inputs, `LimitReachedError` if a used-up Limit stopped it, and `RunStateUnavailableError` if its state
        is gone.

        Fails with `model_price_unknown` (`ModelPriceUnknownError`) when a currency Limit or `ai.allowance.price` applies
        and the Run's `ai.model` has no price, unless `plan.frozen` (ADR 0117).

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
    async def open_run(self, run_id: str, *, timeout: Timeout = ...) -> AsyncRunView[RunReport, RunProgress]:
        """Opens a Run by its ID to read it, never to drive it: a detached Run, another program's, or one that ended.
        Raises `RunNotFoundError`."""


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
class Page(
    _Closeable, _ExplainsSettings, PageAPI, PageWaits, PageEvents, PageStorage, PausesForHuman, AIVerbs, ElementReferences, RawCall, AgentAPI, LendsPage
):
    """A page Goodfetch drives: a tab of a Session, opened by `session.new_page()` or by the page itself (a popup).
    Closing it closes only a tab Goodfetch opened; on a Borrowed browser it only releases it."""

    def close(self, *, run_before_unload: bool = False, timeout: Timeout = ...) -> None: ...

@final
class Session(_Closeable, _ExplainsSettings, SessionStateAPI, RawCall):
    """A Session of a Run: Playwright's context, under the glossary's name (ADR 0085), with its cookies and login
    (ADR 0092). Closing it never closes a Borrowed browser. There is no active page."""

    def new_page(self, *, settings: PageSettings = ..., timeout: Timeout = ...) -> Page: ...
    def pages(self) -> list[Page]: ...
    def on(self, event: Literal["page"], listener: Listener[Page]) -> Subscription: ...
    def route(self, url: URLMatcher, handler: RouteHandler) -> Subscription: ...
    def add_init_script(self, script: str, arg: JSONValue | NotGiven = ...) -> Subscription: ...

class Run(_Closeable, _ExplainsSettings, RunEvents, RunControl[RunReport, RunProgress], RawCall, DeclaresEffects):
    """A Run, always opened explicitly. `finish()` ends it `finished`; closing an unfinished Run stops it with a
    Checkpoint, resumable by starting its ID again (ADR 0102)."""

    @property
    def id(self) -> str: ...
    def close(self, *, timeout: Timeout = ...) -> None:
        """Leaving the scope or `close()` stops the Run with a Checkpoint; `finish()` ends it.

        `AsyncRun.close` in the sync API; leaving `with` calls it."""
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
    def borrow(self, page: BorrowablePage, /) -> Page:
        """Borrows the program's own page (ADR 0056's `client_browser`); ending the borrow never closes it.

        The program's own driver keeps the page too, outside the engine's order of Gestures: await each Collie call
        before that driver acts on the page again. An AI verb whose document a navigation it did not start replaces
        raises `PageNavigatedDuringCallError` (ADR 0114); the driver's input within one document is beyond what the
        engine sees.
        """
    @overload
    def borrow(self, browser: BorrowableBrowser, /) -> Session: ...

class Connection(_Closeable, _ExplainsSettings, RawCall):
    """Any connection, as `AsyncConnection`: what `Collie` and `Foxhound` share."""

    @property
    def engine(self) -> EngineInfo: ...
    @property
    def identities(self) -> Identities: ...
    def start(
        self,
        *,
        run_id: str | None = None,
        settings: RunSettings = ...,
        timeout: Timeout = ...,
        limits: Limits | None = None,
        plan: str | PathLike[str] | None = None,
        recording: Recording | None = None,
    ) -> Run:
        """Leaving the scope or `close()` stops the Run with a Checkpoint; `finish()` ends it.

        Starts a Run and returns once it is started (ADR 0102).

        `run_id` names the Run for good: a start with an existing Run ID resumes the Run if it was interrupted, or
        returns it as it ended (ADR 0102); a new ID starts a new Run. Left out, the engine generates one. A start whose
        `run_id` names an existing Run with the same inputs resumes it if interrupted (`started_as` is `resumed`) or
        returns it as it is if it finished (`ended`); it raises `RunInUseError` if the Run is live, `RunConflictError`
        for other inputs, `LimitReachedError` if a used-up Limit stopped it, and `RunStateUnavailableError` if its state
        is gone.

        Fails with `model_price_unknown` (`ModelPriceUnknownError`) when a currency Limit or `ai.allowance.price` applies
        and the Run's `ai.model` has no price, unless `plan.frozen` (ADR 0117).

        `timeout` caps the whole Run. Omitted: the setting's value, which has no built-in default; `None`: no timeout.
        On expiry the Run stops with a Checkpoint, resumable; a resume's clock starts afresh. On a resume, `limits` are
        compared with what the Run began with: equal leaves each Limit as it stands, lower lowers it, higher raises
        `LimitRaiseRefusedError` (ADR 0041); a changed setting applies from then on, and one fixed for a Run's life, such
        as `crawl.scope.*`, raises `SettingNotApplicableError`.

        `plan` is the Run's Plan file, relative to the working directory: an input of the Run (ADR 0091), read here and
        written back here when a new version is promoted. Left out, the Plan lives only in the Run trace.
        """
    def open_run(self, run_id: str, *, timeout: Timeout = ...) -> RunView[RunReport, RunProgress]: ...

@final
class BorrowRequiresCDPError(ProgramError):
    """`run.borrow()` was given a Firefox or WebKit page, which exposes no CDP (ADRs 0056, 0089): borrow a Chromium page."""

@final
class BrowserBorrowedError(RefusedError):
    """Another Run already borrows this browser, judged on the browser the connection reaches, not on its address (ADR
    0056)."""
