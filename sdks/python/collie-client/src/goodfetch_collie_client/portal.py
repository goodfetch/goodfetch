"""The private thread a sync Connection runs its async core on (ADRs 0038 and 0081).

One per Connection, started by `connect()` and joined by `close()`, never
process-wide. It runs its own event loop, so it never touches the caller's:
the sync API works in a notebook cell, in Streamlit and in a sync tool's
thread. It is a daemon thread, so a program that forgets `close()` still
exits. Any number of threads may call through it at once.
"""

from __future__ import annotations

import threading
from typing import TYPE_CHECKING, TypeVar

import anyio
from anyio.from_thread import BlockingPortal

if TYPE_CHECKING:
    import concurrent.futures
    from collections.abc import Awaitable, Callable

_T = TypeVar("_T")

THREAD_NAME = "goodfetch-collie"
"""The private thread's name, which a thread dump shows."""


class LoopStopped(Exception):
    """The private loop has stopped: its Connection is closed."""


class PrivateLoop:
    """An event loop on a daemon thread of its own."""

    def __init__(self) -> None:
        """Starts the thread and waits for its loop to run."""
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
        without waiting, and propagates unchanged (ADR 0081).

        Raises:
            LoopStopped: The loop has stopped, so its Connection is closed.
        """
        try:
            future: concurrent.futures.Future[_T] = self._portal.start_task_soon(
                function
            )
        except RuntimeError as stopped:
            raise LoopStopped from stopped
        try:
            return future.result()
        except BaseException:
            future.cancel()
            raise

    def stop(self) -> None:
        """Ends the loop once its calls are done, and joins the thread."""
        if threading.current_thread() is self._thread:
            message = "invariant: the private loop is never stopped from its own thread"
            raise RuntimeError(message)
        self._portal.call(self._portal.stop)
        self._thread.join()
