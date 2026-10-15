"""Time, waits, deadlines, cancellation and Limits (ADR 0080, decided in #467).

Every duration is a `datetime.timedelta`; a number is a type error, so a Playwright `30_000` cannot pass as
milliseconds or seconds. `timeout` takes `not_given` (the setting's default, finite for every phase), a
positive `timedelta`, or `None` for no timeout; zero or negative fails with a `ProgramError` naming `None`.
A timeout nests: a call inside another gets the smaller of its own and what remains of the outer one, and
time spent in a Human pause does not count.

Cancellation is the host's own: cancelling the task (`Task.cancel`, `asyncio.timeout`, `anyio.fail_after`)
sends `$/cancel` without waiting for the engine and lets `CancelledError` propagate unchanged.
The page API around the waits (locators, actions, `goto`) is ADR 0085's (`_page`).
"""

import re
from collections.abc import Awaitable, Callable
from contextlib import AbstractAsyncContextManager, AbstractContextManager
from dataclasses import dataclass
from datetime import timedelta
from decimal import Decimal
from typing import Generic, Literal, Never, TypeAlias, TypeVar, final

from typing_extensions import deprecated

from goodfetch_collie_client._page import AsyncRequest, AsyncResponse, Request, Response
from goodfetch_collie_client._results import ProgramError

@final
class NotGiven:
    """The type of `not_given`: an argument left out, so the setting's default applies."""

not_given: NotGiven

Timeout: TypeAlias = timedelta | None | NotGiven
"""`not_given`: the setting's default; `None`: no timeout; a positive `timedelta`: at most that long."""

URLMatcher: TypeAlias = str | re.Pattern[str] | Callable[[str], bool]
"""A glob or a compiled pattern, run in the engine in the Rust `regex` syntax (no look-around or backreferences,
linear time), or a predicate, run in the program (ADR 0081)."""

LoadState: TypeAlias = Literal["domcontentloaded", "load", "networkidle"]
"""`networkidle`: no request in flight for 500 ms, WebSockets and server-sent events excluded."""

@final
@dataclass(frozen=True, kw_only=True)
class Count:
    """A bounded count; an `int` is an exact one."""

    min: int | None = None
    max: int | None = None

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
    """Limits (ADR 0041) a program sets for its own work: one per kind of Usage, plus a price.

    A program may set and lower them, never raise them: only a person raises a Limit. Reaching one follows
    ADR 0079: a call refused because a Limit is used up raises `LimitReachedError`, and AI work a Limit stops
    returns `Stopped` with the Limit by name; the Run ends `stopped` and `incomplete`.
    """

    model_tokens: int | None = None
    model_calls: int | None = None
    browser_time: timedelta | None = None
    """Browser time of Usage, not elapsed time: that is the Run's `timeout`."""
    proxy_bytes: int | None = None
    paid_solves: int | None = None
    paid_fetches: int | None = None
    """Only caps: a Paid fetch is made only under a Limit a Client could not set (ADRs 0041, 0045)."""
    price: Money | None = None
    """A price, not a kind of Usage: priced through the price table (ADR 0012), as `usage.price` is (ADR 0101). A call
    whose model has no price is refused under it with `ModelPriceUnknownError`."""

class RemovedError(ProgramError):
    """The Error code `removed`: a removed name was called, and `fix` names its replacement."""

# Async API.

class AsyncPageWaits:
    """The page's navigation and app-state waits. An `expect_…` block arms its waiter before the block runs."""

    async def wait_for_url(self, url: URLMatcher, *, wait_until: LoadState = "load", timeout: Timeout = ...) -> None: ...
    async def wait_for_response(self, url: URLMatcher, *, timeout: Timeout = ...) -> AsyncResponse: ...
    async def wait_for_request(self, url: URLMatcher, *, timeout: Timeout = ...) -> AsyncRequest: ...
    async def wait_for_request_finished(self, url: URLMatcher, *, timeout: Timeout = ...) -> AsyncRequest:
        """Waits until a matching request has finished, its body read."""

    async def wait_for_load_state(self, state: LoadState = "load", *, timeout: Timeout = ...) -> None: ...
    def expect_response(
        self, url: URLMatcher, *, timeout: Timeout = ...
    ) -> AbstractAsyncContextManager[AsyncEventInfo[AsyncResponse]]: ...
    def expect_request(
        self, url: URLMatcher, *, timeout: Timeout = ...
    ) -> AbstractAsyncContextManager[AsyncEventInfo[AsyncRequest]]: ...
    def expect_request_finished(
        self, url: URLMatcher, *, timeout: Timeout = ...
    ) -> AbstractAsyncContextManager[AsyncEventInfo[AsyncRequest]]: ...
    @deprecated(
        "Removed: a fixed sleep waits for nothing in particular. Wait for the app's state (wait_for_url, wait_for_response, wait_for_load_state, locator.wait_for, locator.wait_for_count), or use the host's own sleep. Fails with the Error code `removed`."
    )
    async def wait_for_timeout(self, timeout: Never) -> Never: ...

# Sync API: the same concepts, generated from the same catalog (ADR 0038).

class PageWaits:
    """The page's navigation and app-state waits. An `expect_…` block arms its waiter before the block runs."""

    def wait_for_url(self, url: URLMatcher, *, wait_until: LoadState = "load", timeout: Timeout = ...) -> None: ...
    def wait_for_response(self, url: URLMatcher, *, timeout: Timeout = ...) -> Response: ...
    def wait_for_request(self, url: URLMatcher, *, timeout: Timeout = ...) -> Request: ...
    def wait_for_request_finished(self, url: URLMatcher, *, timeout: Timeout = ...) -> Request: ...
    def wait_for_load_state(self, state: LoadState = "load", *, timeout: Timeout = ...) -> None: ...
    def expect_response(self, url: URLMatcher, *, timeout: Timeout = ...) -> AbstractContextManager[EventInfo[Response]]: ...
    def expect_request(self, url: URLMatcher, *, timeout: Timeout = ...) -> AbstractContextManager[EventInfo[Request]]: ...
    def expect_request_finished(
        self, url: URLMatcher, *, timeout: Timeout = ...
    ) -> AbstractContextManager[EventInfo[Request]]: ...
    @deprecated(
        "Removed: a fixed sleep waits for nothing in particular. Wait for the app's state (wait_for_url, wait_for_response, wait_for_load_state, locator.wait_for, locator.wait_for_count), or use the host's own sleep. Fails with the Error code `removed`."
    )
    def wait_for_timeout(self, timeout: Never) -> Never: ...
