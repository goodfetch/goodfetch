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
call, such as the engine's `$/log`, goes to the owner. TypeScript's `rpc.ts`
is its twin; served requests and the channel's bounds come with the tickets
that use them.
"""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import TYPE_CHECKING, Protocol, TypeAlias, TypeVar, cast

import anyio

from goodfetch_collie_client.caller import result_of
from goodfetch_collie_client.framing import (
    Failure,
    Invalid,
    LineReader,
    Notification,
    Request,
    Result,
    encode_message,
    object_members,
)
from goodfetch_collie_client.generated.errors import (
    ERROR_GROUP_CLASSES,
    ERROR_GROUP_OF_PROTOCOL_FAILURE,
    ERROR_GROUP_OF_WIRE_CLASS,
)
from goodfetch_collie_client.generated.models import ModelUsage, SettingLimit

if TYPE_CHECKING:
    from collections.abc import Callable, Mapping

    from anyio.abc import ByteStream

    from goodfetch_collie_client.generated.errors import ErrorGroup
    from goodfetch_collie_client.goodfetch_error import (
        ErrorClass,
        ErrorFields,
        GoodfetchError,
    )

_Field = TypeVar("_Field")

METHOD_NOT_FOUND = -32601
"""JSON-RPC's code for a method the receiver does not serve."""


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
    error: GoodfetchError


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
    ) -> None:
        """Takes the transport, the owner told of its end, and the code classes."""
        self._transport = transport
        self._owner = owner
        self._error_classes = error_classes
        self._lines = LineReader()
        self._next_id = 0
        self._waiting: dict[int, _Waiter] = {}
        self._listeners: dict[int, Callable[[str, object], None]] = {}
        self._drainers: list[_Waiter] = []
        self._reading = False
        self._reader_scope: anyio.CancelScope | None = None
        self._write_lock = anyio.Lock()
        self._ending = False
        self._failure: GoodfetchError | None = None
        self._ended = anyio.Event()

    async def call(self, method: str, params: object) -> object:
        """Sends `method` with `params` and returns the engine's result.

        Raises:
            GoodfetchError: The engine answered with an error, or the
                channel ended or was cancelled before the reply.
        """
        return await self.stream(method, params, None)

    async def stream(
        self,
        method: str,
        params: object,
        notified: Callable[[str, object], None] | None,
        *,
        sent: Callable[[int], None] | None = None,
    ) -> object:
        """Sends `method` as `call` does; `notified` hears each notification naming it.

        `sent` is told the call's id once it is written, so the caller can
        grant credits with `notify`; `notified` runs on the reading task
        and must not block.

        Raises:
            GoodfetchError: The engine answered with an error, or the
                channel ended or was cancelled before the reply.
        """
        if self._failure is not None:
            raise self._failure
        message_id = self._next_id
        self._next_id += 1
        waiter = _Waiter()
        self._waiting[message_id] = waiter
        if notified is not None:
            self._listeners[message_id] = notified
        try:
            await self._write({"id": message_id, "method": method, "params": params})
            if sent is not None:
                sent(message_id)
            outcome = await self._wait_for(waiter)
        except BaseException:
            if waiter.outcome is None:
                del self._waiting[message_id]
                with anyio.CancelScope(shield=True):
                    await self._notify_cancel(message_id)
                self._wake_all()
            raise
        finally:
            self._listeners.pop(message_id, None)
        if isinstance(outcome, _Raised):
            raise outcome.error
        return outcome.result

    async def notify(self, method: str, params: object) -> None:
        """Sends the notification `method`, such as `$/credit`, unless it ended."""
        if self._ending:
            return
        await self._write({"method": method, "params": params})

    def cancel_all(self, error: GoodfetchError) -> list[int]:
        """Fails every call still waiting with `error`, and stops the reader.

        Returns:
            The ids of the calls cancelled, whose `$/cancel` the caller sends
            with `notify_cancelled`.
        """
        cancelled = list(self._waiting)
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
        if not isinstance(outcome, _Raised):
            message = "invariant: a channel ends with the error later calls fail with"
            raise TypeError(message)
        return outcome.error

    async def _wait_for(self, waiter: _Waiter) -> _Answered | _Raised:
        while waiter.outcome is None:
            if self._reading:
                waiter.wakeup = anyio.Event()
                await waiter.wakeup.wait()
                continue
            self._reading = True
            try:
                with anyio.CancelScope() as scope:
                    self._reader_scope = scope
                    while waiter.outcome is None:
                        await self._read_one()
            finally:
                self._reading = False
                self._reader_scope = None
                self._wake_all()
        return waiter.outcome

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
        waiter = self._waiting.pop(reply.id, None)
        if waiter is None:
            # The reply to a call cancelled since.
            return
        if isinstance(reply, Result):
            waiter.settle(_Answered(reply.result))
        else:
            waiter.settle(_Raised(error_of_reply(reply.error, self._error_classes)))

    async def _write(self, message: Mapping[str, object]) -> None:
        data = encode_message(message)
        # A message is never cut short: cancellation waits for the whole line.
        with anyio.CancelScope(shield=True):
            async with self._write_lock:
                if self._ending:
                    return
                try:
                    await self._transport.send(data)
                except Exception as cause:
                    await self._finish(WriteFailed(cause))

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
            self._ended.set()
            if self._reader_scope is not None:
                self._reader_scope.cancel()


def error_of_reply(
    error: Mapping[str, object], classes: Mapping[str, ErrorClass]
) -> GoodfetchError:
    """The exception an engine's error reply stands for.

    A known Error code gets its class; an unknown one gets its group's class,
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
