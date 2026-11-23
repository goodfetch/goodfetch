"""The private thread a sync Connection runs its async core on (ADRs 0038 and 0081).

One per Connection, started by `connect()` and joined by `close()`, never
process-wide. It runs its own event loop, so it never touches the caller's:
the sync API works in a notebook cell, in Streamlit and in a sync tool's
thread. It is a daemon thread, so a program that forgets `close()` still
exits. Any number of threads may call through it at once. A forked child
has none of it: a call there fails with `connection_forked` (ADR 0078).
"""

from __future__ import annotations

import concurrent.futures
import contextlib
import threading
from contextvars import ContextVar
from dataclasses import dataclass
from typing import TYPE_CHECKING, TypeVar

import anyio
from anyio.from_thread import BlockingPortal

from goodfetch_collie_client._interruption import Interruption
from goodfetch_collie_client._opening_process import OpeningProcess
from goodfetch_collie_client.goodfetch_error import BaseGoodfetchError

if TYPE_CHECKING:
    from collections.abc import Awaitable, Callable, Generator

__all__ = ["CallGroup", "LoopStopped", "PrivateLoop"]

_T = TypeVar("_T")

THREAD_NAME = "goodfetch-collie"
"""The private thread's name, which a thread dump shows."""

WAIT_STEP_SECONDS = 0.1
"""How long a sync call's wait blocks before its thread checks for signals.

A signal's handler runs in the main thread, and only between waits: a
Ctrl+C that lands on another thread, as Windows's console delivers it,
would never end an untimed wait, which CPython makes interruptible on
Windows only from 3.14. The step bounds how late `KeyboardInterrupt`
reaches a blocked call on every OS; a call that ends wakes the wait at
once, so it adds nothing to the call's own time (ADR 0081).
"""


class LoopStopped(Exception):
    """The private loop has stopped: its Connection is closed."""


@dataclass(frozen=True)
class _Raised:
    """An error outside `Exception` a call raised, carried back as its result."""

    error: BaseGoodfetchError


class CallGroup:
    """The calls a thread makes through a private loop, which one cancel stops.

    Such as a sync crawl handler's, on a worker thread (ADR 0093): each of
    its calls runs as a task of the loop, outside the cancel scope of the
    task that started the thread, so closing its Run cancels them through
    the group. Once cancelled, a call in flight and each later one fail
    with `stopped`'s error. The calls made in `bound()`, and on the threads
    started there, which copy its context, join the group.
    """

    def __init__(self, stopped: Callable[[], Exception]) -> None:
        """Takes the error each call fails with once the group is cancelled."""
        self._stopped = stopped
        self._scopes: set[anyio.CancelScope] = set()
        self._cancelled = False

    @contextlib.contextmanager
    def bound(self) -> Generator[None]:
        """Joins the calls made in the block, and on the threads it starts, to it."""
        token = _bound_group.set(self)
        try:
            yield
        finally:
            _bound_group.reset(token)

    def cancel(self) -> None:
        """Cancels each call in flight, and fails each later one; on the loop."""
        self._cancelled = True
        for scope in self._scopes:
            scope.cancel()

    async def run(self, function: Callable[[], Awaitable[_T]]) -> _T:
        """Runs one call of the group on the loop, unless the group is cancelled.

        Raises:
            Exception: `stopped`'s error, once the group is cancelled.
        """
        if self._cancelled:
            raise self._stopped()
        with anyio.CancelScope() as scope:
            self._scopes.add(scope)
            try:
                return await function()
            finally:
                self._scopes.discard(scope)
        raise self._stopped()


_bound_group: ContextVar[CallGroup | None] = ContextVar(
    "goodfetch_call_group", default=None
)
"""The group the calls made in this context join; none outside `CallGroup.bound()`."""


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
        without waiting, and propagates unchanged (ADR 0081); the wait
        is timed, in steps of `WAIT_STEP_SECONDS`, so a Ctrl+C that lands
        on another thread, as on Windows, still ends it, and before it
        propagates, the call's `Interruption` puts that `$/cancel` ahead of
        any request sent after it (issue 3517). The portal
        would raise an error outside `Exception` in the loop's thread too,
        which ends it, so `attempt_ended`'s comes back as a value and is
        raised here alone (ADR 0093). The portal runs `function` in a copy
        of the calling thread's context, so each request it makes carries
        that thread's Trace context (ADR 0132), as
        `test_trace_context.py`'s two threads show, and joins the
        `CallGroup` that context is bound to, if any.

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
        interruption = Interruption()

        async def kept() -> _T | _Raised:
            interruption.govern()
            group = _bound_group.get()
            try:
                return await (function() if group is None else group.run(function))
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
            while not future.done():
                concurrent.futures.wait((future,), timeout=WAIT_STEP_SECONDS)
            result = future.result()
        except BaseException:
            self._interrupt(future, interruption)
            raise
        if isinstance(result, _Raised):
            raise result.error
        return result

    def _interrupt(
        self, future: concurrent.futures.Future[_T], interruption: Interruption
    ) -> None:
        """Interrupts the call, then cancels it, both on the loop, waiting for neither.

        The caller's thread waits for the loop to run both, as it would for
        the cancellation alone, not for the call's task to notice it.
        """

        def on_the_loop() -> None:
            interruption.interrupt()
            future.cancel()

        try:
            self._portal.call(on_the_loop)
        except RuntimeError:
            # The loop stopped, and the call's task with it.
            return

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
