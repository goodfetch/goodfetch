"""Collie's testing kit (ADRs 0069, 0090 and 0105).

`AsyncTestEngine` and `TestEngine` start a real Collie engine for a test, an
Attached engine, whose connection is an `AsyncCollie` or a `Collie`, with
the client package's `TestEngineOptions`. `async_connect_over` and
`connect_over` open Collie's Connection over a transport the test holds,
such as the Scripted engine's in-memory player, so a program's tests run its
code with no engine process: the same Connection as `connect()`'s. Importing
it reads, patches and logs nothing (ADR 0078). TypeScript's
`@goodfetch/collie/testing` is its twin.
"""

from __future__ import annotations

from typing import TYPE_CHECKING

import anyio
import anyio.to_thread
from goodfetch_collie_client.generated.connection import COLLIE_ENGINE_PATH_VARIABLE
from goodfetch_collie_client.testing import (
    TestEngineOptions,
    make_home,
    parts_of,
    remove_home,
)

from goodfetch_collie.collie import (
    AsyncCollie,
    Collie,
    async_connect_over,
    connect_over,
)

if TYPE_CHECKING:
    from collections.abc import Generator
    from typing import Self, Unpack

__all__ = [
    "AsyncTestEngine",
    "StartingTestEngine",
    "TestEngine",
    "TestEngineOptions",
    "async_connect_over",
    "connect_over",
]


class AsyncTestEngine:
    """A real Collie engine for a test, in the async API.

    An Attached engine in a temporary Goodfetch home, removed when it closes,
    with a cleared environment, `egress.offline` on, short durations and no
    project file unless the test names one. Browsers come from the caller's
    Browser cache, `GOODFETCH_BROWSERS_PATH`'s or the caller's Goodfetch
    home's, which a missing browser leaves as it was: `egress.offline` fails
    its launch with `artifact_unavailable_offline`, whose `fix` names
    `goodfetch-collie browser install`. Opened by `start()`; `async with`
    closes it.
    """

    __test__ = False
    """Not a test class, though a test module imports it: pytest never collects it."""

    def __init__(self, connection: AsyncCollie, home: str) -> None:
        """Takes the open connection and the temporary home it removes on close."""
        self._connection = connection
        self._home = home

    @staticmethod
    def start(**options: Unpack[TestEngineOptions]) -> StartingTestEngine:
        """Starts a test engine with `options`: await it, or open it with `async with`.

        It raises what `AsyncCollie.connect()` raises, such as
        `setting_invalid` for a project file that is not there, leaving no
        temporary home behind.
        """
        return StartingTestEngine(options)

    @property
    def connection(self) -> AsyncCollie:
        """The connection to the engine, on which the code under test runs unchanged."""
        return self._connection

    async def close(self) -> None:
        """Closes the connection, then removes the temporary Goodfetch home.

        The home is removed even when closing the connection raises.
        """
        try:
            await self._connection.close()
        finally:
            with anyio.CancelScope(shield=True):
                await anyio.to_thread.run_sync(remove_home, self._home)

    async def __aenter__(self) -> Self:
        """The test engine, for an `async with` block that closes it."""
        return self

    async def __aexit__(self, *_: object) -> None:
        """Closes the test engine."""
        await self.close()


class StartingTestEngine:
    """An `AsyncTestEngine` being started: await it, or open it with `async with`."""

    def __init__(self, options: TestEngineOptions) -> None:
        """Takes the options it starts the engine with, once awaited or entered."""
        self._options = options
        self._engine: AsyncTestEngine | None = None

    def __await__(self) -> Generator[object, None, AsyncTestEngine]:
        """Starts the test engine."""
        return self._start().__await__()

    async def __aenter__(self) -> AsyncTestEngine:
        """Starts the test engine for an `async with` block."""
        self._engine = await self._start()
        return self._engine

    async def __aexit__(self, *_: object) -> None:
        """Closes the test engine the block started."""
        if self._engine is not None:
            await self._engine.close()

    async def _start(self) -> AsyncTestEngine:
        home = await anyio.to_thread.run_sync(make_home)
        parts = parts_of(self._options, home, COLLIE_ENGINE_PATH_VARIABLE)
        try:
            connection = await AsyncCollie.connect(
                env=parts.env,
                engine="attached",
                settings=parts.settings,
                project=parts.project,
            )
        except BaseException:
            with anyio.CancelScope(shield=True):
                await anyio.to_thread.run_sync(remove_home, home)
            raise
        return AsyncTestEngine(connection, home)


class TestEngine:
    """A real Collie engine for a test, in the sync API, as `AsyncTestEngine` is.

    Opened by `start()`; `with` closes it.
    """

    __test__ = False
    """Not a test class, though a test module imports it: pytest never collects it."""

    def __init__(self, connection: Collie, home: str) -> None:
        """Takes the open connection and the temporary home it removes on close."""
        self._connection = connection
        self._home = home

    @staticmethod
    def start(**options: Unpack[TestEngineOptions]) -> TestEngine:
        """Starts a test engine with `options`, as `AsyncTestEngine.start()` does."""
        home = make_home()
        parts = parts_of(options, home, COLLIE_ENGINE_PATH_VARIABLE)
        try:
            connection = Collie.connect(
                env=parts.env,
                engine="attached",
                settings=parts.settings,
                project=parts.project,
            )
        except BaseException:
            remove_home(home)
            raise
        return TestEngine(connection, home)

    @property
    def connection(self) -> Collie:
        """The connection to the engine, on which the code under test runs unchanged."""
        return self._connection

    def close(self) -> None:
        """Closes the connection, then removes the temporary Goodfetch home.

        The home is removed even when closing the connection raises.
        """
        try:
            self._connection.close()
        finally:
            remove_home(self._home)

    def __enter__(self) -> Self:
        """The test engine, for a `with` block that closes it."""
        return self

    def __exit__(self, *_: object) -> None:
        """Closes the test engine."""
        self.close()
