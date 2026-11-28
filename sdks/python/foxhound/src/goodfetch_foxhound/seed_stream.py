"""Streams seeds to a running crawl (`run.add_seeds`, ADRs 0024 and 0102).

Drives `run.add_seeds` under the engine's `$/credit` grants, sending batches
of at most 64 seeds in `$/seeds` notifications and yielding each seed's
`Enqueued` answer from `$/enqueued` in order without holding unbounded seeds
in memory. TypeScript's `seed-stream.ts` is its twin.
"""

from __future__ import annotations

import collections
import logging
from collections.abc import AsyncIterable, AsyncIterator, Iterable, Iterator
from typing import TYPE_CHECKING, cast

import anyio
from goodfetch_collie_client.caller import result_of
from goodfetch_collie_client.goodfetch_error import BaseGoodfetchError
from goodfetch_collie_client.host_logging import CLIENT_LOGGER
from goodfetch_collie_client.listeners import wait_for

from goodfetch_foxhound.crawl import check_startable
from goodfetch_foxhound.generated.models import (
    Enqueued,
    RequestSeed,
    RunAddSeedsParams,
    SitemapSeed,
    SyndicationSeed,
)

if TYPE_CHECKING:
    from collections.abc import Awaitable, Callable

    from goodfetch_collie_client.listeners import Order, Streams
    from pydantic import BaseModel

    from goodfetch_foxhound.crawl import AsyncCrawl, Crawl

SEED_BATCH_MAX = 64
"""Max seeds in one `$/seeds` batch, staying under `SEED_CREDITS` (256)."""

MAX_QUEUED_ANSWERS = 256
"""Max unread `Enqueued` answers buffered before pausing further `$/seeds` batches."""

CREDIT = "$/credit"
ENQUEUED = "$/enqueued"
SEEDS = "$/seeds"


def _wire_model(model: BaseModel) -> dict[str, object]:
    """Serializes `model` to wire JSON omitting unset fields."""
    return model.model_dump(
        mode="json", by_alias=True, exclude_unset=True, warnings=False
    )


async def _close_source(
    source: Iterator[RequestSeed | SitemapSeed | SyndicationSeed]
    | AsyncIterator[RequestSeed | SitemapSeed | SyndicationSeed],
) -> None:
    """Closes a sync or async seed iterator on early stop or error."""
    aclose = getattr(source, "aclose", None)
    if callable(aclose):
        try:
            await cast("Callable[[], Awaitable[object]]", aclose)()
        except Exception:
            logging.getLogger(CLIENT_LOGGER).debug(
                "seed source aclose raised while stopping", exc_info=True
            )
        return
    close = getattr(source, "close", None)
    if callable(close):
        try:
            cast("Callable[[], object]", close)()
        except Exception:
            logging.getLogger(CLIENT_LOGGER).debug(
                "seed source close raised while stopping", exc_info=True
            )


class SeedStreamReading:
    """Implements `Reads[Enqueued]` for bidirectional `run.add_seeds`."""

    regrant = 16
    """How many answers the iterator hands out before waking the pump."""

    def __init__(
        self,
        connection: Streams,
        params: RunAddSeedsParams,
        crawl: AsyncCrawl | Crawl | None,
        source: Iterable[RequestSeed | SitemapSeed | SyndicationSeed]
        | AsyncIterable[RequestSeed | SitemapSeed | SyndicationSeed],
        order: Order,
    ) -> None:
        """Takes connection, `run.add_seeds` params, crawl, seed source and order."""
        self._connection = connection
        self._params = _wire_model(params)
        self._crawl = crawl
        self._source = source
        self._order = order
        self._previous, self._sent = order.hold()
        self._closing: tuple[anyio.Event | None, anyio.Event] | None = None
        self._close_asked = anyio.Event()
        self._id: int | None = None
        self._credits = 0
        self._pump_wake = anyio.Event()
        self.received: collections.deque[Enqueued] = collections.deque()
        self.ended = False
        self.closed = False
        self.failure: BaseException | None = None
        self._changed = anyio.Event()

    async def changed(self) -> None:
        """Waits until an answer arrives, the call ends or it is closed."""
        await self._changed.wait()

    def close(self) -> None:
        """Stops streaming and cancels the in-flight call. Idempotent."""
        if self.closed:
            return
        self.closed = True
        self.received.clear()
        self._cancel()
        self._wake_pump()
        self._wake()

    def _cancel(self) -> None:
        if not self.ended and self._closing is None:
            self._closing = self._order.hold()
            self._close_asked.set()

    async def grant(self, credits: int) -> None:
        """Wakes the pump once the consumer drains buffered answers."""
        _ = credits
        self._wake_pump()

    async def run(self) -> None:
        """Sends `run.add_seeds` in call order and streams batches under credits."""
        try:
            await wait_for(self._previous)
            if not self.closed:
                await self._call()
        except Exception:
            logging.getLogger(CLIENT_LOGGER).exception(
                "invariant: run.add_seeds stream failed"
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
            self._sent.set()

        async with anyio.create_task_group() as group:
            with anyio.CancelScope() as call:
                group.start_soon(self._cancel_when_closed, call)
                group.start_soon(self._pump, written, call)
                try:
                    await self._connection.stream(
                        "run.add_seeds", self._params, self._notified, sent=sent
                    )
                except BaseGoodfetchError as error:
                    if not self.closed and self.failure is None:
                        self.failure = error
            group.cancel_scope.cancel()

    async def _pump(self, written: anyio.Event, call: anyio.CancelScope) -> None:
        """Reads seeds from `self._source` as credits and queue room allow."""
        await written.wait()
        if self.closed or self._id is None:
            return
        async_iter: (
            AsyncIterator[RequestSeed | SitemapSeed | SyndicationSeed] | None
        ) = None
        sync_iter: Iterator[RequestSeed | SitemapSeed | SyndicationSeed] | None = None
        if isinstance(self._source, AsyncIterable):
            async_iter = self._source.__aiter__()
            closing: (
                AsyncIterator[RequestSeed | SitemapSeed | SyndicationSeed]
                | Iterator[RequestSeed | SitemapSeed | SyndicationSeed]
            ) = async_iter
        else:
            sync_iter = iter(self._source)
            closing = sync_iter
        ended = False
        try:
            while not ended and not self.closed:
                while (
                    self._credits <= 0 or len(self.received) >= MAX_QUEUED_ANSWERS
                ) and not self.closed:
                    await self._pump_wake.wait()
                if self.closed:
                    break
                limit = min(self._credits, SEED_BATCH_MAX)
                batch: list[RequestSeed | SitemapSeed | SyndicationSeed] = []
                while len(batch) < limit and not self.closed:
                    try:
                        if async_iter is not None:
                            candidate = await async_iter.__anext__()
                        elif sync_iter is not None:
                            candidate = next(sync_iter)
                        else:
                            ended = True
                            break
                    except (StopAsyncIteration, StopIteration):
                        ended = True
                        break
                    if self._crawl is not None:
                        check_startable(self._crawl, [candidate])
                    batch.append(candidate)
                if self.closed:
                    break
                if not batch and not ended:
                    continue
                self._credits -= len(batch)
                payload: dict[str, object] = {
                    "id": self._id,
                    "seeds": [_wire_model(item) for item in batch],
                }
                if ended:
                    payload["end"] = True
                with anyio.CancelScope(shield=True):
                    await self._connection.notify(SEEDS, payload)
        except Exception as error:
            if not self.closed and self.failure is None:
                self.failure = error
                call.cancel()
                self._wake()
        finally:
            with anyio.CancelScope(shield=True):
                await _close_source(closing)

    async def _cancel_when_closed(self, call: anyio.CancelScope) -> None:
        await self._close_asked.wait()
        if self._closing is not None:
            await wait_for(self._closing[0])
        call.cancel()

    async def _release_close(self) -> None:
        if self._closing is None:
            return
        previous, closed = self._closing
        with anyio.CancelScope(shield=True):
            await wait_for(previous)
        closed.set()

    def _notified(self, method: str, params: object) -> None:
        if self.closed or self.failure is not None or not isinstance(params, dict):
            return
        raw = cast("dict[str, object]", params)
        if method == CREDIT:
            granted = raw.get("credits")
            if isinstance(granted, int) and granted > 0:
                self._credits += granted
                self._wake_pump()
            return
        if method == ENQUEUED:
            answers = raw.get("answers")
            if isinstance(answers, list):
                for item in cast("list[object]", answers):
                    self.received.append(result_of(Enqueued, item))
                self._wake()

    def _wake_pump(self) -> None:
        wake, self._pump_wake = self._pump_wake, anyio.Event()
        wake.set()

    def _wake(self) -> None:
        changed, self._changed = self._changed, anyio.Event()
        changed.set()
