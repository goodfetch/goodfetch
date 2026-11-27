"""A followed read (ADR 0102), such as `run.progress`, as a sequence's reading.

Its first read is sent when the method that returns it is called, and
answers at once; each next one is sent once the program reads the value
before it, with that value's `position` as `after`, and the engine answers
it with the latest value once a newer one comes, so values the program did
not read in time coalesce. The value whose `last` is true ends it. A
sequence's credit is one read: the program's own pace sets when the next
is sent.

A paged read, such as `run.rejected_records`, is read the same way, one
page a read, each page handing its items in order; its credits count
items, so a next page is asked for only while the program has credit left,
though one page may hand more items than the credit left. TypeScript's
`followed.ts` is its twin.
"""

from __future__ import annotations

import collections
import logging
from typing import TYPE_CHECKING, Generic, TypeVar

import anyio

from goodfetch_collie_client.goodfetch_error import BaseGoodfetchError
from goodfetch_collie_client.host_logging import CLIENT_LOGGER

if TYPE_CHECKING:
    from collections.abc import Awaitable, Callable, Sequence

__all__ = ["PagedReading"]

_Page = TypeVar("_Page")
_Item = TypeVar("_Item")


class PagedReading(Generic[_Page, _Item]):
    """A paged read's items, which its task and a sequence's iterator share.

    A followed read is one whose every page is one item.
    """

    def __init__(
        self,
        read: Callable[[int | None], Awaitable[_Page]],
        *,
        items: Callable[[_Page], Sequence[_Item]],
        position: Callable[[_Page], int],
        last: Callable[[_Page], bool],
        credits: int = 1,
        regrant: int = 1,
    ) -> None:
        """Takes what sends one read after a position, and reads a page's own.

        `credits` is how many items may be read before the program reads
        any; `regrant`, how many the iterator hands out before it grants as
        many again.
        """
        self._read = read
        self._items = items
        self._position = position
        self._last = last
        self._credits = credits
        self.regrant = regrant
        """How many items the iterator hands out before it grants as many again."""
        self._granted = anyio.Event()
        self._reading = anyio.CancelScope()
        self.received: collections.deque[_Item] = collections.deque()
        self.ended = False
        """Whether its reads ended: nothing more is received."""
        self.closed = False
        """Whether the program stopped reading, or read it to its end."""
        self.failure: BaseGoodfetchError | None = None
        """What failed a read, unless the program stopped reading first."""
        self._changed = anyio.Event()

    async def changed(self) -> None:
        """Waits until an item comes, its reads end or it is closed."""
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
        """Lets `credits` more items be read."""
        self._credits += credits
        granted, self._granted = self._granted, anyio.Event()
        granted.set()

    async def run(self) -> None:
        """Sends each read while credit is left, until the last page or a failure."""
        try:
            with self._reading:
                await self._follow()
        except BaseGoodfetchError as error:
            if not self.closed:
                self.failure = error
        except Exception:
            # A spawned task has no caller to raise to.
            logging.getLogger(CLIENT_LOGGER).exception("invariant: a paged read failed")
        finally:
            self.ended = True
            self._wake()

    async def _follow(self) -> None:
        after: int | None = None
        while True:
            while self._credits <= 0:
                await self._granted.wait()
            page = await self._read(after)
            items = self._items(page)
            self._credits -= len(items)
            self.received.extend(items)
            self._wake()
            if self._last(page):
                return
            after = self._position(page)

    def _wake(self) -> None:
        changed, self._changed = self._changed, anyio.Event()
        changed.set()
