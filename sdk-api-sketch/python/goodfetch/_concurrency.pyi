"""Concurrency, events and user code (ADR 0081, decided in #468).

`AsyncGoodfetch` runs on AnyIO, so under asyncio and Trio. Its values belong to the event loop that opened their
connection: any number of tasks on that loop may share them, and a call from another loop or thread fails at once
with a `ProgramError` naming `Goodfetch`. `Goodfetch` (sync) runs the async core on a private daemon thread of its
own connection (ADR 0038) and never touches the caller's event loop, so it also works where one is already running
(a notebook cell, Streamlit, an agent framework's tool); its values are safe to call from any thread. In a sync call,
Ctrl+C, or any exception raised in the waiting thread, sends `$/cancel` without waiting and propagates unchanged.

The SDK keeps no queue: each call goes to the engine when it is made, and the engine starts a page's calls in arrival
order. Each event has one shape: side activity on a page is observed with `on(event, listener)`, which returns a
`Subscription`; a sequence the program reads to its end is an iterator. Both subscribe when the method is called.
User code runs, in the async API, as tasks on the connection's loop inside a task group of the value that registered
it, so closing that value cancels and awaits them; handlers and routes are `async def`, predicates plain functions,
listeners either. In the sync API it runs on a thread pool the connection owns, never on its private thread; it is
plain functions, and on free-threaded builds it runs in parallel. The engine sends no more requests at once than the
concurrency setting allows. Globs and compiled patterns run in the engine, in the Rust `regex` syntax (no look-around
or backreferences, linear time); a pattern using either fails at the call with a `ProgramError` naming the construct.
A predicate runs in the program, on the events the engine sends while it is armed, never in the page.
The page's events, routes and their payloads are placeholders owned by #472; `records()` is #479's.
"""

from collections.abc import Awaitable, Callable, Iterator, Mapping
from contextlib import AbstractAsyncContextManager, AbstractContextManager
from dataclasses import dataclass
from types import TracebackType
from typing import Generic, Literal, Self, TypeAlias, TypeVar, final, overload

from goodfetch._time import AsyncEventInfo, EventInfo, Request, Response, Timeout, UrlMatcher

T = TypeVar("T")

@final
@dataclass(frozen=True, kw_only=True)
class ConsoleMessage:
    """A console message. Its full shape is #472's."""

    type: str
    text: str

@final
class Subscription:
    """A registered listener or route. It closes like any value (ADR 0078), here without waiting.

    There is no `off` and no `once`: close the subscription, and wait for one event with `expect_event` (S2).
    """

    def close(self) -> None:
        """Idempotent. Unsubscribes without waiting for the engine."""

    def __enter__(self) -> Self: ...
    def __exit__(
        self, exc_type: type[BaseException] | None, exc: BaseException | None, tb: TracebackType | None
    ) -> None: ...

# Async API.

AsyncListener: TypeAlias = Callable[[T], Awaitable[None] | None]
"""Gets one source's events one at a time, in order; an awaitable it returns is awaited before the next event, so
lossless events slow their producer and telemetry coalesces (ADR 0024)."""

@final
class AsyncRoute:
    """An intercepted request waiting for the program's answer. Its semantics are #472's."""

    @property
    def request(self) -> Request: ...
    async def continue_(self, *, timeout: Timeout = ...) -> None: ...
    async def abort(self, *, timeout: Timeout = ...) -> None: ...
    async def fulfill(self, *, status: int, body: str | None = None, timeout: Timeout = ...) -> None: ...

@final
class AsyncEventIterator(Generic[T]):
    """A sequence read to its end with `async for`, subscribed when the method was called.

    It unsubscribes, without waiting, when it ends, when `aclose()` or `async with` closes it, or when it is released,
    which in CPython happens as soon as `async for x in run.records():` is left by `break`. One reader each. While
    open, an unread lossless sequence slows its Run (ADR 0024); a telemetry sequence keeps only the latest value.
    """

    def __aiter__(self) -> Self: ...
    async def __anext__(self) -> T: ...
    async def aclose(self) -> None: ...
    async def __aenter__(self) -> Self: ...
    async def __aexit__(
        self, exc_type: type[BaseException] | None, exc: BaseException | None, tb: TracebackType | None
    ) -> None: ...

class AsyncPageEvents:
    """The page's events, under Playwright's names (S3). The full list is #472's."""

    @overload
    def on(self, event: Literal["console"], listener: AsyncListener[ConsoleMessage]) -> Subscription: ...
    @overload
    def on(self, event: Literal["request", "requestfinished"], listener: AsyncListener[Request]) -> Subscription: ...
    @overload
    def on(self, event: Literal["response"], listener: AsyncListener[Response]) -> Subscription: ...
    @overload
    def expect_event(
        self, event: Literal["console"], *, predicate: Callable[[ConsoleMessage], bool] | None = None, timeout: Timeout = ...
    ) -> AbstractAsyncContextManager[AsyncEventInfo[ConsoleMessage]]: ...
    @overload
    def expect_event(
        self, event: Literal["response"], *, predicate: Callable[[Response], bool] | None = None, timeout: Timeout = ...
    ) -> AbstractAsyncContextManager[AsyncEventInfo[Response]]:
        """Waits for one event, armed when the block opens (ADR 0080); the predicate runs in the program."""
    @overload
    async def wait_for_event(
        self, event: Literal["console"], *, predicate: Callable[[ConsoleMessage], bool] | None = None, timeout: Timeout = ...
    ) -> ConsoleMessage: ...
    @overload
    async def wait_for_event(
        self, event: Literal["response"], *, predicate: Callable[[Response], bool] | None = None, timeout: Timeout = ...
    ) -> Response: ...

    def route(self, url: UrlMatcher, handler: Callable[[AsyncRoute], Awaitable[None]]) -> Subscription:
        """Answers matching requests, registered when called. The handler runs as a task on the connection's loop."""

class AsyncRunEvents:
    def records(self) -> AsyncEventIterator[Mapping[str, object]]:
        """Placeholder for #479: the Run's Records. Their type is #469's; where the sequence starts is #479's."""

# Sync API: the same concepts, generated from the same catalog (ADR 0038).

Listener: TypeAlias = Callable[[T], None]
"""Runs on a thread of the connection's pool and gets one source's events one at a time, in order."""

@final
class Route:
    """An intercepted request waiting for the program's answer. Its semantics are #472's."""

    @property
    def request(self) -> Request: ...
    def continue_(self, *, timeout: Timeout = ...) -> None: ...
    def abort(self, *, timeout: Timeout = ...) -> None: ...
    def fulfill(self, *, status: int, body: str | None = None, timeout: Timeout = ...) -> None: ...

@final
class EventIterator(Generic[T]):
    """A sequence read to its end with `for`, subscribed when the method was called; it unsubscribes as
    `AsyncEventIterator` does, with `close()` and `with`."""

    def __iter__(self) -> Iterator[T]: ...
    def __next__(self) -> T: ...
    def close(self) -> None: ...
    def __enter__(self) -> Self: ...
    def __exit__(
        self, exc_type: type[BaseException] | None, exc: BaseException | None, tb: TracebackType | None
    ) -> None: ...

class PageEvents:
    """The page's events, under Playwright's names (S3). The full list is #472's."""

    @overload
    def on(self, event: Literal["console"], listener: Listener[ConsoleMessage]) -> Subscription: ...
    @overload
    def on(self, event: Literal["request", "requestfinished"], listener: Listener[Request]) -> Subscription: ...
    @overload
    def on(self, event: Literal["response"], listener: Listener[Response]) -> Subscription: ...
    @overload
    def expect_event(
        self, event: Literal["console"], *, predicate: Callable[[ConsoleMessage], bool] | None = None, timeout: Timeout = ...
    ) -> AbstractContextManager[EventInfo[ConsoleMessage]]: ...
    @overload
    def expect_event(
        self, event: Literal["response"], *, predicate: Callable[[Response], bool] | None = None, timeout: Timeout = ...
    ) -> AbstractContextManager[EventInfo[Response]]: ...
    @overload
    def wait_for_event(
        self, event: Literal["console"], *, predicate: Callable[[ConsoleMessage], bool] | None = None, timeout: Timeout = ...
    ) -> ConsoleMessage: ...
    @overload
    def wait_for_event(
        self, event: Literal["response"], *, predicate: Callable[[Response], bool] | None = None, timeout: Timeout = ...
    ) -> Response: ...
    def route(self, url: UrlMatcher, handler: Callable[[Route], None]) -> Subscription:
        """Answers matching requests, registered when called. The handler runs on a thread of the connection's pool."""

class RunEvents:
    def records(self) -> EventIterator[Mapping[str, object]]:
        """Placeholder for #479: the Run's Records. Their type is #469's; where the sequence starts is #479's."""
