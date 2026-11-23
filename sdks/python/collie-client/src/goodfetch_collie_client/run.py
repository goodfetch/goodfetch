"""A Run, always opened by its program (ADRs 0078 and 0102).

Its members are the catalog's, generated (ADR 0099): `finish()` ends it and
answers its Run report. Hand-written here is what no catalog entry gives:
its ID and how it started, read without a call, and `close()`, which frees
the value and, while the Run is still running, stops it as an abort by its
Client, resumable by its Run ID. Any other call on a closed Run, or on a
value it opened, fails with `closed`. A listener of a value that threw is
raised once as `listener_failed`, by the next call on that value or the
`close()` that closes it, which first stops the value's listeners, or
logged at error once the value is released first (ADR 0081).
TypeScript's `run.ts` is its twin.
"""

from __future__ import annotations

import functools
import logging
import threading
import weakref
from dataclasses import dataclass
from typing import TYPE_CHECKING, Generic, Literal, Protocol, TypeVar

import anyio

from goodfetch_collie_client._followed import FollowedReading
from goodfetch_collie_client._framing import object_members
from goodfetch_collie_client._once import Once
from goodfetch_collie_client.caller import result_of
from goodfetch_collie_client.generated.errors import (
    CallCancelledError,
    ClosedError,
    ListenerFailedError,
)
from goodfetch_collie_client.generated.handles import AsyncRun as AsyncRunMembers
from goodfetch_collie_client.generated.handles import Run as RunMembers
from goodfetch_collie_client.generated.models import (
    ConfigExplainParams,
    Lifecycle,
    RunAbortParams,
    RunId,
    RunProgress,
    RunProgressParams,
    SettingsExplanation,
)
from goodfetch_collie_client.generated.operations import (
    DEPRECATED_OPERATIONS,
    EXPERIMENTAL_OPERATIONS,
)
from goodfetch_collie_client.goodfetch_error import runtime_message
from goodfetch_collie_client.handle import Subscription
from goodfetch_collie_client.host_logging import CLIENT_LOGGER
from goodfetch_collie_client.key_value_store import (
    KEYS,
    AsyncKeyValueStore,
    KeyValueStore,
    key_of,
    keys_params,
)
from goodfetch_collie_client.listeners import (
    Listening,
    Order,
    Streams,
    Subscribed,
    spawner,
    subscribe,
    subscribe_published,
)
from goodfetch_collie_client.markers import warn_entry
from goodfetch_collie_client.portal import LoopStopped
from goodfetch_collie_client.sequence import AsyncSequence, Reading, Sequence

if TYPE_CHECKING:
    from collections.abc import Awaitable, Callable, Iterable, Mapping
    from typing import Self

    from pydantic import BaseModel, JsonValue

    from goodfetch_collie_client._shared_subscription import SharedSubscription
    from goodfetch_collie_client.generated.models import RunStarted
    from goodfetch_collie_client.goodfetch_error import BaseGoodfetchError
    from goodfetch_collie_client.handle import (
        AsyncHandleHost,
        AsyncListener,
        HandleHost,
        Listener,
    )
    from goodfetch_collie_client.portal import PrivateLoop

__all__ = ["AsyncRun", "Run"]

_Id = TypeVar("_Id")
_ChildId = TypeVar("_ChildId")
_Model = TypeVar("_Model", bound="BaseModel")
_Result = TypeVar("_Result")
_Item = TypeVar("_Item")

_logger = logging.getLogger(CLIENT_LOGGER)


class RunHost(Streams, Protocol):
    """What a Run reaches its engine through: its Connection."""

    async def call(
        self, operation: str, params: BaseModel, *, timeout_setting: str | None = None
    ) -> object:
        """Sends one call and returns its result, as `AsyncHandleHost.call` does."""
        ...

    async def raw(self, operation: str, params: Mapping[str, object]) -> JsonValue:
        """Sends the raw call `operation`, its params and result as on the wire."""
        ...

    def reachable(self) -> bool:
        """Whether the Connection can still reach the engine."""
        ...

    def forget(self, run: AsyncRun) -> None:
        """Forgets a closed Run."""
        ...

    @property
    def published(self) -> SharedSubscription:
        """Its own Subscription, which the listeners of published events share."""
        ...


@dataclass(frozen=True)
class _Failure:
    """A listener that threw, which a call or a close raises once."""

    value: _ValueScope | None
    """The value it listened to, or none for the Run's own."""
    error: ListenerFailedError
    """What raises it."""


class _RunScope:
    """What a Run and the calls made through it share.

    Its ID, where it stands, and its listeners that failed.
    """

    def __init__(self, run_id: str) -> None:
        self.id = run_id
        self.state: Literal["running", "ended", "closed"] = "running"
        self.order = Order()
        """What `on` and a `Subscription`'s `close()` send, before any later call."""
        self._failures: list[_Failure] = []
        # A sync value's release runs on the program's thread, its listener
        # failures on the private loop's.
        self._lock = threading.Lock()
        self._listening: list[tuple[_ValueScope | None, Subscribed]] = []

    def listening(self, value: _ValueScope | None, subscribed: Subscribed) -> None:
        """Keeps `subscribed`, a listener of `value`, for that value's close to stop."""
        self._listening = [
            (held, kept) for held, kept in self._listening if not kept.ended
        ]
        self._listening.append((value, subscribed))

    async def stop_listeners(self, value: _ValueScope | None) -> None:
        """Stops the listeners of `value`, or under it, closed (ADR 0081).

        A listener running is cancelled and awaited, so none runs once the
        close returns; `value` none is the whole Run.
        """
        stopping = [kept for held, kept in self._listening if _within(held, value)]
        self._listening = [
            (held, kept) for held, kept in self._listening if not _within(held, value)
        ]
        for subscribed in stopping:
            await subscribed.stop()

    def failed(self, failure: _Failure) -> None:
        """Keeps `failure` for a call or close to raise, or logs it once released."""
        with self._lock:
            logged = failure.value is not None and failure.value.released
            if not logged:
                self._failures.append(failure)
        if logged:
            _log_failures([failure.error])

    def take_failures(
        self, which: Callable[[_Failure], bool]
    ) -> list[ListenerFailedError]:
        """Takes the failed listeners `which` picks."""
        with self._lock:
            released = self._take_released()
            taken = [failure for failure in self._failures if which(failure)]
            self._failures = [
                failure for failure in self._failures if not which(failure)
            ]
        _log_failures(released)
        return [failure.error for failure in taken]

    def take_first(
        self, which: Callable[[_Failure], bool]
    ) -> ListenerFailedError | None:
        """Takes the first failed listener `which` picks; others stay for later."""
        taken = None
        with self._lock:
            released = self._take_released()
            for index, failure in enumerate(self._failures):
                if which(failure):
                    del self._failures[index]
                    taken = failure.error
                    break
        _log_failures(released)
        return taken

    def _take_released(self) -> list[ListenerFailedError]:
        """Takes the failed listeners of released values; the lock is held."""
        released = [
            failure.error
            for failure in self._failures
            if failure.value is not None and failure.value.released
        ]
        if released:
            self._failures = [
                failure
                for failure in self._failures
                if failure.value is None or not failure.value.released
            ]
        return released

    def raise_failures_within(self, value: _ValueScope | None) -> None:
        """Raises the first failed listener of `value`, or under it, once it is closed.

        It logs the others, which no later call on a closed value could raise.

        Raises:
            ListenerFailedError: A listener of `value`, or of a value it
                opened or built, threw.
        """
        failures = self.take_failures(lambda failure: _within(failure.value, value))
        if failures:
            _log_failures(failures[1:])
            raise failures[0]

    def released(self, value: _ValueScope) -> None:
        """Logs the failed listeners of `value`, whose handle its program released.

        It runs as the handle's finalizer, maybe within a garbage collection
        on a thread that holds the lock, so it never waits for it: when the
        lock is held, the next call or close logs them.
        """
        value.released = True
        if not self._lock.acquire(blocking=False):
            return
        try:
            released = self._take_released()
        finally:
            self._lock.release()
        _log_failures(released)

    def ensure_open(self, operation: str) -> None:
        """Raises `closed` when the Run is closed.

        Raises:
            ClosedError: The Run is closed.
        """
        if self.state == "closed":
            raise self.closed_error(operation)

    def closed_error(self, operation: str) -> ClosedError:
        """The error a call on the closed Run fails with."""
        fix = "Start the Run again by its Run ID to reach it."
        message = runtime_message(
            f"the Run {self.id} is closed", operation=operation, fix=fix
        )
        return ClosedError(message, operation=operation, fix=fix)


class _ValueScope:
    """A value a program holds under a Run: a Session, a page, or a Locator."""

    def __init__(self, parent: _ValueScope | None) -> None:
        self.parent = parent
        """The value that opened or built it, when the Run did not."""
        self.closed = False
        """Whether its program closed it."""
        self.closing: anyio.Event | None = None
        """Set once its `close` in flight ends."""
        self.watched = False
        """Whether its handle is watched for release, as a listener on it asks."""
        self.released = False
        """Whether its program released its handle.

        A listener of it that throws then is logged at once.
        """

    def is_closed(self) -> bool:
        """Whether it, or a value that opened or built it, is closed."""
        held: _ValueScope | None = self
        while held is not None:
            if held.closed:
                return True
            held = held.parent
        return False


def _within(value: _ValueScope | None, within: _ValueScope | None) -> bool:
    """Whether `value` is `within`, or a value `within` opened or built."""
    if within is None:
        return True
    held = value
    while held is not None:
        if held is within:
            return True
        held = held.parent
    return False


def _log_failures(errors: list[ListenerFailedError]) -> None:
    """Logs at error the failed listeners nothing can raise any more."""
    for error in errors:
        _logger.error("%s", error, exc_info=error)


class _AsyncHost(Generic[_Id]):
    """What a handle of ID `id` in a Run reaches the engine through (ADR 0078).

    Each call fails with `closed`, with no round trip, once the Run, the
    handle's `value` or a value above it is closed. The Run's own calls,
    which have no `value`, also read their answer, since a Run report says
    when the Run ended.
    """

    def __init__(
        self,
        connection: RunHost,
        run: _RunScope,
        value: _ValueScope | None,
        handle_id: _Id,
    ) -> None:
        self._connection = connection
        self.run = run
        self._value = value
        self._id = handle_id

    @property
    def id(self) -> _Id:
        """The ID each of the handle's calls carries."""
        return self._id

    def _ensure_callable(self, operation: str) -> None:
        """Raises what stops a call on the value: a closed value, or a failed listener.

        The listener is one of the value or of a value that opened or built
        it, and is raised in place of the call, once.

        Raises:
            ClosedError: The Run, the value or a value above it is closed.
            ListenerFailedError: A listener of the value, or above it, threw.
        """
        self.run.ensure_open(operation)
        value = self._value
        if value is not None and value.is_closed():
            fix = "Open another from its Run: a closed value never reopens."
            message = runtime_message(
                "the value it is called on is closed, or the one that opened it is",
                operation=operation,
                fix=fix,
            )
            raise ClosedError(message, operation=operation, fix=fix)
        failed = self.run.take_first(
            lambda failure: failure.value is not None and _within(value, failure.value)
        )
        if failed is not None:
            raise failed

    async def call(
        self, operation: str, params: BaseModel, *, timeout_setting: str | None = None
    ) -> object:
        """Sends one call, unless the Run or value is closed, after what `on` sent."""
        return await self._sent(
            operation,
            functools.partial(
                self._connection.call,
                operation,
                params,
                timeout_setting=timeout_setting,
            ),
        )

    async def raw(self, operation: str, params: Mapping[str, object]) -> JsonValue:
        """Sends the raw call `operation`, unless the Run or value is closed."""
        return await self._sent(
            operation, functools.partial(self._connection.raw, operation, params)
        )

    async def _sent(
        self, operation: str, send: Callable[[], Awaitable[_Result]]
    ) -> _Result:
        """Sends `operation` through `send` after what `on` sent, unless refused.

        `_ensure_callable` refuses it: a closed value, or a failed listener.
        """
        self._ensure_callable(operation)
        await self.run.order.settled()
        result = await send()
        if self._value is None and self.run.state == "running" and _says_ended(result):
            self.run.state = "ended"
        return result

    def read(
        self,
        operation: str,
        params: Mapping[str, object],
        item_of: Callable[[str, object], _Item | None],
    ) -> AsyncSequence[_Item]:
        """Sends the Sequence `operation`, after what came before, as an iterator.

        Raises:
            ClosedError: The Run, the value or a value above it is closed.
            ListenerFailedError: A listener of the value, or above it, threw.
            RuntimeError: No asyncio or Trio loop runs on this thread.
        """
        self._ensure_callable(operation)
        spawn = spawner()
        reading = Reading(self._connection, operation, params, item_of, self.run.order)
        spawn(reading.run)
        return AsyncSequence(reading)

    def child(self, id: _ChildId) -> AsyncHandleHost[_ChildId]:
        """The host of a handle this one opens or builds, whose calls carry `id`."""
        return self.child_host(id)

    def child_host(self, id: _ChildId) -> _AsyncHost[_ChildId]:
        """`child`, as this class: what the sync twin's `child` wraps."""
        return _AsyncHost(self._connection, self.run, _ValueScope(self._value), id)

    async def close(self, operation: str, params: BaseModel) -> None:
        """Closes the value through `operation`, its own `close`; idempotent.

        Once it closed the value, or waited for the close in flight, it
        raises the first failed listener of the value or of one under it.

        Raises:
            RuntimeError: The handle is the Run's, which closes through its
                own `close()`.
        """
        value = self._value
        if value is None:
            message = (
                "invariant: a Run closes through its own close(), never its host's"
            )
            raise RuntimeError(message)
        waited = False
        while True:
            # What its Run or the value above it closed is closed with it,
            # and the engine holds it no more: nothing is left to send.
            if self.run.state == "closed" or value.is_closed():
                value.closed = True
                if waited:
                    self.run.raise_failures_within(value)
                return
            if value.closing is not None:
                await value.closing.wait()
                waited = True
                continue
            # A close the engine refuses, or that was never sent, closed
            # nothing: the next one sends again.
            closing = anyio.Event()
            value.closing = closing
            try:
                await self.run.order.settled()
                await self._connection.call(operation, params)
                value.closed = True
            finally:
                value.closing = None
                closing.set()
            await self.run.stop_listeners(value)
            self.run.raise_failures_within(value)
            return

    def listen(
        self,
        operation: str,
        params: BaseModel,
        event: str,
        payload: type[_Model],
        listener: AsyncListener[_Model],
    ) -> Subscription:
        """Sends `operation`, whose `event`s `listener` gets, after what came before.

        Raises:
            ClosedError: The Run, the value or a value above it is closed.
            ListenerFailedError: A listener of the value, or above it, threw.
        """
        listening = self._listening(operation, event, payload, listener)
        subscribed = subscribe(
            self._connection, operation, _wire(params), listening, self.run.order
        )
        self.run.listening(self._value, subscribed)
        return subscribed.subscription()

    def listen_published(
        self,
        operation: str,
        params: BaseModel,
        event: str,
        param: str,
        payload: type[_Model],
        listener: AsyncListener[_Model],
    ) -> Subscription:
        """Joins the Connection's Subscription, then sends `operation`, which arms it.

        `listener` gets each `event` whose payload names the handle by
        `param`, after what came before.

        Raises:
            ClosedError: The Run, the value or a value above it is closed.
            ListenerFailedError: A listener of the value, or above it, threw.
        """
        listening = self._listening(operation, event, payload, listener)
        wire = _wire(params)
        owner = wire.get(param)

        def mine(data: object) -> bool:
            members = object_members(data)
            return members is not None and members.get(param) == owner

        subscribed = subscribe_published(
            self._connection,
            self._connection.published,
            operation,
            wire,
            mine,
            listening,
            self.run.order,
        )
        self.run.listening(self._value, subscribed)
        return subscribed.subscription()

    def _listening(
        self,
        operation: str,
        event: str,
        payload: type[_Model],
        listener: AsyncListener[_Model],
    ) -> Listening[_Model]:
        """What a listener of the value's `event` tells it, once the value is watched.

        Raises:
            ClosedError: The Run, the value or a value above it is closed.
            ListenerFailedError: A listener of the value, or above it, threw.
        """
        self._ensure_callable(operation)
        run = self.run
        value = self._value
        if value is not None and not value.watched:
            value.watched = True
            # The callback holds the scopes, never the host it watches.
            weakref.finalize(self, run.released, value).atexit = False

        def failed(cause: Exception | BaseGoodfetchError) -> None:
            error = ListenerFailedError(
                f"{operation}: the listener of `{event}` threw, so it was removed",
                fix="Fix the listener, then listen again with `on`.",
                details={"event": event},
            )
            error.__cause__ = cause
            run.failed(_Failure(value, error))

        def ended(error: BaseGoodfetchError) -> None:
            # A call its Connection's close cancelled, or that stopped as its
            # value closed, ended with everything else; any other error,
            # such as `page_events_lost`, ended this listener alone, and
            # nothing else would say so.
            if isinstance(error, CallCancelledError | ClosedError):
                return
            _logger.error(
                "%s: the listener of `%s` stopped", operation, event, exc_info=error
            )

        return Listening(event, payload, listener, failed, ended)


def _wire(params: BaseModel) -> dict[str, object]:
    """`params` as the call sends them."""
    return params.model_dump(
        mode="json", by_alias=True, exclude_unset=True, warnings=False
    )


class _SyncHost(Generic[_Id]):
    """What a sync handle reaches its engine through: its async twin's host."""

    def __init__(self, loop: PrivateLoop, host: _AsyncHost[_Id]) -> None:
        self._loop = loop
        self._host = host

    @property
    def id(self) -> _Id:
        """The ID each of the handle's calls carries."""
        return self._host.id

    def call(
        self, operation: str, params: BaseModel, *, timeout_setting: str | None = None
    ) -> object:
        """Sends one call on the private loop and waits for its result."""
        try:
            return self._loop.call(
                functools.partial(
                    self._host.call,
                    operation,
                    params,
                    timeout_setting=timeout_setting,
                )
            )
        except LoopStopped:
            # Its Connection closed the Run before stopping the loop.
            raise self._host.run.closed_error(operation) from None

    def raw(self, operation: str, params: Mapping[str, object]) -> JsonValue:
        """Sends the raw call on the private loop and waits for its result."""
        try:
            return self._loop.call(functools.partial(self._host.raw, operation, params))
        except LoopStopped:
            # Its Connection closed the Run before stopping the loop.
            raise self._host.run.closed_error(operation) from None

    def read(
        self,
        operation: str,
        params: Mapping[str, object],
        item_of: Callable[[str, object], _Item | None],
    ) -> Sequence[_Item]:
        """Sends the Sequence `operation` on the private loop, as an iterator.

        Any thread may read the iterator.
        """

        async def reading() -> AsyncSequence[_Item]:
            return self._host.read(operation, params, item_of)

        try:
            sequence = self._loop.call(reading)
        except LoopStopped:
            # Its Connection closed the Run before stopping the loop.
            raise self._host.run.closed_error(operation) from None
        return Sequence(
            self._loop, sequence, lambda: self._host.run.closed_error(operation)
        )

    def child(self, id: _ChildId) -> HandleHost[_ChildId]:
        """The host of a handle this one opens or builds, whose calls carry `id`."""
        return _SyncHost(self._loop, self._host.child_host(id))

    def close(self, operation: str, params: BaseModel) -> None:
        """Closes the value through `operation`; idempotent."""
        try:
            self._loop.call(functools.partial(self._host.close, operation, params))
        except LoopStopped:
            # Its Connection closed the Run, and the value with it.
            return

    def listen(
        self,
        operation: str,
        params: BaseModel,
        event: str,
        payload: type[_Model],
        listener: Listener[_Model],
    ) -> Subscription:
        """Sends `operation` on the private loop; `listener` runs on a worker thread.

        Never on the loop's thread, which reads every call's answer
        (ADR 0081).
        """
        return self._on_loop(
            operation,
            listener,
            lambda in_worker: self._host.listen(
                operation, params, event, payload, in_worker
            ),
        )

    def listen_published(
        self,
        operation: str,
        params: BaseModel,
        event: str,
        param: str,
        payload: type[_Model],
        listener: Listener[_Model],
    ) -> Subscription:
        """Registers the listener on the private loop; it runs on a worker thread."""
        return self._on_loop(
            operation,
            listener,
            lambda in_worker: self._host.listen_published(
                operation, params, event, param, payload, in_worker
            ),
        )

    def _on_loop(
        self,
        operation: str,
        listener: Listener[_Model],
        listen: Callable[[AsyncListener[_Model]], Subscription],
    ) -> Subscription:
        """Calls `listen` on the private loop with `listener` run on a worker thread."""

        async def in_worker(heard: _Model) -> None:
            await anyio.to_thread.run_sync(listener, heard)

        async def listening() -> Subscription:
            return listen(in_worker)

        try:
            subscription = self._loop.call(listening)
        except LoopStopped:
            # Its Connection closed the Run before stopping the loop.
            raise self._host.run.closed_error(operation) from None
        loop = self._loop

        async def closing() -> None:
            subscription.close()

        def close() -> None:
            try:
                loop.call(closing)
            except LoopStopped:
                # Its Connection closed the Run, and the listener with it.
                return

        return Subscription(close)


_held: weakref.WeakKeyDictionary[AsyncRun, tuple[_AsyncHost[RunId], RunStarted]] = (
    weakref.WeakKeyDictionary()
)
"""Each async Run's host, which its sync twin's host wraps, and its start
answer, which its sync twin's reads answer from."""


class AsyncRun(AsyncRunMembers):
    """A Run of the async API."""

    def __init__(
        self, connection: RunHost, started: RunStarted, *, detached: bool = False
    ) -> None:
        """Takes its Connection and the engine's answer to its start.

        A `detached` start's Run outlives its program (ADR 0102): closing its
        value then stops nothing.
        """
        scope = _RunScope(started.run_id.root)
        if started.lifecycle != Lifecycle.running:
            # A start that found its Run ended: closing it stops nothing.
            scope.state = "ended"
        host = _AsyncHost(connection, scope, None, started.run_id)
        super().__init__(host, started)
        _held[self] = (host, started)
        self._connection = connection
        self._scope = scope
        self._detached = detached
        self._closing = Once()
        self.key_value_store = AsyncKeyValueStore(
            lambda prefix: host.read(KEYS, keys_params(self.id, prefix), key_of)
        )
        """Its Key-value store (ADR 0103)."""

    @property
    def id(self) -> str:
        """Its Run ID."""
        return self._scope.id

    async def explain_settings(
        self, keys: Iterable[str] | None = None
    ) -> SettingsExplanation:
        """What the Run runs with (ADR 0083), as `config explain` shows it.

        Shows each setting's effective value, the layer that set it and the layer
        that locked it, from the built-in defaults, the Operator policy, user
        configuration, the project configuration given and the run layer: the
        Connection's own settings with the run values given over them, or a live
        Run's as it opened when `run_id` names one; and lists the keys only the
        other Product honours as that Product's.

        It names this Run's ID, so it explains each setting under `keys`, each
        a key or a prefix of keys such as `timeout`, every one when left out,
        with the value the Run opened with and the origin that set it: its own
        `settings` over its Connection's. A Run that is no longer live fails
        with `run_not_found`, and a closed Run's value with `closed`.
        """
        params = explain_params(keys, run_id=self.id)
        return result_of(
            SettingsExplanation, await self._host.call("config.explain", params)
        )

    def progress(self) -> AsyncSequence[RunProgress]:
        """The Run's progress (`run.progress`, ADR 0102), read with `async for`.

        Reads a Run's progress: what it does and why it waits. Without `after`, it
        answers at once; given `after`, the position of the value the reader last
        read, it waits for a change and answers the latest value, so values in
        between are coalesced. The value whose `last` is true ends the sequence: the
        Run ended, and its `lifecycle` and `stop_reason` say how, or no engine works
        it (`writer: none`) until a start with its Run ID resumes it. A Run another
        engine works is read through that engine, the owner the Run index names, and
        one no engine works from its committed state; a read never takes the Run's
        writer lock. A Run ID the Run index does not list fails with
        `run_not_found`.

        Experimental: outside semver until it is promoted (ADR 0030).

        Its first read, without `after`, is sent when it is called; each next
        one is sent once the program reads the value before it, with that
        value's `position` as `after`, so values it did not read in time
        coalesce. Leaving an `async with` block, `aclose()`, `close()`
        or releasing it cancel its read at once.

        Raises:
            ClosedError: The Run is closed; nothing is sent.
            RuntimeError: No asyncio or Trio loop runs on this thread.
        """
        warn_entry("run.progress", EXPERIMENTAL_OPERATIONS, DEPRECATED_OPERATIONS)
        self._scope.ensure_open("run.progress")
        host = self._host

        async def read(after: int | None) -> RunProgress:
            params = RunProgressParams(run_id=host.id)
            if after is not None:
                params.after = after
            return result_of(RunProgress, await host.call("run.progress", params))

        reading = FollowedReading(
            read, position=lambda value: value.position, last=lambda value: value.last
        )
        spawner()(reading.run)
        return AsyncSequence(reading)

    async def close(self) -> None:
        """Closes the Run; idempotent.

        A Run still running is stopped with `run.abort`, resumable by its Run
        ID, and the Run report it answers is not returned; one no longer
        running, or started `detached`, which outlives its program, is
        closed with nothing sent.

        Stops a Run at its next safe point, `stopped` and `incomplete`: no attempt in
        flight commits, and its work items return to pending. Answers its Run
        report; an ended Run's report is answered as it is. A Run whose engine died
        is stopped by this engine, which takes its writer role at the next run
        epoch; a Run another engine writes is stopped by that engine, the owner the
        Run index names.

        Experimental: outside semver until it is promoted (ADR 0030).

        Raises:
            ListenerFailedError: A listener of the Run, or of a value it
                opened, threw; it is raised once the Run is closed.
        """
        warn_entry("run.abort", EXPERIMENTAL_OPERATIONS, DEPRECATED_OPERATIONS)
        await self._closing.run(self._close)
        self._scope.raise_failures_within(None)

    async def _close(self) -> None:
        stops = self._scope.state == "running" and not self._detached
        self._scope.state = "closed"
        try:
            await self._scope.order.settled()
            if stops and self._connection.reachable():
                await self._connection.call(
                    "run.abort", RunAbortParams(run_id=RunId(self.id))
                )
        finally:
            self._connection.forget(self)
            await self._scope.stop_listeners(None)

    async def __aenter__(self) -> Self:
        """Holds the Run for an `async with` block, which closes it."""
        return self

    async def __aexit__(self, *_: object) -> None:
        """Closes the Run."""
        await self.close()


def explain_params(
    keys: Iterable[str] | None, *, run_id: str | None = None
) -> ConfigExplainParams:
    """`config.explain`'s params, for the Run `run_id` or else the Connection.

    `keys` and `run_id` each stay off the wire when left out.
    """
    params = ConfigExplainParams()
    if run_id is not None:
        params.run_id = run_id
    if keys is not None:
        params.keys = list(keys)
    return params


def _says_ended(result: object) -> bool:
    """Whether `result` is an answer that says its Run is no longer running."""
    members = object_members(result)
    if members is None:
        return False
    lifecycle = members.get("lifecycle")
    return lifecycle is not None and lifecycle != "running"


class Run(RunMembers):
    """A Run of the sync API, which any number of threads may share."""

    def __init__(self, loop: PrivateLoop, run: AsyncRun) -> None:
        """Takes its Connection's private loop and the Run it drives there."""
        async_host, started = _held[run]
        host = _SyncHost(loop, async_host)
        super().__init__(host, started)
        self._loop = loop
        self._run = run
        self.key_value_store = KeyValueStore(
            lambda prefix: host.read(KEYS, keys_params(run.id, prefix), key_of)
        )
        """Its Key-value store (ADR 0103)."""

    @property
    def id(self) -> str:
        """Its Run ID."""
        return self._run.id

    def explain_settings(
        self, keys: Iterable[str] | None = None
    ) -> SettingsExplanation:
        """What the Run runs with, as `AsyncRun.explain_settings` says.

        Shows each setting's effective value, the layer that set it and the layer
        that locked it, from the built-in defaults, the Operator policy, user
        configuration, the project configuration given and the run layer: the
        Connection's own settings with the run values given over them, or a live
        Run's as it opened when `run_id` names one; and lists the keys only the
        other Product honours as that Product's.
        """
        params = explain_params(keys, run_id=self.id)
        return result_of(SettingsExplanation, self._host.call("config.explain", params))

    def progress(self) -> Sequence[RunProgress]:
        """The Run's progress, read with `for`, as `AsyncRun.progress` says.

        Reads a Run's progress: what it does and why it waits. Without `after`, it
        answers at once; given `after`, the position of the value the reader last
        read, it waits for a change and answers the latest value, so values in
        between are coalesced. The value whose `last` is true ends the sequence: the
        Run ended, and its `lifecycle` and `stop_reason` say how, or no engine works
        it (`writer: none`) until a start with its Run ID resumes it. A Run another
        engine works is read through that engine, the owner the Run index names, and
        one no engine works from its committed state; a read never takes the Run's
        writer lock. A Run ID the Run index does not list fails with
        `run_not_found`.

        Experimental: outside semver until it is promoted (ADR 0030).

        Any thread may read it. Leaving a `with` block, `close()` or
        releasing it cancel its read at once.

        Raises:
            ClosedError: The Run is closed; nothing is sent.
        """
        run = self._run
        scope = _held[run][0].run

        async def reading() -> AsyncSequence[RunProgress]:
            return run.progress()

        try:
            sequence = self._loop.call(reading)
        except LoopStopped:
            # Its Connection closed the Run before stopping the loop.
            raise scope.closed_error("run.progress") from None
        return Sequence(
            self._loop, sequence, lambda: scope.closed_error("run.progress")
        )

    def close(self) -> None:
        """Closes the Run; idempotent.

        A Run still running is stopped with `run.abort`, resumable by its Run
        ID, unless it was started `detached`, as `AsyncRun.close` says.

        Stops a Run at its next safe point, `stopped` and `incomplete`: no attempt in
        flight commits, and its work items return to pending. Answers its Run
        report; an ended Run's report is answered as it is. A Run whose engine died
        is stopped by this engine, which takes its writer role at the next run
        epoch; a Run another engine writes is stopped by that engine, the owner the
        Run index names.

        Experimental: outside semver until it is promoted (ADR 0030).
        """
        warn_entry("run.abort", EXPERIMENTAL_OPERATIONS, DEPRECATED_OPERATIONS)
        try:
            self._loop.call(self._run.close)
        except LoopStopped:
            # Its Connection closed it before stopping the loop.
            return

    def __enter__(self) -> Self:
        """Holds the Run for a `with` block, which closes it."""
        return self

    def __exit__(self, *_: object) -> None:
        """Closes the Run."""
        self.close()
