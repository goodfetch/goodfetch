"""The JSON-RPC channel a Connection speaks over its transport (ADRs 0024 and 0038).

Each call writes one request and waits for the reply with its id. The
channel runs no task of its own, so it never outlives the caller's task
group: whichever call is waiting while nobody reads becomes the reader,
reads until its own reply has come, resolving the others' replies on the
way, then hands reading over to a call still waiting. A call cancelled
while it waits sends `$/cancel` with its id. When the engine's side ends,
the owner says which error every call still waiting fails with, and every
later call fails with it too. A stream is a call whose notifications, such
as `$/chunk`, name its id: the reader hands each to the stream's listener,
and the caller grants credits with `notify`; a notification that names no
call, such as the engine's `$/log`, goes to the owner.

At most the engine's cap of calls, which `hello`'s result advertises, are
in flight; the rest wait in a queue, and the reader sends them in the order
they were made as answers free their places (ADR 0038). A call cancelled
once sent keeps its place until the engine answers it. A queued call takes
its turn at reading, since the calls holding the places may all have been
cancelled, and fails alone, never sent, when its own timeout passes or its
task is cancelled there. What waits is bounded: past the Connection's
bound on calls waiting, sent or queued, or on bytes waiting to be written,
a new call is refused at once with `connection_overloaded`. TypeScript's
`rpc.ts` is its twin; served requests come with the ticket that uses them.
"""

from __future__ import annotations

import math
from dataclasses import dataclass, field
from typing import TYPE_CHECKING, Literal, Protocol, TypeAlias, TypeVar, cast

import anyio

from goodfetch_collie_client._framing import (
    Failure,
    Invalid,
    LineReader,
    Notification,
    Request,
    Result,
    encode_message,
    object_members,
)
from goodfetch_collie_client.caller import result_of
from goodfetch_collie_client.generated.errors import (
    ERROR_GROUP_CLASSES,
    ERROR_GROUP_OF_PROTOCOL_FAILURE,
    ERROR_GROUP_OF_WIRE_CLASS,
    ConnectionOverloadedError,
    DeadlineExceededError,
    SettingInvalidError,
)
from goodfetch_collie_client.generated.models import (
    ModelUsage,
    RunOrigin,
    SettingLimit,
    SettingsLayer,
)
from goodfetch_collie_client.goodfetch_error import GoodfetchError, in_call

if TYPE_CHECKING:
    from collections.abc import Callable, Mapping

    from anyio.abc import ByteStream

    from goodfetch_collie_client.generated.errors import ErrorGroup
    from goodfetch_collie_client.goodfetch_error import (
        BaseGoodfetchError,
        ErrorClass,
        ErrorFields,
    )

_Field = TypeVar("_Field")

METHOD_NOT_FOUND = -32601
"""JSON-RPC's code for a method the receiver does not serve."""


@dataclass(frozen=True)
class _Bound:
    """One of the Client's own bounds, a setting of `connect()`'s own `settings`."""

    key: str
    """Its key, as the wire and the engine's details write it."""
    fallback: int
    """Its default."""

    @property
    def segment(self) -> str:
        """Its name under `connection`."""
        return self.key.removeprefix("connection.")


_PENDING_CALLS = _Bound("connection.max_pending_calls", 10_000)
"""The bound on calls waiting for their answer, sent or queued."""

_QUEUED_WRITE_BYTES = _Bound("connection.max_queued_write_bytes", 64 * 1024 * 1024)
"""The bound on bytes waiting to be written to the engine."""


@dataclass(frozen=True)
class ChannelBounds:
    """What a Connection lets wait before it refuses a call, `connection_overloaded`."""

    max_pending_calls: int = _PENDING_CALLS.fallback
    """How many calls may wait for their answer at once, sent or queued."""
    max_queued_write_bytes: int = _QUEUED_WRITE_BYTES.fallback
    """How many bytes may wait to be written to the engine.

    One message larger than this is still taken while nothing waits.
    """


def bounds_of(settings: Mapping[str, object] | None) -> ChannelBounds:
    """The bounds `connect()`'s own `settings` set, each left out its default.

    Checked before anything is reached. A value may be one a type checker
    would refuse, since a program may pass it. TypeScript's `boundsOf` is
    its twin.

    Raises:
        SettingInvalidError: A bound is no whole number of 1 or more, which
            would refuse every call, or none.
    """
    connection = object_members((settings or {}).get("connection")) or {}
    return ChannelBounds(
        max_pending_calls=_whole_bound(_PENDING_CALLS, connection),
        max_queued_write_bytes=_whole_bound(_QUEUED_WRITE_BYTES, connection),
    )


def _whole_bound(bound: _Bound, connection: Mapping[str, object]) -> int:
    if bound.segment not in connection:
        return bound.fallback
    return whole_bound(
        connection[bound.segment],
        key=bound.key,
        fix=(
            f"Set `{bound.key}` in connect()'s settings to a whole number of 1 or"
            f" more, or leave it out for its default of {bound.fallback}."
        ),
        layer="run",
    )


def whole_bound(given: object, *, key: str, fix: str, layer: str | None = None) -> int:
    """`given`, the Client's own bound `key`, when a whole number of 1 or more.

    It takes any value, since a program may pass one its type checker would
    refuse. TypeScript's `wholeBound` is its twin.

    Raises:
        SettingInvalidError: `given` is no whole number of 1 or more, with
            `fix`, and `layer` in its details when the bound is a setting.
    """
    if isinstance(given, int) and not isinstance(given, bool) and given >= 1:
        return given
    reason = f"{given!r} is no whole number of 1 or more"
    details: dict[str, object] = {"key": key, "reason": reason}
    if layer is not None:
        details["layer"] = layer
    raise SettingInvalidError(
        f"`{key}` is not valid: {reason}", fix=fix, details=details
    )


@dataclass(frozen=True)
class CallDeadline:
    """A call's own timeout, which fails it while it waits in the queue (issue 3188)."""

    key: str
    """What sets it: the setting a typed call's `timeout` sets, or a raw call's."""
    value: int
    """How long, in milliseconds."""


def phase_exceeded(
    message: str,
    *,
    fix: str,
    limit: SettingLimit,
    started_at: float,
    waited_for: Literal["hello", "close", "calls_in_flight"],
) -> DeadlineExceededError:
    """The `deadline_exceeded` of a phase `limit` bounds (ADR 0080).

    Its wait for `waited_for` started at `started_at`, on anyio's clock.
    """
    return DeadlineExceededError(
        message,
        code="deadline_exceeded",
        group="deadline_exceeded",
        retryable=True,
        fix=fix,
        limit=limit,
        details={
            "elapsed": round((anyio.current_time() - started_at) * 1000),
            "waited_for": waited_for,
        },
    )


@dataclass(frozen=True)
class Ended:
    """The engine's side reached its end."""


@dataclass(frozen=True)
class ReadFailed:
    """Reading the engine's side failed with `cause`."""

    cause: Exception


@dataclass(frozen=True)
class WriteFailed:
    """Writing to the engine's side failed with `cause`."""

    cause: Exception


@dataclass(frozen=True)
class InvalidLine:
    """The engine wrote `line`, which is not one JSON-RPC message."""

    line: str


TransportEnd: TypeAlias = Ended | ReadFailed | WriteFailed | InvalidLine
"""How the engine's side of a channel ended."""


class ChannelOwner(Protocol):
    """What the channel asks of the Connection that owns it."""

    async def ended(self, end: TransportEnd) -> GoodfetchError:
        """The error each call still waiting, and every later one, fails with."""
        ...

    def notified(self, method: str, params: object) -> None:
        """Hears a notification that names no call, such as the engine's `$/log`.

        It runs on the reading task, so it must not block.
        """
        ...


@dataclass(frozen=True)
class _Answered:
    result: object


@dataclass(frozen=True)
class _Raised:
    error: BaseGoodfetchError
    answered: bool = False
    """Whether the engine answered with it, rather than the runtime raising it."""


@dataclass(eq=False)
class _Waiter:
    """A call waiting for its reply, or a wait for the channel's end."""

    outcome: _Answered | _Raised | None = None
    wakeup: anyio.Event | None = field(default=None)

    def settle(self, outcome: _Answered | _Raised) -> None:
        self.outcome = outcome
        self.wake()

    def wake(self) -> None:
        if self.wakeup is not None:
            self.wakeup.set()


class Channel:
    """One Connection's JSON-RPC calls over `transport`."""

    def __init__(
        self,
        transport: ByteStream,
        owner: ChannelOwner,
        error_classes: Mapping[str, ErrorClass],
        bounds: ChannelBounds | None = None,
    ) -> None:
        """Takes the transport, the owner told of its end, the code classes and bounds.

        `bounds`, already checked by `bounds_of`, are the defaults' when left out.
        """
        self._transport = transport
        self._owner = owner
        self._error_classes = error_classes
        self._bounds = bounds or ChannelBounds()
        self._lines = LineReader()
        self._next_id = 0
        self._waiting: dict[int, _Waiter] = {}
        self._in_flight: set[int] = set()
        """The calls sent and not yet answered, a cancelled one included."""
        self._queued: dict[int, bytes] = {}
        """Each call held back for a place, its encoded request, in the order made."""
        self._cap: float = math.inf
        """How many calls may be in flight at once: as `hello` names it, if it does."""
        self._unwritten_bytes = 0
        """The bytes queued or waiting to be written, a message being written too."""
        self._listeners: dict[int, Callable[[str, object], None]] = {}
        self._drainers: list[_Waiter] = []
        self._reading = False
        self._reader_scope: anyio.CancelScope | None = None
        self._write_lock = anyio.Lock()
        self._ending = False
        self._failure: GoodfetchError | None = None
        self._ended = anyio.Event()

    async def call(
        self, method: str, params: object, *, deadline: CallDeadline | None = None
    ) -> object:
        """Sends `method` with `params` and returns the engine's result.

        `deadline`, the call's own timeout, fails it while it waits in the
        queue for a place among the engine's calls in flight.

        Raises:
            GoodfetchError: The engine answered with an error, or the
                channel ended or was cancelled before the reply, or the call
                was refused with `connection_overloaded`, or its `deadline`
                passed while it was queued.
            BaseGoodfetchError: Foxhound's `AttemptEndedError`: the engine
                answered `attempt_ended`, whose class is outside `Exception`
                (ADR 0093).
        """
        return await self.stream(method, params, None, deadline=deadline)

    async def stream(
        self,
        method: str,
        params: object,
        notified: Callable[[str, object], None] | None,
        *,
        sent: Callable[[int], None] | None = None,
        deadline: CallDeadline | None = None,
    ) -> object:
        """Sends `method` as `call` does; `notified` hears each notification naming it.

        `sent` is told the call's id once it is sent, ahead of anything
        written later, so the caller can grant credits with `notify`, which
        the engine drops for a call it has not received; a call that ended
        first, as one failed while queued, is never told. `notified` runs on
        the reading task and must not block.

        Raises:
            GoodfetchError: As `call` raises.
            BaseGoodfetchError: Foxhound's `AttemptEndedError`: the engine
                answered `attempt_ended`, whose class is outside `Exception`
                (ADR 0093).
        """
        if self._failure is not None:
            raise in_call(self._failure, method)
        message_id = self._next_id
        # Encoded before the call waits, so params that cannot be encoded
        # fail this call alone, and its length is known to the bounds.
        request = encode_message({"id": message_id, "method": method, "params": params})
        overloaded = self._overloaded_by(len(request))
        if overloaded is not None:
            raise in_call(overloaded, method)
        self._next_id += 1
        waiter = _Waiter()
        self._waiting[message_id] = waiter
        if notified is not None:
            self._listeners[message_id] = notified
        try:
            if self._queued or len(self._in_flight) >= self._cap:
                await self._wait_queued(message_id, request, method, deadline)
            else:
                self._in_flight.add(message_id)
                await self._write_bytes(request)
            if sent is not None and waiter.outcome is None:
                sent(message_id)
            outcome = await self._wait_for(waiter)
        except BaseException:
            if waiter.outcome is None:
                del self._waiting[message_id]
                # A queued call leaves the queue unsent; a sent one is cancelled.
                if not self._unqueue(message_id) and message_id in self._in_flight:
                    with anyio.CancelScope(shield=True):
                        await self._notify_cancel(message_id)
                self._wake_all()
            raise
        finally:
            self._listeners.pop(message_id, None)
        if isinstance(outcome, _Raised):
            raise outcome.error if outcome.answered else in_call(outcome.error, method)
        return outcome.result

    def cap_calls_in_flight(self, cap: int) -> None:
        """Keeps at most `cap` calls in flight from now on, as `hello`'s result says.

        Set once `hello` is answered, before any call can be queued.
        """
        self._cap = cap

    async def _wait_queued(
        self,
        message_id: int,
        request: bytes,
        method: str,
        deadline: CallDeadline | None,
    ) -> None:
        """Holds the call back until the reader sends it, as a place frees.

        Raises:
            DeadlineExceededError: `deadline` passed first; the call, never
                sent, is still queued, for the caller to drop.
        """
        self._queued[message_id] = request
        self._unwritten_bytes += len(request)
        waiter = self._waiting[message_id]

        def left_the_queue() -> bool:
            return waiter.outcome is not None or message_id not in self._queued

        if deadline is None:
            await self._wait_until(waiter, left_the_queue)
            return
        started_at = anyio.current_time()
        with anyio.move_on_after(deadline.value / 1000):
            await self._wait_until(waiter, left_the_queue)
        if left_the_queue():
            return
        raise in_call(
            phase_exceeded(
                f"the call waited {deadline.value} ms, its `{deadline.key}`, for a"
                " place among the engine's calls in flight, so it was not sent",
                fix=(
                    "Make fewer calls at once on this Connection, or give the call"
                    " a longer `timeout`."
                ),
                limit=SettingLimit(
                    key=deadline.key,
                    value=deadline.value,
                    layer=SettingsLayer.run,
                    origin=RunOrigin.argument,
                ),
                started_at=started_at,
                waited_for="calls_in_flight",
            ),
            method,
        )

    def _unqueue(self, message_id: int) -> bool:
        """Takes the call `message_id` out of the queue: whether it was queued."""
        request = self._queued.pop(message_id, None)
        if request is None:
            return False
        self._unwritten_bytes -= len(request)
        return True

    def _overloaded_by(self, length: int) -> ConnectionOverloadedError | None:
        """The error a new call of `length` bytes is refused with, past a bound.

        Notifications are never refused: each belongs to a call already
        counted, and a `$/cancel` is what frees one.
        """
        max_pending_calls = self._bounds.max_pending_calls
        if len(self._waiting) >= max_pending_calls:
            return ConnectionOverloadedError(
                f"the Connection already holds {len(self._waiting)} calls waiting for"
                " their answer, sent or queued, its bound, so this call was not sent",
                fix=(
                    "Wait for earlier calls to end, or raise"
                    f" `{_PENDING_CALLS.key}` in connect()'s settings."
                ),
                details={"bound": "pending_calls", "value": max_pending_calls},
            )
        unwritten = self._unwritten_bytes
        max_queued_write_bytes = self._bounds.max_queued_write_bytes
        if unwritten > 0 and unwritten + length > max_queued_write_bytes:
            return ConnectionOverloadedError(
                f"the Connection already holds {unwritten} bytes waiting to be written"
                f" to the engine, and this call's {length} would pass its bound of"
                f" {max_queued_write_bytes}, so it was not sent",
                fix=(
                    "Wait for the engine to read earlier calls, or raise"
                    f" `{_QUEUED_WRITE_BYTES.key}` in connect()'s settings."
                ),
                details={
                    "bound": "queued_write_bytes",
                    "value": max_queued_write_bytes,
                },
            )
        return None

    async def notify(self, method: str, params: object) -> None:
        """Sends the notification `method`, such as `$/credit`, unless it ended."""
        if self._ending:
            return
        await self._write({"method": method, "params": params})

    def cancel_all(self, error: GoodfetchError) -> list[int]:
        """Fails every call still waiting with `error`, and stops the reader.

        Returns:
            The ids of the sent calls cancelled, whose `$/cancel` the caller
            sends with `notify_cancelled`; a queued one is never sent.
        """
        cancelled = [
            message_id for message_id in self._waiting if not self._unqueue(message_id)
        ]
        for waiter in self._waiting.values():
            waiter.settle(_Raised(error))
        self._waiting.clear()
        if self._reader_scope is not None:
            self._reader_scope.cancel()
        return cancelled

    async def notify_cancelled(self, ids: list[int]) -> None:
        """Sends `$/cancel` for each call of `ids`, while the channel lasts."""
        with anyio.CancelScope(shield=True):
            for message_id in ids:
                await self._notify_cancel(message_id)

    async def close(self) -> None:
        """Ends the Client's side: an Attached engine stops at the end of its input."""
        try:
            await self._transport.send_eof()
        except Exception:
            # The engine's side already ended, which the reader reports.
            return

    async def abandon(self) -> None:
        """Closes both sides at once, as giving up does."""
        try:
            await self._transport.aclose()
        except Exception:
            # Nothing it held matters any more.
            return

    async def wait_ended(self) -> GoodfetchError:
        """Reads until the engine's side ends; the error later calls fail with."""
        if self._failure is not None:
            # It ended already, as on a line that is not JSON-RPC.
            return self._failure
        drainer = _Waiter()
        self._drainers.append(drainer)
        try:
            outcome = await self._wait_for(drainer)
        finally:
            self._drainers.remove(drainer)
        if not isinstance(outcome, _Raised) or not isinstance(
            outcome.error, GoodfetchError
        ):
            message = "invariant: a channel ends with the error later calls fail with"
            raise TypeError(message)
        return outcome.error

    async def _wait_for(self, waiter: _Waiter) -> _Answered | _Raised:
        await self._wait_until(waiter, lambda: waiter.outcome is not None)
        if waiter.outcome is None:
            message = "invariant: a waiter waited for is settled"
            raise TypeError(message)
        return waiter.outcome

    async def _wait_until(self, waiter: _Waiter, done: Callable[[], bool]) -> None:
        """Waits, woken through `waiter`, until `done`, reading while none else does."""
        while not done():
            if self._reading:
                waiter.wakeup = anyio.Event()
                await waiter.wakeup.wait()
                continue
            self._reading = True
            try:
                with anyio.CancelScope() as scope:
                    self._reader_scope = scope
                    while not done():
                        await self._read_one()
            finally:
                self._reading = False
                self._reader_scope = None
                self._wake_all()

    def _wake_all(self) -> None:
        # Each woken waiter checks its outcome, and the first unsettled one reads.
        for waiter in self._waiting.values():
            waiter.wake()
        for drainer in self._drainers:
            drainer.wake()

    async def _read_one(self) -> None:
        if self._ending:
            await self._ended.wait()
            return
        try:
            chunk = await self._transport.receive()
        except anyio.EndOfStream:
            await self._finish(Ended())
            return
        except Exception as cause:
            await self._finish(ReadFailed(cause))
            return
        unserved: list[Request] = []
        for line in self._lines.feed(chunk):
            if isinstance(line, Invalid):
                await self._finish(InvalidLine(line.line))
                return
            if isinstance(line, Result | Failure):
                self._settle(line)
            elif isinstance(line, Request):
                unserved.append(line)
            else:
                self._hand_over(line)
        # A notification no stream listens to, such as `$/cancel`, needs no answer.
        for request in unserved:
            with anyio.CancelScope(shield=True):
                await self._write(
                    {
                        "id": request.id,
                        "error": {
                            "code": METHOD_NOT_FOUND,
                            "message": f"this Client serves no {request.method}",
                        },
                    }
                )
        if self._queued and len(self._in_flight) < self._cap:
            # Answers freed places: the next queued calls take them.
            with anyio.CancelScope(shield=True):
                await self._drain()

    async def _drain(self) -> None:
        """Sends queued calls, in the order they were made, while places are free.

        One hold of the write lock sends them all, so they reach the
        engine in that order whichever task each call runs in.
        """
        async with self._write_lock:
            while (
                self._queued and len(self._in_flight) < self._cap and not self._ending
            ):
                message_id, request = next(iter(self._queued.items()))
                del self._queued[message_id]
                self._in_flight.add(message_id)
                waiter = self._waiting.get(message_id)
                if waiter is not None:
                    waiter.wake()
                await self._send_locked(request)

    def _hand_over(self, notification: Notification) -> None:
        members = object_members(notification.params)
        stream_id = None if members is None else members.get("id")
        if not isinstance(stream_id, int) or isinstance(stream_id, bool):
            self._owner.notified(notification.method, notification.params)
            return
        listener = self._listeners.get(stream_id)
        if listener is not None:
            listener(notification.method, notification.params)

    def _settle(self, reply: Result | Failure) -> None:
        if not isinstance(reply.id, int) or isinstance(reply.id, bool):
            return
        if reply.id in self._queued:
            # A reply to a call never sent answers nothing.
            return
        # Answered, the call frees its place, a call cancelled since included.
        self._in_flight.discard(reply.id)
        waiter = self._waiting.pop(reply.id, None)
        if waiter is None:
            # The reply to a call cancelled since.
            return
        if isinstance(reply, Result):
            waiter.settle(_Answered(reply.result))
        else:
            error = error_of_reply(reply.error, self._error_classes)
            waiter.settle(_Raised(error, answered=True))

    async def _write(self, message: Mapping[str, object]) -> None:
        await self._write_bytes(encode_message(message))

    async def _write_bytes(self, data: bytes) -> None:
        """Writes `data` whole, its bytes counted as waiting until written."""
        self._unwritten_bytes += len(data)
        # A message is never cut short: cancellation waits for the whole line.
        with anyio.CancelScope(shield=True):
            async with self._write_lock:
                await self._send_locked(data)

    async def _send_locked(self, data: bytes) -> None:
        """Sends `data`, counted as waiting, under the write lock its caller holds."""
        try:
            if self._ending:
                return
            try:
                await self._transport.send(data)
            except Exception as cause:
                await self._finish(WriteFailed(cause))
        finally:
            self._unwritten_bytes -= len(data)

    async def _notify_cancel(self, message_id: int) -> None:
        if self._ending:
            return
        await self._write({"method": "$/cancel", "params": {"id": message_id}})

    async def _finish(self, end: TransportEnd) -> None:
        if self._ending:
            return
        self._ending = True
        with anyio.CancelScope(shield=True):
            failure = await self._owner.ended(end)
            self._failure = failure
            for waiter in [*self._waiting.values(), *self._drainers]:
                waiter.settle(_Raised(failure))
            self._waiting.clear()
            for message_id in list(self._queued):
                self._unqueue(message_id)
            self._in_flight.clear()
            self._ended.set()
            if self._reader_scope is not None:
                self._reader_scope.cancel()


def error_of_reply(
    error: Mapping[str, object], classes: Mapping[str, ErrorClass]
) -> BaseGoodfetchError:
    """The exception an engine's error reply stands for.

    A known Error code gets its class, a `GoodfetchError` but for
    `attempt_ended`'s (ADR 0093); an unknown one gets its group's class,
    named by the reply's wire class or, for a failure of the protocol
    itself, the code JSON-RPC predefines that it takes (ADR 0079), or
    `internal`'s.
    """
    message = error.get("message")
    text = message if isinstance(message, str) else "the engine failed the call"
    details = object_members(error.get("data")) or {}
    code = details.pop("code", None)
    retryable = details.pop("retryable", None)
    fields = _fields_of_data(details)
    if isinstance(code, str) and code in classes:
        return classes[code](text, **fields)
    wire = error.get("code")
    group: ErrorGroup = "internal"
    if isinstance(wire, int) and not isinstance(wire, bool):
        group = ERROR_GROUP_OF_WIRE_CLASS.get(
            wire, ERROR_GROUP_OF_PROTOCOL_FAILURE.get(wire, "internal")
        )
    return ERROR_GROUP_CLASSES[group](
        text,
        code=code if isinstance(code, str) else "internal",
        group=group,
        retryable=retryable is True,
        **fields,
    )


def _text(value: object) -> str | None:
    return value if isinstance(value, str) else None


def _lines(value: object) -> list[str] | None:
    if not isinstance(value, list):
        return None
    lines = [line for line in cast("list[object]", value) if isinstance(line, str)]
    return lines if len(lines) == len(cast("list[object]", value)) else None


def _setting_limit(value: object) -> SettingLimit | None:
    members = object_members(value)
    if members is None or not (
        isinstance(members.get("key"), str) and isinstance(members.get("layer"), str)
    ):
        return None
    return result_of(SettingLimit, members)


def _model_usage(value: object) -> ModelUsage | None:
    members = object_members(value)
    return None if members is None else result_of(ModelUsage, members)


def _take(
    data: dict[str, object], key: str, read: Callable[[object], _Field | None]
) -> _Field | None:
    """`data[key]` read as its field, taken out of `data` when it has the shape."""
    field = read(data.get(key))
    if field is not None:
        del data[key]
    return field


def _fields_of_data(data: dict[str, object]) -> ErrorFields:
    """The fields every error carries, taken out of a reply's `data`.

    What stays in `data` is the code's own details. A code's details may use
    a field's name for something else, such as `response_limit_exceeded`'s
    byte count as `limit`: a value without the field's shape stays in them.
    `limit` and `usage` are built unvalidated, as every result is.
    """
    return {
        "fix": _take(data, "fix", _text),
        "operation": _take(data, "operation", _text),
        "call_id": _take(data, "call_id", _text),
        "run_id": _take(data, "run_id", _text),
        "target": _take(data, "target", _text),
        "url": _take(data, "url", _text),
        "observed": _take(data, "observed", _text),
        "limit": _take(data, "limit", _setting_limit),
        "call_log": _take(data, "call_log", _lines) or [],
        "usage": _take(data, "usage", _model_usage),
        "agent_task": _take(data, "agent_task", object_members),
        "details": data,
    }
