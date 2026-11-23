"""The JSON-RPC channel a Connection speaks over its transport (ADRs 0024 and 0038).

Each call writes one request and waits for the reply with its id. The
channel runs no task of its own, so it never outlives the caller's task
group: whichever call is waiting while nobody reads becomes the reader,
reads until its own reply has come, resolving the others' replies on the
way, then hands reading over to a call still waiting. A call cancelled
while it waits sends `$/cancel` with its id. A sync call whose caller's
thread is interrupted (`_interruption`) owes its `$/cancel` at once, and
every later write sends the `$/cancel`s owed first, so a request sent
after the interruption never reaches the engine ahead of them, however late
the call's task notices its cancellation (issue 3517). When the engine's side ends,
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
a new call is refused at once with `connection_overloaded`.

Told how with `relink_with`, as a Connection to a remote Daemon is, a
channel whose link drops reaches its engine again instead of ending (spec
606): it holds every call, opens a new link with backoff within
`timeout.connect` of the drop, lets the Connection greet the engine on it
ahead of all, then sends again by its id each call in flight that may be,
a read or an idempotent write, a resumable stream after what it read, and
fails every other one with `connection_lost`. Since no task of its own
reads, a drop is seen once a call reads or writes.

The engine's own requests, such as Foxhound's `crawl_request.handle` (ADR
0093), go to the owner, which runs each where it runs its work and answers
it through its `ServedRequest`; one it does not serve is answered as
JSON-RPC's unknown method, so the engine never waits on it. The engine's
`$/cancel` naming one of them, whose ids are counted apart from the
Client's calls, stops it and answers `call_cancelled`; the channel's end
and a dropped link stop every one, unanswered. TypeScript's `rpc.ts` is its
twin.
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
    MessageId,
    Notification,
    Request,
    Result,
    encode_message,
    object_members,
)
from goodfetch_collie_client._interruption import when_interrupted
from goodfetch_collie_client._relink import lost_call, lost_stream, reconnect_delay
from goodfetch_collie_client.caller import result_of
from goodfetch_collie_client.generated.errors import (
    ERROR_GROUP_CLASSES,
    ERROR_GROUP_OF_PROTOCOL_FAILURE,
    ERROR_GROUP_OF_WIRE_CLASS,
    CallCancelledError,
    ConnectionOverloadedError,
    DeadlineExceededError,
    InternalError,
    SettingInvalidError,
)
from goodfetch_collie_client.generated.models import (
    ModelUsage,
    RunOrigin,
    SettingLimit,
    SettingsLayer,
)
from goodfetch_collie_client.goodfetch_error import (
    BaseGoodfetchError,
    GoodfetchError,
    in_call,
)

if TYPE_CHECKING:
    from collections.abc import Awaitable, Callable, Mapping

    from anyio.abc import ByteStream

    from goodfetch_collie_client._relink import ChannelRelinking, Resumable
    from goodfetch_collie_client.generated.errors import ErrorGroup
    from goodfetch_collie_client.goodfetch_error import ErrorClass, ErrorFields

_Field = TypeVar("_Field")

METHOD_NOT_FOUND = -32601
"""JSON-RPC's code for a method the receiver does not serve."""

_WIRE_CLASS_OF_GROUP: dict[ErrorGroup, int] = {
    group: wire for wire, group in ERROR_GROUP_OF_WIRE_CLASS.items()
}
"""The JSON-RPC error code each group is answered with, its wire class (ADR 0038)."""


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

    def serves(self, request: ServedRequest) -> bool:
        """Whether it serves the engine's `request`, which it then answers.

        It starts `request.serve` where it runs its work and returns; it
        runs on the reading task, so it must not block. A request it does
        not serve is answered as JSON-RPC's unknown method.
        """
        ...


class ServedRequest:
    """A request the engine sent, which the Client serves and answers once."""

    def __init__(
        self,
        request: Request,
        answer: Callable[[ServedRequest, Mapping[str, object]], Awaitable[None]],
    ) -> None:
        """Takes the request and how its answer reaches the engine."""
        self.id: MessageId = request.id
        """Its id, counted apart from the Client's calls."""
        self.method = request.method
        """Its method, such as `crawl_request.handle`."""
        self.params = request.params
        """Its params, as the engine sent them."""
        self._answer = answer
        self._scope = anyio.CancelScope()

    async def serve(self, work: Callable[[], Awaitable[object]]) -> None:
        """Runs `work` and answers the engine with what it returns or raises.

        A Goodfetch error is answered as itself, any other exception as
        `internal`, a bug in what served it. The engine's `$/cancel`, the
        channel's end or a dropped link cancels `work`, and nothing more is
        answered then.
        """
        with self._scope:
            try:
                result = await work()
            except (BaseGoodfetchError, Exception) as error:
                await self._answer(self, {"error": reply_of_error(error)})
                return
            await self._answer(self, {"result": result})

    def stop(self) -> None:
        """Cancels its work, whose answer the engine no longer reads."""
        self._scope.cancel()


@dataclass(frozen=True)
class _Answered:
    result: object


@dataclass(frozen=True)
class _Raised:
    error: BaseGoodfetchError
    answered: bool = False
    """Whether the engine answered with it, rather than the runtime raising it."""


@dataclass(eq=False)
class _Queued:
    """A call held back, its encoded request and the notifications that follow it."""

    request: bytes
    notes: list[bytes] = field(default_factory=list[bytes])
    """Each notification naming the call made while it was held, such as `$/credit`."""

    @property
    def size(self) -> int:
        """Its bytes waiting to be written."""
        return len(self.request) + sum(len(note) for note in self.notes)


@dataclass(eq=False)
class _Drop:
    """A dropped link the channel is reaching its engine again from."""

    end: TransportEnd
    """How the link ended."""
    at: float
    """When it dropped, on anyio's clock."""
    cause: BaseException | None
    """Why the last attempt failed, or the link dropped."""
    made: int = 0
    """How many attempts failed."""
    next_at: float | None = None
    """When the next attempt starts, once its wait was counted."""


class _LinkEndedError(Exception):
    """The new link ended while the greeting used it."""

    def __init__(self, end: TransportEnd) -> None:
        super().__init__("the new link ended while the greeting used it")
        self.end = end


def _cause_of(end: TransportEnd) -> BaseException | None:
    return end.cause if isinstance(end, ReadFailed | WriteFailed) else None


@dataclass(eq=False)
class _Waiter:
    """A call waiting for its reply, or a wait for the channel's end."""

    outcome: _Answered | _Raised | None = None
    wakeup: anyio.Event | None = field(default=None)
    method: str = ""
    """The call's method."""
    request: bytes = b""
    """Its encoded request, as first sent."""
    streams: bool = False
    """Whether notifications naming it are heard, which a drop cannot resume."""
    resume: Resumable | None = None
    """What it is sent again with, after what it read, once its link dropped."""
    written: bool = False
    """Whether its request was written to the current link."""
    interrupted: bool = False
    """Whether its sync caller's thread was interrupted (`_interruption`)."""
    cancel_sent: bool = False
    """Whether its `$/cancel` was written."""

    @property
    def answered(self) -> bool:
        """Whether the engine answered it, which no `$/cancel` follows."""
        outcome = self.outcome
        return isinstance(outcome, _Answered) or (
            isinstance(outcome, _Raised) and outcome.answered
        )

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
        self._queued: dict[int, _Queued] = {}
        """Each call held back, for a place or a new link, in the order it is sent."""
        self._cancels_owed: dict[int, _Waiter] = {}
        """Each written call whose `$/cancel` goes before the next write, in order."""
        self._cap: float = math.inf
        """How many calls may be in flight at once: as `hello` names it, if it does."""
        self._unwritten_bytes = 0
        """The bytes queued or waiting to be written, a message being written too."""
        self._listeners: dict[int, Callable[[str, object], None]] = {}
        self._serving: dict[MessageId, ServedRequest] = {}
        """Each request of the engine's being served, by its id, until answered."""
        self._drainers: list[_Waiter] = []
        self._reading = False
        self._reader_scope: anyio.CancelScope | None = None
        self._write_lock = anyio.Lock()
        self._ending = False
        self._failure: GoodfetchError | None = None
        self._ended = anyio.Event()
        self._relinking: ChannelRelinking | None = None
        self._held = False
        """Whether every call waits for a new link, its old one dropped."""
        self._drop: _Drop | None = None

    def relink_with(self, relinking: ChannelRelinking) -> None:
        """Reaches the engine again with `relinking` whenever the link drops."""
        self._relinking = relinking

    @property
    def relinks(self) -> bool:
        """Whether it reaches its engine again once its link drops."""
        return self._relinking is not None

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
        resume: Resumable | None = None,
    ) -> object:
        """Sends `method` as `call` does; `notified` hears each notification naming it.

        `sent` is told the call's id once it is first sent, ahead of
        anything written later, so the caller can grant credits with
        `notify`, which the engine drops for a call it has not received; a
        call that ended first, as one failed while queued, is never told.
        `notified` runs on the reading task and must not block. Once its
        link dropped, a stream is sent again with `resume`'s params and
        credits when it gives one and the channel resends its method; any
        other fails with `connection_lost`.

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
        started_at = anyio.current_time()
        waiter = _Waiter(
            method=method,
            request=request,
            streams=notified is not None,
            resume=resume,
        )
        self._waiting[message_id] = waiter
        if notified is not None:
            self._listeners[message_id] = notified
        unheard = when_interrupted(lambda: self._interrupted(message_id, waiter))
        try:
            if self._held or self._queued or len(self._in_flight) >= self._cap:
                self._enqueue(message_id, _Queued(request))
            else:
                self._in_flight.add(message_id)
                self._unwritten_bytes += len(request)
                with anyio.CancelScope(shield=True):
                    async with self._write_lock:
                        await self._send_request_locked(message_id, waiter, request)
            outcome = await self._wait_answer(
                message_id, waiter, deadline, started_at, sent
            )
        except BaseException:
            if waiter.outcome is None:
                del self._waiting[message_id]
                # A queued call leaves the queue unsent; a sent one is cancelled.
                if not self._unqueue(message_id) and message_id in self._in_flight:
                    with anyio.CancelScope(shield=True):
                        await self._cancel(message_id, waiter)
                self._wake_all()
            raise
        finally:
            unheard()
            self._listeners.pop(message_id, None)
        if isinstance(outcome, _Raised):
            raise outcome.error if outcome.answered else in_call(outcome.error, method)
        return outcome.result

    def cap_calls_in_flight(self, cap: int) -> None:
        """Keeps at most `cap` calls in flight from now on, as `hello`'s result says.

        Set once `hello` is answered, before any call can be queued.
        """
        self._cap = cap

    async def _wait_answer(
        self,
        message_id: int,
        waiter: _Waiter,
        deadline: CallDeadline | None,
        started_at: float,
        sent: Callable[[int], None] | None,
    ) -> _Answered | _Raised:
        """Waits for the call's answer, queued again whenever its link drops.

        `sent` is told its id the first time it is sent.
        """
        told = sent is None
        while True:
            if message_id in self._queued and waiter.outcome is None:
                await self._wait_queued(message_id, waiter, deadline, started_at)
            if waiter.outcome is not None:
                return waiter.outcome
            if not told and sent is not None:
                told = True
                sent(message_id)
            await self._wait_until(
                waiter,
                lambda: waiter.outcome is not None or message_id in self._queued,
            )

    def _enqueue(self, message_id: int, queued: _Queued) -> None:
        """Holds the call back, after every call held before it."""
        self._queued[message_id] = queued
        self._unwritten_bytes += queued.size

    async def _wait_queued(
        self,
        message_id: int,
        waiter: _Waiter,
        deadline: CallDeadline | None,
        started_at: float,
    ) -> None:
        """Holds the call back until the reader sends it, as a place or a link frees.

        `deadline` counts from `started_at`, when the call was made.

        Raises:
            DeadlineExceededError: `deadline` passed first; the call, never
                sent, or not sent again, is still queued, for the caller to drop.
        """

        def left_the_queue() -> bool:
            return waiter.outcome is not None or message_id not in self._queued

        if deadline is None:
            await self._wait_until(waiter, left_the_queue)
            return
        with anyio.CancelScope(deadline=started_at + deadline.value / 1000):
            await self._wait_until(waiter, left_the_queue)
        if left_the_queue():
            return
        limit = SettingLimit(
            key=deadline.key,
            value=deadline.value,
            layer=SettingsLayer.run,
            origin=RunOrigin.argument,
        )
        if self._held:
            exceeded = phase_exceeded(
                f"the call waited {deadline.value} ms, its `{deadline.key}`, while"
                " the Client reached the engine again after the link dropped, so it"
                " was not sent",
                fix=(
                    "Give the call a longer `timeout`, or make it again once the"
                    " engine is reached."
                ),
                limit=limit,
                started_at=started_at,
                waited_for="hello",
            )
        else:
            exceeded = phase_exceeded(
                f"the call waited {deadline.value} ms, its `{deadline.key}`, for a"
                " place among the engine's calls in flight, so it was not sent",
                fix=(
                    "Make fewer calls at once on this Connection, or give the call"
                    " a longer `timeout`."
                ),
                limit=limit,
                started_at=started_at,
                waited_for="calls_in_flight",
            )
        raise in_call(exceeded, waiter.method)

    def _unqueue(self, message_id: int) -> bool:
        """Takes the call `message_id` out of the queue: whether it was queued."""
        queued = self._queued.pop(message_id, None)
        if queued is None:
            return False
        self._unwritten_bytes -= queued.size
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
        """Sends the notification `method`, such as `$/credit`, unless it ended.

        One naming a call held back follows that call's request; while the
        link is down, one naming no call held is dropped, as the engine
        that would have read it is gone.
        """
        if self._ending:
            return
        data = encode_message({"method": method, "params": params})
        members = object_members(params) or {}
        named = members.get("id")
        if isinstance(named, int) and not isinstance(named, bool):
            waiter = self._waiting.get(named)
            credits = members.get("credits")
            if (
                method == "$/credit"
                and waiter is not None
                and waiter.resume is not None
                and isinstance(credits, int)
                and not isinstance(credits, bool)
            ):
                waiter.resume.granted(credits)
            queued = self._queued.get(named)
            if queued is not None:
                queued.notes.append(data)
                self._unwritten_bytes += len(data)
                return
        if self._held:
            return
        await self._write_bytes(data)

    def cancel_all(self, error: GoodfetchError) -> list[int]:
        """Fails every call still waiting with `error`, and stops the reader.

        Returns:
            The ids of the sent calls cancelled, whose `$/cancel` the caller
            sends with `notify_cancelled`; a queued one is never sent, nor
            one whose `$/cancel` was, and those owed come first.
        """
        cancelled = list(self._cancels_owed)
        self._cancels_owed.clear()
        cancelled += [
            message_id
            for message_id, waiter in self._waiting.items()
            if not self._unqueue(message_id)
            and not waiter.cancel_sent
            and message_id not in cancelled
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
        if self._drop is not None:
            await self._relink_step(self._drop)
            return
        transport = self._transport
        try:
            chunk = await transport.receive()
        except anyio.EndOfStream:
            await self._link_broke(transport, Ended())
            return
        except Exception as cause:
            await self._link_broke(transport, ReadFailed(cause))
            return
        end = await self._read_chunk(chunk)
        if end is not None:
            await self._finish(end)
            return
        if not self._held and self._queued and len(self._in_flight) < self._cap:
            # Answers freed places: the next queued calls take them.
            with anyio.CancelScope(shield=True):
                await self._drain()

    async def _read_chunk(self, chunk: bytes) -> InvalidLine | None:
        """Settles the replies `chunk` ends and hands its messages over.

        Returns:
            The end of a line that is not one JSON-RPC message, if one is.
        """
        answers: list[Mapping[str, object]] = []
        for line in self._lines.feed(chunk):
            if isinstance(line, Invalid):
                return InvalidLine(line.line)
            if isinstance(line, Result | Failure):
                self._settle(line)
            elif isinstance(line, Request):
                unserved = self._serve(line)
                if unserved is not None:
                    answers.append(unserved)
            elif line.method == "$/cancel":
                cancelled = self._cancel_served(line.params)
                if cancelled is not None:
                    answers.append(cancelled)
            else:
                self._hand_over(line)
        for answer in answers:
            with anyio.CancelScope(shield=True):
                await self._write(answer)
        return None

    def _serve(self, request: Request) -> Mapping[str, object] | None:
        """Hands the engine's `request` to the owner: the answer if it serves none."""
        served = ServedRequest(request, self._answer_served)
        self._serving[request.id] = served
        if self._owner.serves(served):
            return None
        del self._serving[request.id]
        return {
            "id": request.id,
            "error": {
                "code": METHOD_NOT_FOUND,
                "message": f"this Client serves no {request.method}",
            },
        }

    def _cancel_served(self, params: object) -> Mapping[str, object] | None:
        """Stops the served request the engine's `$/cancel` names: its answer.

        The engine cancels as the requesting Client, so `by` is `client`
        (ADR 0113). A `$/cancel` naming no request being served, such as
        one answered already, needs no answer.
        """
        members = object_members(params)
        served_id = None if members is None else members.get("id")
        if isinstance(served_id, bool) or not isinstance(served_id, str | int | float):
            return None
        served = self._serving.pop(served_id, None)
        if served is None:
            return None
        served.stop()
        cancelled = CallCancelledError(
            "the engine cancelled its request", details={"by": "client"}
        )
        return {"id": served.id, "error": reply_of_error(cancelled)}

    async def _answer_served(
        self, served: ServedRequest, answer: Mapping[str, object]
    ) -> None:
        """Sends a served request's answer, unless it was stopped first."""
        if self._serving.get(served.id) is not served:
            return
        del self._serving[served.id]
        await self._write({"id": served.id, **answer})

    def _stop_served(self) -> None:
        """Stops every request being served: no answer reaches the engine any more."""
        serving = list(self._serving.values())
        self._serving.clear()
        for served in serving:
            served.stop()

    async def _link_broke(self, transport: ByteStream, end: TransportEnd) -> None:
        """Ends the channel as `transport` ended, or reaches the engine again.

        It reaches the engine again when told how and the Connection is not
        closing. An end of a link already dropped, or of one being reached,
        is that attempt's to see.
        """
        if transport is not self._transport or self._drop is not None:
            return
        relinking = self._relinking
        if relinking is None or relinking.closing():
            await self._finish(end)
            return
        self._dropped(end)
        with anyio.CancelScope(shield=True):
            await _close_quietly(transport)
        self._wake_all()

    def _dropped(self, end: TransportEnd) -> None:
        """Holds every call from now on, once the link dropped.

        Each call in flight that may be is sent again by its id, first, and
        every other one fails with `connection_lost`; the calls queued
        before follow.
        """
        self._held = True
        self._drop = _Drop(end, anyio.current_time(), _cause_of(end))
        earlier = self._queued
        self._queued = {}
        for message_id, waiter in list(self._waiting.items()):
            if message_id in earlier:
                continue
            again = self._again_of(message_id, waiter)
            if again is None:
                del self._waiting[message_id]
                waiter.settle(_Raised(lost_stream() if waiter.streams else lost_call()))
            else:
                waiter.written = False
                self._enqueue(message_id, again)
        self._queued.update(earlier)
        # A call cancelled and not answered yet holds no place on the new link.
        self._in_flight.clear()
        self._cancels_owed.clear()
        # The engine's requests on the old link are answered on none.
        self._stop_served()

    def _again_of(self, message_id: int, waiter: _Waiter) -> _Queued | None:
        """What the call is sent again with on the new link, if it may be."""
        relinking = self._relinking
        if relinking is None or not relinking.resends(waiter.method):
            return None
        if waiter.resume is not None:
            params, credits = waiter.resume.resumed()
            request = encode_message(
                {"id": message_id, "method": waiter.method, "params": params}
            )
            notes = (
                [
                    encode_message(
                        {
                            "method": "$/credit",
                            "params": {"id": message_id, "credits": credits},
                        }
                    )
                ]
                if credits > 0
                else []
            )
            return _Queued(request, notes)
        if waiter.streams:
            return None
        return _Queued(waiter.request)

    async def _relink_step(self, drop: _Drop) -> None:
        """Waits for the next attempt to reach the engine again, then makes it.

        Past `timeout.connect` from the drop, the channel ends with the
        relinking's `gave_up`; a closing Connection ends it as its owner
        says. A cancelled attempt counts as one made, so the next reader
        waits the next delay before it makes another, and a greeting that
        fails for any other reason than the engine's answer or the new link
        ending ends the channel with `gave_up`.
        """
        relinking = self._relinking
        if relinking is None or relinking.closing():
            await self._finish(drop.end)
            return
        if drop.next_at is None:
            delay = reconnect_delay(
                drop.made, anyio.current_time() - drop.at, relinking.within
            )
            if delay is None:
                await self._finish(drop.end, relinking.gave_up(drop.cause))
                return
            drop.next_at = anyio.current_time() + delay
        await anyio.sleep_until(drop.next_at)
        if relinking.closing():
            await self._finish(drop.end)
            return
        within = math.inf if relinking.within is None else drop.at + relinking.within
        transport: ByteStream | None = None
        try:
            with anyio.CancelScope(deadline=within) as attempt:
                transport = await relinking.reopen()
                self._transport = transport
                self._lines = LineReader()
                await relinking.greet(self._ahead)
        except _LinkEndedError as ended:
            if isinstance(ended.end, InvalidLine):
                await self._finish(ended.end)
                return
            await self._failed(drop, transport, ended.__cause__ or ended)
            return
        except GoodfetchError as refused:
            if transport is None:
                # Reopening failed: nothing answers yet, so it is tried again.
                await self._failed(drop, transport, refused)
                return
            await self._finish(drop.end, refused)
            return
        except Exception as cause:
            if transport is None:
                await self._failed(drop, transport, cause)
                return
            with anyio.CancelScope(shield=True):
                await _close_quietly(transport)
            await self._finish(drop.end, relinking.gave_up(cause))
            return
        except BaseException:
            # Cancelled with its caller: the next reader makes another attempt
            # once the next delay passed, so short calls keep the backoff.
            drop.made += 1
            drop.next_at = None
            if transport is not None:
                with anyio.CancelScope(shield=True):
                    await _close_quietly(transport)
            raise
        if attempt.cancelled_caught:
            await self._failed(drop, transport, TimeoutError("timeout.connect passed"))
            return
        self._drop = None
        self._held = False
        with anyio.CancelScope(shield=True):
            await self._drain()

    async def _failed(
        self, drop: _Drop, transport: ByteStream | None, cause: BaseException
    ) -> None:
        """Counts a failed attempt, whose new link is closed, for the next one."""
        drop.made += 1
        drop.next_at = None
        drop.cause = cause
        if transport is not None:
            with anyio.CancelScope(shield=True):
                await _close_quietly(transport)

    async def _ahead(self, method: str, params: object) -> object:
        """Makes the call `method` on the new link, ahead of every call held.

        It reads the new link itself until the answer comes.

        Raises:
            _LinkEndedError: The new link ended first.
            GoodfetchError: The engine answered with an error.
        """
        message_id = self._next_id
        self._next_id += 1
        waiter = _Waiter(method=method)
        self._waiting[message_id] = waiter
        self._in_flight.add(message_id)
        transport = self._transport
        try:
            data = encode_message(
                {"id": message_id, "method": method, "params": params}
            )
            with anyio.CancelScope(shield=True):
                async with self._write_lock:
                    try:
                        await transport.send(data)
                    except Exception as cause:
                        raise _LinkEndedError(WriteFailed(cause)) from cause
            while waiter.outcome is None:
                try:
                    chunk = await transport.receive()
                except anyio.EndOfStream:
                    raise _LinkEndedError(Ended()) from None
                except Exception as cause:
                    raise _LinkEndedError(ReadFailed(cause)) from cause
                end = await self._read_chunk(chunk)
                if end is not None:
                    raise _LinkEndedError(end)
        finally:
            self._waiting.pop(message_id, None)
            self._in_flight.discard(message_id)
        outcome = waiter.outcome
        if isinstance(outcome, _Raised):
            raise outcome.error
        return outcome.result

    async def _drain(self) -> None:
        """Sends queued calls, in the order they were made, while places are free.

        One hold of the write lock sends them all, so they reach the
        engine in that order whichever task each call runs in; the
        notifications held with a call follow it.
        """
        async with self._write_lock:
            while (
                self._queued
                and len(self._in_flight) < self._cap
                and not self._ending
                and not self._held
            ):
                message_id, queued = next(iter(self._queued.items()))
                del self._queued[message_id]
                self._in_flight.add(message_id)
                waiter = self._waiting.get(message_id)
                if waiter is None:
                    await self._send_locked(queued.request)
                else:
                    waiter.wake()
                    await self._send_request_locked(message_id, waiter, queued.request)
                for note in queued.notes:
                    await self._send_locked(note)

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

    async def _send_locked(self, data: bytes) -> bool:
        """Sends `data`, counted as waiting, under the write lock its caller holds.

        The `$/cancel`s owed when it starts go first.

        Returns:
            Whether `data` was written, which it is not while the link is
            down or once the channel ends.
        """
        try:
            await self._send_cancels_owed_locked()
            return await self._transmit_locked(data)
        finally:
            self._unwritten_bytes -= len(data)

    async def _transmit_locked(self, data: bytes) -> bool:
        """Writes `data` alone under the write lock: whether it was written."""
        if self._ending or self._held:
            return False
        transport = self._transport
        try:
            await transport.send(data)
        except Exception as cause:
            await self._link_broke(transport, WriteFailed(cause))
            return False
        return True

    async def _send_request_locked(
        self, message_id: int, waiter: _Waiter, request: bytes
    ) -> None:
        """Sends the call's request under the write lock its caller holds.

        A call interrupted while its request was being written owes its
        `$/cancel` once it is.
        """
        waiter.written = await self._send_locked(request)
        if waiter.interrupted:
            self._owe_cancel(message_id, waiter)

    def _interrupted(self, message_id: int, waiter: _Waiter) -> None:
        """Its sync caller's thread was interrupted: the call owes its `$/cancel`.

        Run on the loop before that thread goes on, while the call's task
        may notice its cancellation later. A call not written yet owes it
        once it is; one still queued is never sent, as its task takes it
        out of the queue.
        """
        waiter.interrupted = True
        self._owe_cancel(message_id, waiter)

    def _owe_cancel(self, message_id: int, waiter: _Waiter) -> None:
        """Owes the call's `$/cancel`, unless it is not written or needs none."""
        if waiter.written and not waiter.cancel_sent and not waiter.answered:
            self._cancels_owed.setdefault(message_id, waiter)

    async def _cancel(self, message_id: int, waiter: _Waiter) -> None:
        """Sends the call's `$/cancel`, once, after those owed before it."""
        if waiter.cancel_sent:
            return
        self._cancels_owed.setdefault(message_id, waiter)
        async with self._write_lock:
            await self._send_cancels_owed_locked()

    async def _send_cancels_owed_locked(self) -> None:
        """Sends the `$/cancel`s owed now, in order, under the write lock.

        A call answered since needs none; one owed while they are sent waits
        for the next write, so it never goes ahead of the request being sent.
        """
        owed = list(self._cancels_owed.items())
        self._cancels_owed.clear()
        for message_id, waiter in owed:
            waiter.cancel_sent = True
            if not waiter.answered:
                await self._transmit_locked(
                    encode_message({"method": "$/cancel", "params": {"id": message_id}})
                )

    async def _notify_cancel(self, message_id: int) -> None:
        if self._ending or self._held:
            # While the link is down, no call is in flight to cancel.
            return
        await self._write({"method": "$/cancel", "params": {"id": message_id}})

    async def _finish(
        self, end: TransportEnd, failure: GoodfetchError | None = None
    ) -> None:
        """Ends the channel: every call waiting, and every later one, fails.

        With `failure`, or else with the error its owner says `end` takes.
        """
        if self._ending:
            return
        self._ending = True
        with anyio.CancelScope(shield=True):
            if failure is None:
                failure = await self._owner.ended(end)
            self._failure = failure
            for waiter in [*self._waiting.values(), *self._drainers]:
                waiter.settle(_Raised(failure))
            self._waiting.clear()
            for message_id in list(self._queued):
                self._unqueue(message_id)
            self._in_flight.clear()
            self._stop_served()
            self._drop = None
            self._held = False
            self._ended.set()
            if self._reader_scope is not None:
                self._reader_scope.cancel()


async def _close_quietly(transport: ByteStream) -> None:
    """Closes both sides of a link that dropped or failed."""
    try:
        await transport.aclose()
    except Exception:
        # Nothing it held matters any more.
        return


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


def reply_of_error(error: BaseException) -> dict[str, object]:
    """The JSON-RPC `error` a request the engine sent is answered with.

    `error` is what serving it failed with; one that is no Goodfetch error
    is a bug in what served it, answered `internal`. The inverse of
    `error_of_reply`.
    """
    failure = (
        error
        if isinstance(error, BaseGoodfetchError)
        else InternalError(
            str(error), code="internal", group="internal", retryable=False
        )
    )
    data: dict[str, object] = {
        **failure.details,
        "code": failure.code,
        "retryable": failure.retryable,
    }
    fields: dict[str, object] = {
        "fix": failure.fix,
        "operation": failure.operation,
        "call_id": failure.call_id,
        "run_id": failure.run_id,
        "target": failure.target,
        "url": failure.url,
        "observed": failure.observed,
        "limit": None
        if failure.limit is None
        else failure.limit.model_dump(mode="json", by_alias=True, exclude_none=True),
        "call_log": failure.call_log or None,
        "usage": None
        if failure.usage is None
        else failure.usage.model_dump(mode="json", by_alias=True, exclude_none=True),
        "agent_task": failure.agent_task,
    }
    data.update({key: value for key, value in fields.items() if value is not None})
    return {
        "code": _WIRE_CLASS_OF_GROUP[failure.group],
        "message": str(failure),
        "data": data,
    }


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
