"""Collie's Connection types (ADRs 0078, 0081, 0090 and 0095).

`AsyncCollie.connect()` and the sync `Collie.connect()` find Collie's
executable, start it as an Attached engine with exactly the program's
environment map, finish `hello`, naming Collie, and set the Connection's
Limits before they return.
Everything else is the client package's common Connection. The sync
`Collie` runs it on a private thread of its own, which `close()` joins.
TypeScript's `collie.ts` is its twin.
"""

from __future__ import annotations

import functools
import threading
from pathlib import Path
from typing import TYPE_CHECKING

from goodfetch_collie_client.connection import ClientConnection, closed_error
from goodfetch_collie_client.connection_type import AsyncConnection, Connection
from goodfetch_collie_client.generated.connection import (
    COLLIE_ENGINE_PATH_VARIABLE,
    COLLIE_EXECUTABLE,
)
from goodfetch_collie_client.generated.errors import ERROR_CLASSES
from goodfetch_collie_client.generated.models import Product
from goodfetch_collie_client.portal import LoopStopped, PrivateLoop
from goodfetch_collie_client.product_connection import (
    ProductPackage,
    open_attached,
    open_over,
)
from goodfetch_collie_client.run import AsyncRun, Run

if TYPE_CHECKING:
    from collections.abc import Awaitable, Callable, Generator, Iterable, Mapping

    from anyio.abc import ByteStream
    from goodfetch_collie_client.connection import EngineInfo
    from goodfetch_collie_client.limits import Limits

COLLIE = ProductPackage(
    needs=Product.collie,
    name="Collie",
    package="goodfetch-collie",
    executable=COLLIE_EXECUTABLE,
    engine_path_variable=COLLIE_ENGINE_PATH_VARIABLE,
    error_classes=ERROR_CLASSES,
    bin_folder=Path(__file__).parent / "bin",
)
"""What makes this package Collie's.

The Product its `hello` names, its executable and the one variable it reads.
"""


class AsyncCollie(AsyncConnection):
    """A Connection to an engine: the entry point of the async API.

    The engine is Collie's or Foxhound's (ADR 0095).
    Each one is independent, and many may be open in one process. Opened by
    `connect()`, never constructed directly; `async with` closes it.
    """

    def __init__(self, connection: ClientConnection) -> None:
        """Wraps an open Connection."""
        self._connection = connection

    @staticmethod
    def connect(
        *, env: Mapping[str, str] | None = None, limits: Limits | None = None
    ) -> Connecting:
        """Starts Collie's engine as an Attached engine and finishes `hello`.

        Await it, or open it with `async with`, which closes it. `env` is
        the environment map, read here and never at import: the process's
        own when left out. `limits` caps the Connection (ADR 0080); it only
        lowers what covers its work. An amount the SDK cannot turn exactly
        (`setting_invalid`), a missing executable (`engine_not_found`), a
        refused Product (`not_in_product`) or version, or a refused Limit
        raises here, with a `fix`, leaving no engine behind; so does a
        cancelled opening. The engine sends its logs from the effective
        level of `goodfetch.engine`, read here (ADR 0078).
        """
        return Connecting(
            functools.partial(open_attached, COLLIE, env=env, limits=limits)
        )

    @property
    def engine(self) -> EngineInfo:
        """The engine reached, read without a round trip.

        Its kind, Product, Product version and protocol version.
        """
        return self._connection.engine

    async def start(
        self,
        *,
        run_id: str | None = None,
        tags: Iterable[str] | None = None,
        limits: Limits | None = None,
    ) -> AsyncRun:
        """Leaving the scope or `close()` stops the Run with a Checkpoint.

        `finish()` ends it. Starts a Run and returns once it is started
        (ADR 0102). A start that resumed its Run is logged at info, and one
        that found it ended at warning, on the `goodfetch.client.run` logger.
        It tells the engine the effective level of `goodfetch.engine` first
        when that changed since the engine was last told.

        Args:
            run_id: Names the Run for good: a start with an existing Run ID
                resumes the Run if it was interrupted, or returns it as it
                ended (`started_as` says which); the engine generates one
                when it is left out. A Run another engine writes fails with
                `run_in_use`, one begun with other inputs with
                `run_conflict`, and an ID off its pattern with
                `run_id_invalid`.
            tags: Its tags: at most 16, each at most 64 characters
                (ADR 0036).
            limits: Caps the Run: a start that names it again lowers its
                Limits, never raises them (`limit_raise_refused`).
        """
        return await self._connection.start(run_id=run_id, tags=tags, limits=limits)

    async def close(self) -> None:
        """Closes the Connection; idempotent.

        Closes every Run it opened first, stopping one still running, cancels
        the calls in flight, then stops the Attached engine and waits for it.
        Any later call fails with `closed`.
        """
        await self._connection.close()


class Connecting:
    """An `AsyncCollie` being opened: await it, or open it with `async with`."""

    def __init__(self, opening: Callable[[], Awaitable[ClientConnection]]) -> None:
        """Takes what opens the Connection, run once it is awaited or entered."""
        self._opening = opening
        self._collie: AsyncCollie | None = None

    def __await__(self) -> Generator[object, None, AsyncCollie]:
        """Opens the Connection."""
        return self._open().__await__()

    async def __aenter__(self) -> AsyncCollie:
        """Opens the Connection for an `async with` block."""
        self._collie = await self._open()
        return self._collie

    async def __aexit__(self, *_: object) -> None:
        """Closes the Connection the block opened."""
        if self._collie is not None:
            await self._collie.close()

    async def _open(self) -> AsyncCollie:
        return AsyncCollie(await self._opening())


class Collie(Connection):
    """A Connection to an engine: the entry point of the sync API.

    The engine is Collie's or Foxhound's (ADR 0095).
    It runs on a private daemon thread of its own, never on the caller's
    event loop, so it works in a notebook cell, in Streamlit and in a sync
    tool's thread; called from `async def`, it blocks that loop, where
    `AsyncCollie` does not. Any number of threads may share it and its Runs
    (ADR 0081). Opened by `connect()`, never constructed directly; `with`
    closes it.
    """

    def __init__(self, loop: PrivateLoop, connection: ClientConnection) -> None:
        """Takes the private loop and the open Connection it drives there."""
        self._loop = loop
        self._connection = connection
        self._close_lock = threading.Lock()
        self._closed = False
        self._close_failure: Exception | None = None

    @staticmethod
    def connect(
        *, env: Mapping[str, str] | None = None, limits: Limits | None = None
    ) -> Collie:
        """Starts Collie's engine as an Attached engine and finishes `hello`.

        Open it with `with`, which closes it. `env` is the environment map,
        read here and never at import: the process's own when left out.
        `limits` caps the Connection (ADR 0080); it only lowers what covers
        its work. An amount the SDK cannot turn exactly (`setting_invalid`),
        a missing executable (`engine_not_found`), a refused Product
        (`not_in_product`) or version, or a refused Limit raises here, with a
        `fix`, leaving neither engine nor thread behind. The engine sends its
        logs from the effective level of `goodfetch.engine`, read here
        (ADR 0078).
        """
        return _open_sync(
            functools.partial(open_attached, COLLIE, env=env, limits=limits)
        )

    @property
    def engine(self) -> EngineInfo:
        """The engine reached, read without a round trip.

        Its kind, Product, Product version and protocol version.
        """
        return self._connection.engine

    def start(
        self,
        *,
        run_id: str | None = None,
        tags: Iterable[str] | None = None,
        limits: Limits | None = None,
    ) -> Run:
        """Leaving the scope or `close()` stops the Run with a Checkpoint.

        `finish()` ends it. Starts a Run and returns once it is started
        (ADR 0102). A start that resumed its Run is logged at info, and one
        that found it ended at warning, on the `goodfetch.client.run` logger.
        It tells the engine the effective level of `goodfetch.engine` first
        when that changed since the engine was last told.

        Args:
            run_id: Names the Run for good: a start with an existing Run ID
                resumes the Run if it was interrupted, or returns it as it
                ended (`started_as` says which); the engine generates one
                when it is left out. A Run another engine writes fails with
                `run_in_use`, one begun with other inputs with
                `run_conflict`, and an ID off its pattern with
                `run_id_invalid`.
            tags: Its tags: at most 16, each at most 64 characters
                (ADR 0036).
            limits: Caps the Run: a start that names it again lowers its
                Limits, never raises them (`limit_raise_refused`).
        """
        starting = functools.partial(
            self._connection.start,
            run_id=run_id,
            tags=None if tags is None else list(tags),
            limits=limits,
        )
        try:
            run = self._loop.call(starting)
        except LoopStopped:
            raise closed_error("run.start") from None
        return Run(self._loop, run)

    def close(self) -> None:
        """Closes the Connection and joins its thread; idempotent, from any thread.

        Closes every Run it opened first, stopping one still running, cancels
        the calls in flight, then stops the Attached engine and waits for it.
        Any later call fails with `closed`; a later close raises the first
        one's failure.
        """
        with self._close_lock:
            if not self._closed:
                try:
                    self._loop.call(self._connection.close)
                except LoopStopped:
                    # An interrupted close stopped the loop after the close ended.
                    pass
                except Exception as failure:
                    self._close_failure = failure
                finally:
                    self._closed = True
                    self._loop.stop()
        if self._close_failure is not None:
            raise self._close_failure


def async_connect_over(transport: ByteStream) -> Connecting:
    """Opens an `AsyncCollie` over `transport`, with no process behind it."""
    return Connecting(functools.partial(open_over, COLLIE, transport))


def connect_over(transport: ByteStream) -> Collie:
    """Opens a sync `Collie` over `transport`, with no process behind it.

    Its private loop drives `transport`, so a transport bound to another
    event loop does not fit.
    """
    return _open_sync(functools.partial(open_over, COLLIE, transport))


def _open_sync(opening: Callable[[], Awaitable[ClientConnection]]) -> Collie:
    """Opens a Connection on a private loop, stopped again when the opening fails."""
    loop = PrivateLoop()
    try:
        connection = loop.call(opening)
    except BaseException:
        loop.stop()
        raise
    return Collie(loop, connection)
