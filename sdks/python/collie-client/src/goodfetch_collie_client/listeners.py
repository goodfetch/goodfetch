"""A listener's subscription (ADR 0081).

Its `on` call is sent when `on` is called and stays in flight, and the
engine sends the event on the call itself, in `$/events` batches, never
more than the credits granted. The listener gets the events one at a time,
in order, and an awaitable it returns is awaited before the next; a credit
goes back only once it settled, so a slow listener slows a lossless
producer. One that throws is removed: its call is cancelled and its value
hears the failure. Closing the value, or its Run, stops its listeners:
one running is cancelled and awaited, as in a task group the value owns.

`on` and a `Subscription`'s `close()` are plain methods, so what they send
is written by a task of its own, which runs beside the program's tasks,
never in them: on asyncio a task of the running loop, on Trio a system
task. An `Order` keeps what they send in the order the program called, and
before every later call of the Run. TypeScript's `listeners.ts` is its twin.
"""

from __future__ import annotations

import asyncio
import contextvars
import inspect
import logging
import math
from dataclasses import dataclass
from typing import TYPE_CHECKING, Any, Generic, Protocol, TypeVar, cast

import anyio

from goodfetch_collie_client.caller import result_of
from goodfetch_collie_client.framing import object_members
from goodfetch_collie_client.goodfetch_error import GoodfetchError
from goodfetch_collie_client.handle import Subscription
from goodfetch_collie_client.host_logging import CLIENT_LOGGER

if TYPE_CHECKING:
    from collections.abc import Callable, Coroutine, Mapping

    from anyio.streams.memory import (
        MemoryObjectReceiveStream,
        MemoryObjectSendStream,
    )
    from pydantic import BaseModel

    from goodfetch_collie_client.handle import AsyncListener

    _Work = Callable[[], Coroutine[Any, Any, None]]

_Model = TypeVar("_Model", bound="BaseModel")

CREDITS = 16
"""The credits granted once the call is sent: at most this many events wait."""

REGRANT = 8
"""How many events its listener gets through before as many are granted again."""

EVENTS = "$/events"
"""The notification that carries a batch of the call's events."""

_running: set[asyncio.Task[None]] = set()
"""The subscriptions' asyncio tasks, held until they end: the loop holds them weakly."""

_heard_by: contextvars.ContextVar[object] = contextvars.ContextVar(
    "goodfetch_heard_by", default=None
)
"""The subscription whose listener the current code runs in, if any."""


class Streams(Protocol):
    """What a subscription sends its call through: its Connection."""

    async def stream(
        self,
        method: str,
        params: Mapping[str, object],
        notified: Callable[[str, object], None],
        *,
        sent: Callable[[int], None] | None = None,
    ) -> object:
        """Sends `method`, whose notifications `notified` hears; awaits its answer."""
        ...

    async def notify(self, method: str, params: Mapping[str, object]) -> None:
        """Sends the engine the notification `method`, such as `$/credit`."""
        ...


class Order:
    """The order of what a Run's program sends without awaiting it (ADR 0081).

    `on` and a `Subscription`'s `close()` each take a hold when the program
    calls them, wait for the hold before theirs, write their message, then
    set theirs; a call of the Run waits for the last hold first. So each
    reaches the engine in the order the program called, and before any
    later call, as TypeScript's, written at once, do.
    """

    def __init__(self) -> None:
        """Starts with nothing held."""
        self._last: anyio.Event | None = None

    def hold(self) -> tuple[anyio.Event | None, anyio.Event]:
        """A new hold, set once its taker wrote its message, and the one before."""
        previous, self._last = self._last, anyio.Event()
        return previous, self._last

    async def settled(self) -> None:
        """Waits until every hold taken so far is set."""
        last = self._last
        if last is None:
            return
        if not last.is_set():
            await last.wait()
        if self._last is last:
            self._last = None


@dataclass(frozen=True)
class Listening(Generic[_Model]):
    """What a subscription listens to, and what it tells its value."""

    event: str
    """The event its listener gets, such as `page.console`; others are skipped."""
    payload: type[_Model]
    """The model of the event's payload."""
    listener: AsyncListener[_Model]
    """The program's listener."""
    failed: Callable[[Exception], None]
    """Hears, once, what the listener threw, after it was removed for it."""
    ended: Callable[[GoodfetchError], None]
    """Hears the error that ended the call unasked, such as `page_events_lost`."""


class Subscribed(Protocol):
    """A subscription, as its value sees it, whatever its payload's model."""

    @property
    def ended(self) -> bool:
        """Whether its call ended and nothing more is delivered."""
        ...

    def subscription(self) -> Subscription:
        """The `Subscription` its program closes."""
        ...

    async def stop(self) -> None:
        """Stops it as its value closed: a running listener is cancelled and awaited."""
        ...


def subscribe(
    connection: Streams,
    method: str,
    params: Mapping[str, object],
    listening: Listening[_Model],
    order: Order,
) -> Subscribed:
    """Sends the call `method` and delivers its events to `listening`'s listener.

    It returns at once, on the program's own event loop: the call is sent
    by a task of its own, after every hold `order` had.

    Raises:
        RuntimeError: No asyncio or Trio loop runs on this thread.
    """
    spawn = _spawner()
    subscribed = _Subscribed(connection, method, params, listening, order)
    spawn(subscribed.run)
    return subscribed


async def _wait(hold: anyio.Event | None) -> None:
    if hold is not None and not hold.is_set():
        await hold.wait()


class _Subscribed(Generic[_Model]):
    """One subscription's call, its deliveries and its close."""

    def __init__(
        self,
        connection: Streams,
        method: str,
        params: Mapping[str, object],
        listening: Listening[_Model],
        order: Order,
    ) -> None:
        self._connection = connection
        self._method = method
        self._params = params
        self._listening = listening
        self._order = order
        self._previous, self._sent = order.hold()
        self._closed = False
        self._ended = False
        self._closing: tuple[anyio.Event | None, anyio.Event] | None = None
        self._close_asked = anyio.Event()
        self._id: int | None = None
        self._events: MemoryObjectSendStream[object] | None = None
        self._hearing: tuple[anyio.CancelScope, anyio.Event] | None = None

    @property
    def ended(self) -> bool:
        """Whether its call ended and nothing more is delivered."""
        return self._ended

    def subscription(self) -> Subscription:
        """The `Subscription` its program closes."""
        return Subscription(self.close)

    async def stop(self) -> None:
        """Stops it as its value closed: a running listener is cancelled and awaited.

        The engine ends its call with the value, so nothing is sent. A
        listener that closed its own value is not awaited: it awaits that
        close.
        """
        self._closed = True
        hearing = self._hearing
        if hearing is None or _heard_by.get() is self:
            return
        scope, heard = hearing
        scope.cancel()
        await heard.wait()

    def close(self) -> None:
        """Removes the listener: its call is cancelled, after every earlier hold."""
        if self._closed:
            return
        self._closed = True
        if not self._ended:
            self._closing = self._order.hold()
        self._close_asked.set()

    async def run(self) -> None:
        """Sends the call once its turn came, and delivers its events until it ends."""
        try:
            await _wait(self._previous)
            if not self._closed:
                await self._listen()
        except Exception:
            # A spawned task has no caller to raise to.
            logging.getLogger(CLIENT_LOGGER).exception(
                "invariant: %s's subscription failed", self._method
            )
        finally:
            self._ended = True
            self._sent.set()
            await self._release_close()

    async def _release_close(self) -> None:
        """Sets the close's hold, after the one before it, with nothing left to send."""
        if self._closing is None:
            return
        previous, closed = self._closing
        with anyio.CancelScope(shield=True):
            await _wait(previous)
        closed.set()

    async def _listen(self) -> None:
        send, receive = anyio.create_memory_object_stream[object](math.inf)
        self._events = send
        async with anyio.create_task_group() as group:
            group.start_soon(self._deliver, receive)
            with send:
                await self._call()
            # What the call carried before it ended is still delivered.
        self._events = None

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
                except GoodfetchError as error:
                    if not self._closed:
                        self._listening.ended(error)
            group.cancel_scope.cancel()
        self._sent.set()
        await self._release_close()

    async def _grant_first(self, written: anyio.Event) -> None:
        await written.wait()
        await self._grant(CREDITS)
        self._sent.set()

    async def _cancel_when_closed(self, call: anyio.CancelScope) -> None:
        await self._close_asked.wait()
        if self._closing is not None:
            await _wait(self._closing[0])
        call.cancel()

    async def _grant(self, credits: int) -> None:
        if self._id is None:
            return
        # A grant cut short would leave half a line on the transport.
        with anyio.CancelScope(shield=True):
            await self._connection.notify(
                "$/credit", {"id": self._id, "credits": credits}
            )

    def _notified(self, method: str, params: object) -> None:
        """Queues a batch for delivery; it runs on the reading task, so never blocks."""
        events = self._events
        if method != EVENTS or self._closed or events is None:
            return
        try:
            events.send_nowait(params)
        except (anyio.ClosedResourceError, anyio.BrokenResourceError):
            # The call ended: nothing more is delivered.
            return

    async def _deliver(self, receive: MemoryObjectReceiveStream[object]) -> None:
        delivered = 0
        with receive:
            async for batch in receive:
                for event, data in _batch_of(batch):
                    if self._closed:
                        return
                    if event == self._listening.event and not await self._heard(data):
                        return
                    delivered += 1
                    if delivered == REGRANT:
                        delivered = 0
                        await self._grant(REGRANT)

    async def _heard(self, data: object) -> bool:
        """Gives one event's payload to the listener; false once it threw."""
        payload = result_of(self._listening.payload, object_members(data) or {})
        scope, heard = anyio.CancelScope(), anyio.Event()
        self._hearing = (scope, heard)
        by = _heard_by.set(self)
        try:
            with scope:
                returned = self._listening.listener(payload)
                if inspect.isawaitable(returned):
                    await returned
        except Exception as error:
            self.close()
            self._listening.failed(error)
            return False
        finally:
            _heard_by.reset(by)
            self._hearing = None
            heard.set()
        return True


def _batch_of(params: object) -> list[tuple[str, object]]:
    """The events of a `$/events` batch, each its name and data."""
    members = object_members(params)
    events = None if members is None else members.get("events")
    if not isinstance(events, list):
        return []
    batch: list[tuple[str, object]] = []
    for item in cast("list[object]", events):
        fields = object_members(item)
        name = None if fields is None else fields.get("event")
        if fields is not None and isinstance(name, str):
            batch.append((name, fields.get("data")))
    return batch


def _spawner() -> Callable[[_Work], None]:
    """What runs work beside the running loop's tasks, with the caller's context.

    The work raises nothing: a task no caller awaits has nowhere to raise to.

    Raises:
        RuntimeError: No asyncio or Trio loop runs on this thread.
    """
    context = contextvars.copy_context()
    try:
        loop = asyncio.get_running_loop()
    except RuntimeError:
        return _trio_spawner(context)

    def spawn(work: _Work) -> None:
        task = loop.create_task(work(), context=context)
        _running.add(task)
        task.add_done_callback(_running.discard)

    return spawn


def _trio_spawner(context: contextvars.Context) -> Callable[[_Work], None]:
    """`_spawner` under Trio, the other loop AnyIO runs on.

    Raises:
        RuntimeError: No Trio loop runs on this thread either.
    """
    try:
        # Imported only when no asyncio loop runs, so maybe under Trio.
        import trio

        trio.lowlevel.current_trio_token()
    except (ImportError, RuntimeError):
        message = (
            "on: no event loop runs on this thread; call it from asyncio or"
            " Trio code, or through the sync Collie"
        )
        raise RuntimeError(message) from None

    def spawn(work: _Work) -> None:
        trio.lowlevel.spawn_system_task(work, context=context)

    return spawn
