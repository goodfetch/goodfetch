"""The private thread a sync Connection runs its async core on (ADRs 0038 and 0081).

One per Connection, started by `connect()` and joined by `close()`, never
process-wide. It runs its own event loop, so it never touches the caller's:
the sync API works in a notebook cell, in Streamlit and in a sync tool's
thread. It is a daemon thread, so a program that forgets `close()` still
exits. Any number of threads may call through it at once. A forked child
has none of it: a call there fails with `connection_forked` (ADR 0078).
"""

from __future__ import annotations

import threading
from dataclasses import dataclass
from typing import TYPE_CHECKING, TypeVar

import anyio
from anyio.from_thread import BlockingPortal

from goodfetch_collie_client.goodfetch_error import BaseGoodfetchError
from goodfetch_collie_client.opening_process import OpeningProcess

if TYPE_CHECKING:
    import concurrent.futures
    from collections.abc import Awaitable, Callable

_T = TypeVar("_T")

THREAD_NAME = "goodfetch-collie"
"""The private thread's name, which a thread dump shows."""


class LoopStopped(Exception):
    """The private loop has stopped: its Connection is closed."""


@dataclass(frozen=True)
class _Raised:
    """An error outside `Exception` a call raised, carried back as its result."""

    error: BaseGoodfetchError


class PrivateLoop:
    """An event loop on a daemon thread of its own."""

    def __init__(self) -> None:
        """Starts the thread and waits for its loop to run."""
        self._opener = OpeningProcess()
        ready = threading.Event()
        holder: list[BlockingPortal] = []

        async def serve() -> None:
            async with BlockingPortal() as portal:
                holder.append(portal)
                ready.set()
                await portal.sleep_until_stopped()

        self._thread = threading.Thread(
            target=anyio.run, args=(serve,), name=THREAD_NAME, daemon=True
        )
        self._thread.start()
        ready.wait()
        self._portal = holder[0]

    def call(self, function: Callable[[], Awaitable[_T]]) -> _T:
        """Runs `function` on the loop and waits for its result.

        An exception raised in the waiting thread, such as Ctrl+C's
        `KeyboardInterrupt`, cancels the call, which sends `$/cancel`
        without waiting, and propagates unchanged (ADR 0081). The portal
        would raise an error outside `Exception` in the loop's thread too,
        which ends it, so `attempt_ended`'s comes back as a value and is
        raised here alone (ADR 0093).

        Raises:
            LoopStopped: The loop has stopped, so its Connection is closed.
            ConnectionForkedError: Another process than the one that started
                the loop, such as a forked child, which has no loop thread,
                made the call.
            BaseGoodfetchError: Foxhound's `AttemptEndedError`: `function`
                raised `attempt_ended`, whose class is outside `Exception`
                (ADR 0093).
        """
        self._opener.ensure_current()

        async def kept() -> _T | _Raised:
            try:
                return await function()
            except BaseGoodfetchError as error:
                if isinstance(error, Exception):
                    raise
                return _Raised(error)

        try:
            future: concurrent.futures.Future[_T | _Raised] = (
                self._portal.start_task_soon(kept)
            )
        except RuntimeError as stopped:
            raise LoopStopped from stopped
        try:
            result = future.result()
        except BaseException:
            future.cancel()
            raise
        if isinstance(result, _Raised):
            raise result.error
        return result

    def stop(self) -> None:
        """Ends the loop once its calls are done, and joins the thread.

        In another process than the one that started it, such as a forked
        child, it does nothing: the thread is the opener's alone.
        """
        if not self._opener.is_current():
            return
        if threading.current_thread() is self._thread:
            message = "invariant: the private loop is never stopped from its own thread"
            raise RuntimeError(message)
        self._portal.call(self._portal.stop)
        self._thread.join()
