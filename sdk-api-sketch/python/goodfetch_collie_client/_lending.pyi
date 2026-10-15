"""Lent pages (ADR 0089): `page.lend()` opens a CDP endpoint that shows only the page's Session, for a library in the
same program, and one helper per supported library lends a page and connects that library with the safe options.

The helpers are free functions of this package, named by ADR 0099, taking the page and then the program's own library
object, so no SDK depends on Playwright or Browser Use (G7). Only the SDKs lend; a Firefox or WebKit page raises
`CapabilityUnavailableError`.
"""

from collections.abc import Awaitable, Mapping
from dataclasses import dataclass
from types import TracebackType
from typing import Literal, Protocol, Self, TypeVar, final, overload

from goodfetch_collie_client._lifecycle import AsyncPage, Page
from goodfetch_collie_client._results import ProgramError, UnavailableError
from goodfetch_collie_client._time import Timeout

_P = TypeVar("_P")
_S = TypeVar("_S")
_S_co = TypeVar("_S_co", covariant=True)

@final
class AsyncCDPEndpoint:
    """A lent page's endpoint: a WebSocket under ADR 0017's rules, its token in `headers`, never in `url`. The attach
    ends with `close()`, the library's `Browser.close`, its last connection closing, the page closing or the Run ending;
    in a crawl handler, with the work item's lease (ADR 0003)."""

    @property
    def url(self) -> str: ...
    @property
    def headers(self) -> Mapping[str, str]: ...
    async def close(self, *, timeout: Timeout = ...) -> None: ...
    async def __aenter__(self) -> Self: ...
    async def __aexit__(
        self, exc_type: type[BaseException] | None, exc: BaseException | None, tb: TracebackType | None
    ) -> None: ...

@final
class CDPEndpoint:
    """`AsyncCDPEndpoint` in the sync API."""

    @property
    def url(self) -> str: ...
    @property
    def headers(self) -> Mapping[str, str]: ...
    def close(self, *, timeout: Timeout = ...) -> None: ...
    def __enter__(self) -> Self: ...
    def __exit__(self, exc_type: type[BaseException] | None, exc: BaseException | None, tb: TracebackType | None) -> None: ...

class AsyncLendsPage:
    """Lending belongs to the page, so Collie carries it and Foxhound's handlers inherit it."""

    async def lend(self, *, timeout: Timeout = ...) -> AsyncCDPEndpoint:
        """Raises `SessionReservedError` while another attach on the page's Session is open.

        While the library is attached, await each of the program's own Collie calls on the page before the library acts:
        an AI verb whose document a navigation it did not start replaces raises `PageNavigatedDuringCallError` (ADR
        0114).
        """

class LendsPage:
    def lend(self, *, timeout: Timeout = ...) -> CDPEndpoint: ...

class _PlaywrightContext(Protocol[_P]):
    @property
    def pages(self) -> list[_P]: ...

class _PlaywrightBrowser(Protocol[_P]):
    @property
    def contexts(self) -> list[_PlaywrightContext[_P]]: ...

class _AsyncPlaywrightBrowser(Protocol[_P]):
    @property
    def contexts(self) -> list[_PlaywrightContext[_P]]: ...

class PlaywrightChromium(Protocol[_P]):
    """The part of Playwright's sync `chromium` the helper uses, matched by shape (G7)."""

    def connect_over_cdp(self, endpoint_url: str, *, headers: dict[str, str] | None = ..., no_defaults: bool | None = ...) -> _PlaywrightBrowser[_P]: ...

class AsyncPlaywrightChromium(Protocol[_P]):
    """The part of Playwright's async `chromium` the helper uses, matched by shape (G7)."""

    def connect_over_cdp(
        self, endpoint_url: str, *, headers: dict[str, str] | None = ..., no_defaults: bool | None = ...
    ) -> Awaitable[_AsyncPlaywrightBrowser[_P]]: ...

class BrowserSessionClass(Protocol[_S_co]):
    """Browser Use's `BrowserSession` class, matched by shape (G7): built from a CDP URL and headers."""

    def __call__(self, *, cdp_url: str, headers: dict[str, str], permissions: list[str]) -> _S_co: ...

@overload
def lend_to_playwright(page: Page, chromium: PlaywrightChromium[_P], /, *, timeout: Timeout = ...) -> _P:
    """Lends the page and connects Playwright to it with `no_defaults=True`, returning Playwright's page for it: sync with
    a `Page` and Playwright's sync `chromium`, awaited with an `AsyncPage` and its async one. Closing Playwright's
    browser ends the attach and never closes the browser (ADR 0089)."""
@overload
def lend_to_playwright(page: AsyncPage, chromium: AsyncPlaywrightChromium[_P], /, *, timeout: Timeout = ...) -> Awaitable[_P]: ...

async def lend_to_browser_use(page: AsyncPage, browser_session: BrowserSessionClass[_S], /, *, timeout: Timeout = ...) -> _S:
    """Lends the page and builds a Browser Use session on its endpoint, with an empty `permissions` list; Browser Use is
    async only, so it takes an `AsyncPage`."""

@final
class SessionReservedError(ProgramError):
    """A second `page.lend()` on a Session while one attach is open, or a lend of a Session a handler's attach reserves
    (ADR 0003)."""

@final
@dataclass(frozen=True, kw_only=True)
class AttachEndedDetails:
    reason: Literal["closed", "lease", "page_closed", "run_ended"] | str

@final
class AttachEndedError(UnavailableError):
    """The attach was cut: the work item's lease ended, the page closed or the Run ended (ADR 0003). Lend the page
    again."""

    @property
    def details(self) -> AttachEndedDetails: ...
