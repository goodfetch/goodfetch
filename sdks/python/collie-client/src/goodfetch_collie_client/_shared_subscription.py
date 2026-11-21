"""The Connection's own Subscription (ADRs 0031 and 0081).

One `events.subscribe`, sent when the first listener whose events the
engine publishes on its event stream joins, such as a Session's, and
cancelled once the last leaves. Each event's credit goes back as it goes
to the readers that take it, since each listener's own queue holds it or
loses it (`listeners`), so a slow listener slows no other; should the
stream still fall behind, the engine sends `events.resync` rather than
wait, and the events in between are gone, which is logged. TypeScript's
`shared-subscription.ts` is its twin.
"""

from __future__ import annotations

import logging
import math
from typing import TYPE_CHECKING

import anyio

from goodfetch_collie_client.goodfetch_error import GoodfetchError
from goodfetch_collie_client.host_logging import CLIENT_LOGGER
from goodfetch_collie_client.listeners import (
    CREDITS,
    EVENTS,
    REGRANT,
    Reader,
    Regranting,
    batch_of,
    spawner,
)

if TYPE_CHECKING:
    from goodfetch_collie_client.listeners import Streams

RESYNC = "events.resync"
"""The event that says the engine no longer held the Subscription's position."""

_logger = logging.getLogger(CLIENT_LOGGER)


class SharedSubscription:
    """One Connection's Subscription, shared by the listeners that read it."""

    def __init__(self, connection: Streams, listener_queue: int) -> None:
        """Sends nothing until a reader joins.

        `listener_queue`, already checked, bounds each listener's own queue.
        """
        self._connection = connection
        self.listener_queue = listener_queue
        """How many events each listener's own queue holds, the one it has included."""
        self._readers: dict[Reader, None] = {}
        self._current: _Open | None = None

    async def join(self, reader: Reader) -> None:
        """Adds `reader`, sending the Subscription with credits when none is in flight.

        It returns once they are written, or once the Subscription ended,
        which `reader` then heard.
        """
        self._readers[reader] = None
        current = self._current
        if current is None:
            current = _Open(self, self._connection)
            self._current = current
            spawner()(current.run)
        await current.ready.wait()

    async def leave(self, reader: Reader) -> None:
        """Removes `reader`; once none is left, cancels the Subscription.

        It returns once the cancel is written. Removing it again changes
        nothing.
        """
        if reader not in self._readers:
            return
        del self._readers[reader]
        if self._readers:
            return
        current, self._current = self._current, None
        if current is None:
            return
        current.stop()
        with anyio.CancelScope(shield=True):
            await current.done.wait()

    def received(self, current: _Open, params: object) -> None:
        """Gives each event of a batch's credit back, then hands it to its readers."""
        for event, data in batch_of(params):
            if event == RESYNC:
                _logger.warning(
                    "The Connection's Subscription fell behind: the events"
                    " published since its last read are gone, so a Session's"
                    " listener may have missed pages its `pages()` lists"
                )
            current.settle()
            for reader in list(self._readers):
                if reader.takes(event, data):
                    reader.take(data)

    def ended(self, current: _Open, error: GoodfetchError | None) -> None:
        """The Subscription ended unasked: each reader hears it.

        The next to join sends another.
        """
        if self._current is not current:
            return
        self._current = None
        readers = list(self._readers)
        self._readers = {}
        for reader in readers:
            reader.ended(error)


class _Open:
    """The Subscription in flight: its call, its credits and its cancel."""

    def __init__(self, shared: SharedSubscription, connection: Streams) -> None:
        self._shared = shared
        self._connection = connection
        self._id: int | None = None
        self._stopping = False
        self._regranting = Regranting()
        self._grants, self._granted = anyio.create_memory_object_stream[int](math.inf)
        self._call = anyio.CancelScope()
        self.ready = anyio.Event()
        """Set once its first credits are written, or once it ended."""
        self.done = anyio.Event()
        """Set once it ended, and its cancel, if asked, is written."""

    def stop(self) -> None:
        """Cancels it once the credits being written are: it grants nothing more."""
        self._stopping = True
        self._grants.close()

    def settle(self) -> None:
        """Counts one event settled, granting as many again every `REGRANT`."""
        if not self._regranting.settle() or self._stopping:
            return
        try:
            self._grants.send_nowait(REGRANT)
        except (anyio.ClosedResourceError, anyio.BrokenResourceError):
            # It ended: nothing more is granted.
            return

    async def run(self) -> None:
        """Sends `events.subscribe` and grants its credits until it ends."""
        error: GoodfetchError | None = None
        try:
            async with anyio.create_task_group() as group:
                group.start_soon(self._grant_all)
                with self._call:
                    try:
                        await self._connection.stream(
                            "events.subscribe", {}, self._notified, sent=self._sent
                        )
                    except GoodfetchError as failure:
                        error = failure
                group.cancel_scope.cancel()
        except Exception:
            # A spawned task has no caller to raise to.
            _logger.exception("invariant: the Connection's Subscription failed")
        finally:
            self._grants.close()
            # Its readers hear it ended, so the next to join sends another.
            if not self._stopping:
                self._shared.ended(self, error)
            self.ready.set()
            self.done.set()

    def _sent(self, call_id: int) -> None:
        """Queues the first credits, now that the call is written."""
        self._id = call_id
        try:
            self._grants.send_nowait(CREDITS)
        except (anyio.ClosedResourceError, anyio.BrokenResourceError):
            # Stopped before it was written: it grants nothing.
            return

    async def _grant_all(self) -> None:
        """Writes each grant in turn; once stopped, cancels the call after them."""
        with self._granted:
            async for credits in self._granted:
                if self._stopping or self._id is None:
                    continue
                # A grant cut short would leave half a line on the transport.
                with anyio.CancelScope(shield=True):
                    await self._connection.notify(
                        "$/credit", {"id": self._id, "credits": credits}
                    )
                self.ready.set()
        self._call.cancel()

    def _notified(self, method: str, params: object) -> None:
        """Hands a batch on; it runs on the reading task, so never blocks."""
        if method == EVENTS and not self._stopping:
            self._shared.received(self, params)
