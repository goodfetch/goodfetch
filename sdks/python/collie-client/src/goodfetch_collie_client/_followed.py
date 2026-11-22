"""A followed read (ADR 0102), such as `run.progress`, as a sequence's reading.

Its first read is sent when the method that returns it is called, and
answers at once; each next one is sent once the program reads the value
before it, with that value's `position` as `after`, and the engine answers
it with the latest value once a newer one comes, so values the program did
not read in time coalesce. The value whose `last` is true ends it. A
sequence's credit is one read: the program's own pace sets when the next
is sent. TypeScript's `followed.ts` is its twin.
"""

from __future__ import annotations

import collections
import logging
from typing import TYPE_CHECKING, Generic, TypeVar

import anyio

from goodfetch_collie_client.goodfetch_error import BaseGoodfetchError
from goodfetch_collie_client.host_logging import CLIENT_LOGGER

if TYPE_CHECKING:
    from collections.abc import Awaitable, Callable

__all__ = ["FollowedReading"]

_Value = TypeVar("_Value")


class FollowedReading(Generic[_Value]):
    """A followed read's values, which its task and a sequence's iterator share."""

    regrant = 1
    """How many values the iterator hands out before it grants as many reads."""

    def __init__(
        self,
        read: Callable[[int | None], Awaitable[_Value]],
        *,
        position: Callable[[_Value], int],
        last: Callable[[_Value], bool],
    ) -> None:
        """Takes what sends one read after a position, and reads a value's own."""
        self._read = read
        self._position = position
        self._last = last
        self._credits = 1
        self._granted = anyio.Event()
        self._reading = anyio.CancelScope()
        self.received: collections.deque[_Value] = collections.deque()
        self.ended = False
        """Whether its reads ended: nothing more is received."""
        self.closed = False
        """Whether the program stopped reading, or read it to its end."""
        self.failure: BaseGoodfetchError | None = None
        """What failed a read, unless the program stopped reading first."""
        self._changed = anyio.Event()

    async def changed(self) -> None:
        """Waits until a value comes, its reads end or it is closed."""
        await self._changed.wait()

    def close(self) -> None:
        """Stops reading: the read in flight is cancelled. Idempotent."""
        if self.closed:
            return
        self.closed = True
        self.received.clear()
        self._reading.cancel()
        self._wake()

    async def grant(self, credits: int) -> None:
        """Lets `credits` more reads be sent."""
        self._credits += credits
        granted, self._granted = self._granted, anyio.Event()
        granted.set()

    async def run(self) -> None:
        """Sends each read once granted, until the last value or a failure."""
        try:
            with self._reading:
                await self._follow()
        except BaseGoodfetchError as error:
            if not self.closed:
                self.failure = error
        except Exception:
            # A spawned task has no caller to raise to.
            logging.getLogger(CLIENT_LOGGER).exception(
                "invariant: a followed read failed"
            )
        finally:
            self.ended = True
            self._wake()

    async def _follow(self) -> None:
        after: int | None = None
        while True:
            while self._credits == 0:
                await self._granted.wait()
            self._credits -= 1
            value = await self._read(after)
            self.received.append(value)
            self._wake()
            if self._last(value):
                return
            after = self._position(value)

    def _wake(self) -> None:
        changed, self._changed = self._changed, anyio.Event()
        changed.set()
