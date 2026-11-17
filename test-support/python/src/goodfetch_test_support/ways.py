"""Python's three APIs, each a named case of one test (ADR 0069).

A Client conformance case is written once, against the async API, and
`three_ways` runs it async on asyncio, async on trio and through the
generated sync API, so the three stay equal.
"""

from __future__ import annotations

import enum
import functools
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
        worker thread, as a sync caller outside any event loop would call it.
        """
        if self is not Way.SYNC:
            return async_api
        # The generated sync API has each async method's name and parameters,
        # so the proxy answers the async API's interface.
        return cast("_Api", _InWorkerThread(sync_api))


class _InWorkerThread:
    """A sync API whose methods are awaited, each call run in a worker thread."""

    def __init__(self, sync_api: object) -> None:
        self._sync_api = sync_api

    def __getattr__(self, name: str) -> Callable[..., Awaitable[object]]:
        method = cast("Callable[..., object]", getattr(self._sync_api, name))

        async def call(*args: object, **kwargs: object) -> object:
            return await anyio.to_thread.run_sync(
                functools.partial(method, *args, **kwargs)
            )

        return call


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
