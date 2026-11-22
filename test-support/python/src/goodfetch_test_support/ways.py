"""Python's three APIs, each a named case of one test (ADR 0069).

A Client conformance case is written once, against the async API, and
`three_ways` runs it async on asyncio, async on trio and through the
generated sync API, so the three stay equal.
"""

from __future__ import annotations

import enum
import functools
import inspect
import sys
from collections.abc import Awaitable, Callable
from typing import TypeVar, cast

import anyio.to_thread
import pytest

ASYNC_BACKENDS = ("asyncio", "trio")
"""The anyio backends every async test runs on, named rather than probed."""

_Api = TypeVar("_Api")
_Test = TypeVar("_Test", bound=Callable[..., Awaitable[None]])


class Way(enum.Enum):
    """One of the three ways a case calls the SDK."""

    ASYNCIO = "asyncio"
    """The async API, on asyncio."""
    TRIO = "trio"
    """The async API, on trio."""
    SYNC = "sync"
    """The generated sync API, each call from a worker thread, off the event loop."""

    def api(self, async_api: _Api, sync_api: object) -> _Api:
        """The API this way calls, with the async API's methods.

        The two async ways return `async_api`. The sync way returns an object
        whose every method awaits the sync API's method of the same name in a
        worker thread, as a sync caller outside any event loop would call it;
        an attribute that is no method, such as a property's value, is the
        sync API's own.
        """
        if self is not Way.SYNC:
            return async_api
        # The generated sync API has each async method's name and parameters,
        # so the proxy answers the async API's interface.
        return cast("_Api", _InWorkerThread(sync_api))

    async def open(
        self, open_async: Callable[[], Awaitable[_Api]], open_sync: Callable[[], object]
    ) -> _Api:
        """The value this way's API opens, such as a Connection, with async methods.

        The two async ways await `open_async`. The sync way runs `open_sync`
        in a worker thread and hands its value as `api` hands the sync API.
        """
        if self is not Way.SYNC:
            return await open_async()
        opened = await anyio.to_thread.run_sync(open_sync)
        return cast("_Api", _InWorkerThread(opened))


class _InWorkerThread:
    """A sync API whose methods are awaited, each call run in a worker thread.

    A method's value that has a `close` method, as every value with a
    round trip has (ADR 0078), such as a Run, is handed the same way. A
    method its async twin, `Async<Name>` of the same module, has as a
    plain function, such as a page's `on`, is the sync API's own, since
    both APIs call it without `await`.
    """

    def __init__(self, sync_api: object) -> None:
        self._sync_api = sync_api

    def __getattr__(self, name: str) -> object:
        method: object = getattr(self._sync_api, name)
        if not callable(method) or _plain_in_async_twin(self._sync_api, name):
            return method

        async def call(*args: object, **kwargs: object) -> object:
            value = await anyio.to_thread.run_sync(
                functools.partial(method, *args, **kwargs)
            )
            return (
                _InWorkerThread(value)
                if callable(getattr(value, "close", None))
                else value
            )

        return call


def _plain_in_async_twin(sync_api: object, name: str) -> bool:
    """Whether the async twin of `sync_api`'s class has `name` as a plain function."""
    sync_class = type(sync_api)
    twin: object = getattr(
        sys.modules[sync_class.__module__], f"Async{sync_class.__name__}", None
    )
    member: object = getattr(twin, name, None)
    return callable(member) and not inspect.iscoroutinefunction(member)


_WAYS = [
    pytest.param("asyncio", Way.ASYNCIO, id="asyncio"),
    pytest.param("trio", Way.TRIO, id="trio"),
    pytest.param("asyncio", Way.SYNC, id="sync"),
]


def three_ways(test: _Test) -> _Test:
    """Runs an async test three ways, as the cases `[asyncio]`, `[trio]` and `[sync]`.

    The test takes `way` and calls `way.api(async_api, sync_api)`; the sync
    way's event loop is asyncio's.
    """
    marked = pytest.mark.parametrize(("anyio_backend", "way"), _WAYS)(test)
    # Typed here: mypy reads `MarkDecorator.__call__` as returning `Any`.
    anyio_test: _Test = pytest.mark.anyio(marked)
    return anyio_test


async def until(
    holds: Callable[[], bool], what: str, *, within: float, every: float = 0.01
) -> None:
    """Waits until `holds()` is true, for `within` seconds at most, looking `every`.

    A listener runs on its own task, or on a worker thread in the sync way,
    and sets no event the test could wait on, so the wait polls it.

    Raises:
        AssertionError: `holds()` was still false after `within` seconds.
    """
    for _ in range(round(within / every)):
        if holds():
            return
        await anyio.sleep(every)
    message = f"{what} did not happen within {within} seconds"
    raise AssertionError(message)
