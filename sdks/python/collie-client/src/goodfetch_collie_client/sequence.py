"""A sequence's iterator (ADR 0081): a stream read to its end.

Such as a Run's Key-value store's keys. Its call is sent when the method
that returns it is called, in call order with the Run's other calls, and it
reads the items in order, granting the engine credits as they are read, so
an unread lossless sequence slows its producer. It is a plain class, not a
generator, so it closes synchronously and at once: leaving an `async with`
or `with` block, `close()`, `aclose()` and releasing it, which a bare
`async for … break` does in CPython, cancel its call, before any later call
of the Run. It has one reader: it is its own iterator. In the sync API,
`Sequence` reads its async twin on the Connection's private loop.
TypeScript's `sequence.ts` is its twin.
"""

from __future__ import annotations

import collections
import logging
import weakref
from typing import TYPE_CHECKING, Generic, TypeVar

import anyio

from goodfetch_collie_client.goodfetch_error import BaseGoodfetchError
from goodfetch_collie_client.host_logging import CLIENT_LOGGER
from goodfetch_collie_client.listeners import CREDITS, REGRANT, wait_for
from goodfetch_collie_client.portal import LoopStopped

if TYPE_CHECKING:
    from collections.abc import Callable, Mapping
    from typing import Self

    from goodfetch_collie_client.listeners import Order, Streams
    from goodfetch_collie_client.portal import PrivateLoop

_Item = TypeVar("_Item")


class Reading(Generic[_Item]):
    """A sequence's call and what it received, which its task and iterator share.

    It never holds the iterator, so releasing the iterator unread lets it go.
    """

    def __init__(
        self,
        connection: Streams,
        method: str,
        params: Mapping[str, object],
        item_of: Callable[[str, object], _Item | None],
        order: Order,
    ) -> None:
        """Takes its call, what reads an item from a notification, the Run's order."""
        self._connection = connection
        self._method = method
        self._params = params
        self._item_of = item_of
        self._order = order
        self._previous, self._sent = order.hold()
        self._closing: tuple[anyio.Event | None, anyio.Event] | None = None
        self._close_asked = anyio.Event()
        self._id: int | None = None
        self.received: collections.deque[_Item] = collections.deque()
        self.ended = False
        """Whether its call ended: nothing more is received."""
        self.closed = False
        """Whether the program stopped reading, or read it to its end."""
        self.failure: BaseGoodfetchError | None = None
        """What ended its call, unless the program stopped reading first.

        `attempt_ended`'s class included, which is outside `Exception` and
        reaches the reader as any failure does (ADR 0093).
        """
        self._changed = anyio.Event()

    async def changed(self) -> None:
        """Waits until an item comes, its call ends or it is closed."""
        await self._changed.wait()

    def close(self) -> None:
        """Stops reading: its call is cancelled after every earlier hold. Idempotent."""
        if self.closed:
            return
        self.closed = True
        self.received.clear()
        if not self.ended:
            self._closing = self._order.hold()
            self._close_asked.set()
        self._wake()

    async def grant(self, credits: int) -> None:
        """Grants the engine `credits` more items."""
        if self._id is None:
            return
        # A grant cut short would leave half a line on the transport.
        with anyio.CancelScope(shield=True):
            await self._connection.notify(
                "$/credit", {"id": self._id, "credits": credits}
            )

    async def run(self) -> None:
        """Sends the call once its turn came, and receives its items until it ends."""
        try:
            await wait_for(self._previous)
            if not self.closed:
                await self._call()
        except Exception:
            # A spawned task has no caller to raise to.
            logging.getLogger(CLIENT_LOGGER).exception(
                "invariant: %s's sequence failed", self._method
            )
        finally:
            self.ended = True
            self._sent.set()
            self._wake()
            await self._release_close()

    async def _call(self) -> None:
        written = anyio.Event()

        def sent(call_id: int) -> None:
            self._id = call_id
            written.set()

        async with anyio.create_task_group() as group:
            group.start_soon(self._grant_first, written)
            with anyio.CancelScope() as call:
                group.start_soon(self._cancel_when_closed, call)
                try:
                    await self._connection.stream(
                        self._method, self._params, self._notified, sent=sent
                    )
                except BaseGoodfetchError as error:
                    if not self.closed:
                        self.failure = error
            group.cancel_scope.cancel()

    async def _grant_first(self, written: anyio.Event) -> None:
        await written.wait()
        await self.grant(CREDITS)
        self._sent.set()

    async def _cancel_when_closed(self, call: anyio.CancelScope) -> None:
        await self._close_asked.wait()
        if self._closing is not None:
            await wait_for(self._closing[0])
        call.cancel()

    async def _release_close(self) -> None:
        """Sets the close's hold, after the one before it, with nothing left to send."""
        if self._closing is None:
            return
        previous, closed = self._closing
        with anyio.CancelScope(shield=True):
            await wait_for(previous)
        closed.set()

    def _notified(self, method: str, params: object) -> None:
        """Keeps an item the call carried; on the reading task, so it never blocks."""
        if self.closed:
            return
        item = self._item_of(method, params)
        if item is not None:
            self.received.append(item)
            self._wake()

    def _wake(self) -> None:
        changed, self._changed = self._changed, anyio.Event()
        changed.set()


class AsyncSequence(Generic[_Item]):
    """A sequence read to its end with `async for`; it closes at once however it stops.

    A program never constructs one: a method such as
    `run.key_value_store.keys()` returns it.
    """

    def __init__(self, reading: Reading[_Item]) -> None:
        """Takes its call, already spawned."""
        self._reading = reading
        self._release = weakref.finalize(self, reading.close)
        self._release.atexit = False
        self._read = 0
        """Items read since credits were last granted."""
        self._handed_out = False
        """Whether the last read handed out an item, which the next read counts."""

    def __aiter__(self) -> Self:
        """Itself: it has one reader."""
        return self

    async def __anext__(self) -> _Item:
        """The next item, once it comes.

        Raises:
            StopAsyncIteration: Every item is read, or the program stopped
                reading.
            GoodfetchError: The call failed, raised once every item it
                carried is read.
            BaseGoodfetchError: Foxhound's `AttemptEndedError`: the call's
                attempt ended (ADR 0093), raised in the same way.
        """
        reading = self._reading
        if self._handed_out:
            self._handed_out = False
            self._read += 1
            if self._read >= REGRANT and not reading.ended and not reading.closed:
                await reading.grant(self._read)
                self._read = 0
        while True:
            if reading.closed:
                raise StopAsyncIteration
            if reading.received:
                self._handed_out = True
                return reading.received.popleft()
            if reading.ended:
                failure = reading.failure
                self.close()
                if failure is not None:
                    raise failure
                raise StopAsyncIteration
            await reading.changed()

    def close(self) -> None:
        """Stops reading at once, cancelling the call before the Run's next one."""
        self._release()

    async def aclose(self) -> None:
        """Stops reading, as `close()` does."""
        self.close()

    async def __aenter__(self) -> Self:
        """Holds the iterator for an `async with` block, which closes it."""
        return self

    async def __aexit__(self, *_: object) -> None:
        """Closes the iterator."""
        self.close()


class Sequence(Generic[_Item]):
    """A sequence read to its end with `for`, in the sync API; any thread reads it.

    It reads its async twin on the Connection's private loop, and closes as
    it does. A program never constructs one.
    """

    def __init__(
        self,
        loop: PrivateLoop,
        sequence: AsyncSequence[_Item],
        stopped: Callable[[], Exception],
    ) -> None:
        """Takes the private loop, the async twin it reads there, and a stop's error."""
        self._loop = loop
        self._sequence = sequence
        self._stopped = stopped
        self._release = weakref.finalize(self, _close_on, loop, sequence)
        self._release.atexit = False

    def __iter__(self) -> Self:
        """Itself: it has one reader."""
        return self

    def __next__(self) -> _Item:
        """The next item, once it comes.

        Raises:
            StopIteration: Every item is read, or the program stopped reading.
            GoodfetchError: The call failed, raised once every item it
                carried is read.
            BaseGoodfetchError: Foxhound's `AttemptEndedError`: the call's
                attempt ended (ADR 0093), raised in the same way.
        """
        try:
            return self._loop.call(self._sequence.__anext__)
        except StopAsyncIteration:
            raise StopIteration from None
        except LoopStopped:
            # Its Connection closed the Run before stopping the loop.
            raise self._stopped() from None

    def close(self) -> None:
        """Stops reading at once, cancelling the call before the Run's next one."""
        self._release()

    def __enter__(self) -> Self:
        """Holds the iterator for a `with` block, which closes it."""
        return self

    def __exit__(self, *_: object) -> None:
        """Closes the iterator."""
        self.close()


def _close_on(loop: PrivateLoop, sequence: AsyncSequence[_Item]) -> None:
    """Closes `sequence` on `loop`, which owns it.

    Released on the loop's own thread, it cannot wait there: the async
    twin, released right after, closes itself.
    """

    async def closing() -> None:
        sequence.close()

    try:
        loop.call(closing)
    except LoopStopped:
        # Its Connection closed the Run, and the sequence with it; or this
        # is the loop's own thread.
        return
