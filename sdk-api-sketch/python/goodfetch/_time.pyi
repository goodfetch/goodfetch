"""Time, waits, deadlines, cancellation and Limits (ADR 0080, decided in #467).

Every duration is a `datetime.timedelta`; a number is a type error, so a Playwright `30_000` cannot pass as
milliseconds or seconds. `timeout` takes `not_given` (the setting's default, finite for every phase), a
positive `timedelta`, or `None` for no timeout; zero or negative fails with a `ProgramError` naming `None`.
A timeout nests: a call inside another gets the smaller of its own and what remains of the outer one, and
time spent in a Human pause does not count.

Cancellation is the host's own: cancelling the task (`Task.cancel`, `asyncio.timeout`, `anyio.fail_after`)
sends `$/cancel` without waiting for the engine and lets `CancelledError` propagate unchanged.
The page API around the waits (locators, actions, `goto`) is a placeholder owned by #472.
"""

import re
from collections.abc import Awaitable, Callable
from contextlib import AbstractAsyncContextManager, AbstractContextManager
from dataclasses import dataclass
from datetime import timedelta
from decimal import Decimal
from typing import Generic, Literal, Never, TypeAlias, TypeVar, final

from typing_extensions import deprecated

from goodfetch._results import ProgramError

@final
class NotGiven:
    """The type of `not_given`: an argument left out, so the setting's default applies."""

not_given: NotGiven

Timeout: TypeAlias = timedelta | None | NotGiven
"""`not_given`: the setting's default; `None`: no timeout; a positive `timedelta`: at most that long."""

UrlMatcher: TypeAlias = str | re.Pattern[str] | Callable[[str], bool]
"""A glob, a compiled pattern, or a host-side predicate (where it runs is #468's)."""

LoadState: TypeAlias = Literal["domcontentloaded", "load", "networkidle"]
"""`networkidle`: no request in flight for 500 ms, WebSockets and event streams excluded."""

@final
@dataclass(frozen=True, kw_only=True)
class Count:
    """A bounded count; an `int` is an exact one."""

    min: int | None = None
    max: int | None = None

@final
@dataclass(frozen=True, kw_only=True)
class Response:
    """A response the page received. Its full shape is #472's."""

    url: str
    status: int

@final
@dataclass(frozen=True, kw_only=True)
class Request:
    """A request the page sent. Its full shape is #472's."""

    url: str
    method: str

T = TypeVar("T")

@final
class AsyncEventInfo(Generic[T]):
    """What `async with page.expect_…() as info:` binds: the event, awaited after the block."""

    @property
    def value(self) -> Awaitable[T]:
        """`await info.value` once the block has run its trigger."""

@final
class EventInfo(Generic[T]):
    """What `with page.expect_…() as info:` binds: the event, read after the block."""

    @property
    def value(self) -> T: ...

@final
@dataclass(frozen=True, kw_only=True)
class Money:
    """A sum of money in a currency; `Decimal`, so no float rounding reaches a Limit."""

    amount: Decimal
    currency: str
    """An ISO 4217 code, such as `EUR`."""

@final
@dataclass(frozen=True, kw_only=True)
class Limits:
    """Limits (ADR 0041) a program sets for its own work, one per kind of Usage.

    A program may set and lower them, never raise them: only a person raises a Limit. Reaching one stops the
    Run cleanly, `stopped` and `incomplete`, and the call that hit it raises `LimitReachedError`.
    """

    model_tokens: int | None = None
    model_calls: int | None = None
    browser_time: timedelta | None = None
    """Browser time of Usage, not elapsed time: that is the Run's `timeout`."""
    proxy_bytes: int | None = None
    paid_solves: int | None = None
    cost: Money | None = None
    """Priced through ADR 0012's price table."""

class RemovedError(ProgramError):
    """The Error code `removed`: a removed name was called, and `fix` names its replacement."""

# Async API.

class AsyncLocator:
    """A placeholder locator: its constructors and actions are #472's; its waits are #467's."""

    async def click(self, *, timeout: Timeout = ...) -> None:
        """Waits through the guard (ADR 0019) until the action is safe, then acts."""

    async def fill(self, value: str, *, timeout: Timeout = ...) -> None: ...
    async def wait_for(
        self, *, state: Literal["attached", "detached", "visible", "hidden"] = "visible", timeout: Timeout = ...
    ) -> None:
        """Playwright's `wait_for`: one element in the given state."""

    async def wait_for_count(self, count: int | Count, *, timeout: Timeout = ...) -> int:
        """Waits until the number of matches is in range: a different behaviour, so a different name (S3)."""

class AsyncPageWaits:
    """The page's navigation and app-state waits. An `expect_…` block arms its waiter before the block runs."""

    async def goto(self, url: str, *, wait_until: LoadState = "load", timeout: Timeout = ...) -> Response | None:
        """Placeholder for #472's navigation. Its timeout is the navigation phase's."""

    def locator(self, selector: str) -> AsyncLocator: ...
    def get_by_role(self, role: str, *, name: str | re.Pattern[str] | None = None) -> AsyncLocator: ...
    async def wait_for_url(self, url: UrlMatcher, *, wait_until: LoadState = "load", timeout: Timeout = ...) -> None: ...
    async def wait_for_response(self, url: UrlMatcher, *, timeout: Timeout = ...) -> Response: ...
    async def wait_for_request(self, url: UrlMatcher, *, timeout: Timeout = ...) -> Request: ...
    async def wait_for_request_finished(self, url: UrlMatcher, *, timeout: Timeout = ...) -> Request:
        """Waits until a matching request has finished, its body read."""

    async def wait_for_load_state(self, state: LoadState = "load", *, timeout: Timeout = ...) -> None: ...
    async def wait_for_function(self, page_function: str, arg: object = None, *, timeout: Timeout = ...) -> object:
        """Waits until a page function returns a truthy value; its serialization contract is #472's."""

    def expect_response(
        self, url: UrlMatcher, *, timeout: Timeout = ...
    ) -> AbstractAsyncContextManager[AsyncEventInfo[Response]]: ...
    def expect_request(
        self, url: UrlMatcher, *, timeout: Timeout = ...
    ) -> AbstractAsyncContextManager[AsyncEventInfo[Request]]: ...
    def expect_request_finished(
        self, url: UrlMatcher, *, timeout: Timeout = ...
    ) -> AbstractAsyncContextManager[AsyncEventInfo[Request]]: ...
    @deprecated(
        "Removed: a fixed sleep waits for nothing in particular. Wait for the app's state (wait_for_url, wait_for_response, wait_for_load_state, locator.wait_for, locator.wait_for_count), or use the host's own sleep. Fails with the Error code `removed`."
    )
    async def wait_for_timeout(self, timeout: Never) -> Never: ...

# Sync API: the same concepts, generated from the same catalog (ADR 0038).

class Locator:
    """A placeholder locator: its constructors and actions are #472's; its waits are #467's."""

    def click(self, *, timeout: Timeout = ...) -> None:
        """Waits through the guard (ADR 0019) until the action is safe, then acts."""

    def fill(self, value: str, *, timeout: Timeout = ...) -> None: ...
    def wait_for(
        self, *, state: Literal["attached", "detached", "visible", "hidden"] = "visible", timeout: Timeout = ...
    ) -> None: ...
    def wait_for_count(self, count: int | Count, *, timeout: Timeout = ...) -> int: ...

class PageWaits:
    """The page's navigation and app-state waits. An `expect_…` block arms its waiter before the block runs."""

    def goto(self, url: str, *, wait_until: LoadState = "load", timeout: Timeout = ...) -> Response | None: ...
    def locator(self, selector: str) -> Locator: ...
    def get_by_role(self, role: str, *, name: str | re.Pattern[str] | None = None) -> Locator: ...
    def wait_for_url(self, url: UrlMatcher, *, wait_until: LoadState = "load", timeout: Timeout = ...) -> None: ...
    def wait_for_response(self, url: UrlMatcher, *, timeout: Timeout = ...) -> Response: ...
    def wait_for_request(self, url: UrlMatcher, *, timeout: Timeout = ...) -> Request: ...
    def wait_for_request_finished(self, url: UrlMatcher, *, timeout: Timeout = ...) -> Request: ...
    def wait_for_load_state(self, state: LoadState = "load", *, timeout: Timeout = ...) -> None: ...
    def wait_for_function(self, page_function: str, arg: object = None, *, timeout: Timeout = ...) -> object: ...
    def expect_response(self, url: UrlMatcher, *, timeout: Timeout = ...) -> AbstractContextManager[EventInfo[Response]]: ...
    def expect_request(self, url: UrlMatcher, *, timeout: Timeout = ...) -> AbstractContextManager[EventInfo[Request]]: ...
    def expect_request_finished(
        self, url: UrlMatcher, *, timeout: Timeout = ...
    ) -> AbstractContextManager[EventInfo[Request]]: ...
    @deprecated(
        "Removed: a fixed sleep waits for nothing in particular. Wait for the app's state (wait_for_url, wait_for_response, wait_for_load_state, locator.wait_for, locator.wait_for_count), or use the host's own sleep. Fails with the Error code `removed`."
    )
    def wait_for_timeout(self, timeout: Never) -> Never: ...
