"""Entry points and lifecycle (ADR 0078, decided in #465).

Logs go to `logging` loggers under `goodfetch` (`goodfetch.client`, `goodfetch.engine.<area>`); the SDK adds only a
`NullHandler`. The Run's start verbs, Sessions and the page API are placeholders owned by #478, #476 and #472.
Timeouts, waits and Limits come from #467 (`_time`).
"""

from collections.abc import Generator
from dataclasses import dataclass
from types import TracebackType
from typing import Literal, Never, Protocol, Self, TypeAlias, final, overload

from goodfetch._time import AsyncPageWaits, Limits, PageWaits, Timeout

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

class _AsyncCloseable:
    async def close(self, *, timeout: Timeout = ...) -> None:
        """Idempotent. Its timeout is the close phase's. Closes children first; any later call on this value fails with the Error code `closed`."""

    async def __aenter__(self) -> Self: ...
    async def __aexit__(
        self, exc_type: type[BaseException] | None, exc: BaseException | None, tb: TracebackType | None
    ) -> None: ...

@final
class AsyncPage(_AsyncCloseable, AsyncPageWaits):
    """A page Goodfetch drives. Its API is #472's. Closing it closes only a tab Goodfetch opened."""

@final
class AsyncSession(_AsyncCloseable):
    """A Session of a Run. Its API is #476's. Closing it never closes a Borrowed browser."""

    async def new_page(self) -> AsyncPage: ...

@final
class AsyncRun(_AsyncCloseable):
    """A Run, always opened explicitly. Closing an unfinished Run stops it with a Checkpoint, resumable by its ID."""

    @property
    def id(self) -> str: ...
    async def new_page(self) -> AsyncPage: ...
    @overload
    async def borrow(self, page: BorrowableAsyncPage, /) -> AsyncPage: ...
    @overload
    async def borrow(self, browser: BorrowableAsyncBrowser, /) -> AsyncSession: ...

@final
class AsyncCollie:
    """Collie, reached from the connection without I/O."""

    async def start(self, *, timeout: Timeout = ..., limits: Limits | None = None) -> AsyncRun:
        """Placeholder start verb: its spelling is #478's; `timeout` and `limits` are #467's.

        `timeout` caps the whole Run. Omitted: the setting's value, which has no built-in default; `None`: no timeout.
        On expiry the Run stops with a Checkpoint, resumable.
        """

@final
class AsyncFoxhound:
    """Foxhound, reached from the connection without I/O. Its crawl API is #477's."""

@final
class _AsyncConnecting:
    """What `AsyncGoodfetch.connect()` returns: awaited, or used with `async with`."""

    def __await__(self) -> Generator[None, None, AsyncGoodfetch]: ...
    async def __aenter__(self) -> AsyncGoodfetch: ...
    async def __aexit__(
        self, exc_type: type[BaseException] | None, exc: BaseException | None, tb: TracebackType | None
    ) -> None: ...

@final
class AsyncGoodfetch(_AsyncCloseable):
    """The engine connection: the entry point. Each one is independent, and many may be open in one process."""

    @classmethod
    def connect(cls, *, engine: EngineChoice = "auto", timeout: Timeout = ..., limits: Limits | None = None) -> _AsyncConnecting:
        """Starts or finds the engine and completes `hello` before the connection is returned."""

    @property
    def engine(self) -> EngineInfo: ...
    @property
    def collie(self) -> AsyncCollie: ...
    @property
    def foxhound(self) -> AsyncFoxhound: ...

# Sync API: the same concepts, generated from the same catalog (ADR 0038).

class _Closeable:
    def close(self, *, timeout: Timeout = ...) -> None:
        """Idempotent. Its timeout is the close phase's. Closes children first; any later call on this value fails with the Error code `closed`."""

    def __enter__(self) -> Self: ...
    def __exit__(
        self, exc_type: type[BaseException] | None, exc: BaseException | None, tb: TracebackType | None
    ) -> None: ...

@final
class Page(_Closeable, PageWaits):
    """A page Goodfetch drives. Its API is #472's. Closing it closes only a tab Goodfetch opened."""

@final
class Session(_Closeable):
    """A Session of a Run. Its API is #476's. Closing it never closes a Borrowed browser."""

    def new_page(self) -> Page: ...

@final
class Run(_Closeable):
    """A Run, always opened explicitly. Closing an unfinished Run stops it with a Checkpoint, resumable by its ID."""

    @property
    def id(self) -> str: ...
    def new_page(self) -> Page: ...
    @overload
    def borrow(self, page: BorrowablePage, /) -> Page: ...
    @overload
    def borrow(self, browser: BorrowableBrowser, /) -> Session: ...

@final
class Collie:
    """Collie, reached from the connection without I/O."""

    def start(self, *, timeout: Timeout = ..., limits: Limits | None = None) -> Run:
        """Placeholder start verb: its spelling is #478's; `timeout` and `limits` are #467's.

        `timeout` caps the whole Run. Omitted: the setting's value, which has no built-in default; `None`: no timeout.
        On expiry the Run stops with a Checkpoint, resumable.
        """

@final
class Foxhound:
    """Foxhound, reached from the connection without I/O. Its crawl API is #477's."""

@final
class Goodfetch(_Closeable):
    """The engine connection: the entry point. Each one is independent, and many may be open in one process."""

    @classmethod
    def connect(cls, *, engine: EngineChoice = "auto", timeout: Timeout = ..., limits: Limits | None = None) -> Goodfetch:
        """Starts or finds the engine and completes `hello` before returning; use it with `with`."""

    @property
    def engine(self) -> EngineInfo: ...
    @property
    def collie(self) -> Collie: ...
    @property
    def foxhound(self) -> Foxhound: ...
