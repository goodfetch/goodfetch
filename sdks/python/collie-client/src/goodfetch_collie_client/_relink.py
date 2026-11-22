"""How a Connection to a remote Daemon reaches it again once its link drops.

Spec 606 and ADR 0079: within `timeout.connect` of the drop, the channel
opens a new link, 250 ms before the first attempt, doubling each time and
capped at 10 s; the Connection greets the Daemon with the same `hello` and
sets its Limits again; then each call in flight whose operation is a read or
an idempotent write is sent again by its id, `events.subscribe` after the
last event it read, and every other one fails with `connection_lost`.
TypeScript's `reconnect.ts` and `resumed-stream.ts` are its twins.
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import TYPE_CHECKING, Protocol, cast

from goodfetch_collie_client._framing import object_members
from goodfetch_collie_client.generated.errors import ConnectionLostError

if TYPE_CHECKING:
    from collections.abc import Awaitable, Callable, Mapping

    from anyio.abc import ByteStream

    from goodfetch_collie_client.goodfetch_error import GoodfetchError

FIRST_DELAY = 0.25
"""The wait before the first attempt, in seconds."""

LONGEST_DELAY = 10.0
"""The longest wait between two attempts, in seconds."""

EVENTS = "$/events"
"""The notification that carries a batch of `events.subscribe`'s events."""


def reconnect_delay(attempt: int, elapsed: float, within: float | None) -> float | None:
    """The seconds to wait before attempt `attempt`, the first being 0.

    `elapsed` seconds after the drop; `None` once that attempt would start
    at or past `within`, the `timeout.connect` in force, which `None` leaves
    unbounded.
    """
    delay = min(FIRST_DELAY * 2**attempt, LONGEST_DELAY)
    return None if within is not None and elapsed + delay >= within else delay


def lost_call() -> ConnectionLostError:
    """The error a call in flight when its link dropped fails with, not sent again."""
    return ConnectionLostError(
        "the Connection to the engine dropped before the call was answered, so"
        " whether it took effect is unknown, and it was not sent again",
        fix=(
            "Check the Run's Effect journal for whether it happened before making"
            " the call again: the Client reaches the engine again by itself."
        ),
    )


def lost_stream() -> ConnectionLostError:
    """The error a stream open when its link dropped fails with, when not resumed."""
    return ConnectionLostError(
        "the Connection to the engine dropped while the stream was open, and it"
        " cannot be resumed, so it was not sent again",
        fix="Open the stream again: the Client reaches the engine again by itself.",
    )


@dataclass(frozen=True)
class ChannelRelinking:
    """What a channel reaches its engine again with, once its link drops."""

    resends: Callable[[str], bool]
    """Whether a call in flight of that method is sent again on the new link."""
    within: float | None
    """The seconds after the drop past which no attempt starts; `None` for none."""
    closing: Callable[[], bool]
    """Whether the Connection's close began, which ends the channel instead."""
    reopen: Callable[[], Awaitable[ByteStream]]
    """Opens a new link to the engine."""
    greet: Callable[[Callable[[str, object], Awaitable[object]]], Awaitable[None]]
    """Greets the engine on the new link, with the call it is given, ahead of all."""
    gave_up: Callable[[BaseException | None], GoodfetchError]
    """The error the channel ends with once no attempt reached the engine in time."""


@dataclass(frozen=True)
class Reconnect:
    """How a Connection to a remote Daemon reaches it again once its link drops."""

    operation_classes: Mapping[str, str]
    """Each operation's class by its catalog name (ADR 0011)."""
    reopen: Callable[[], Awaitable[ByteStream]]
    """Opens a new link to the Daemon."""


class Resumable(Protocol):
    """A stream the channel sends again on a new link, after what it read."""

    def granted(self, credits: int) -> None:
        """Counts the credits a `$/credit` naming the stream grants."""
        ...

    def resumed(self) -> tuple[object, int]:
        """Its params on the new link, and the credits it still had."""
        ...


class ResumedEvents:
    """`events.subscribe`, sent again after the last event it read (spec 606).

    As the engine instance that published that event, with the credits it
    granted less the events it received, so its readers lose no event.
    """

    def __init__(
        self,
        notified: Callable[[str, object], None] | None,
        engine_instance: Callable[[], str | None],
    ) -> None:
        """Wraps `notified`; `engine_instance` names the engine instance reached."""
        self._wrapped = notified
        self._engine_instance = engine_instance
        self._last: dict[str, object] | None = None
        self._granted = 0
        self._received = 0

    def notified(self, method: str, params: object) -> None:
        """Counts a batch's events, keeps the last one's position, and hands it on."""
        if method == EVENTS:
            positions = _positions_of(params)
            self._received += len(positions)
            instance = self._engine_instance()
            if positions and instance is not None:
                self._last = {"engine_instance": instance, "position": positions[-1]}
        if self._wrapped is not None:
            self._wrapped(method, params)

    def granted(self, credits: int) -> None:
        """Counts the credits granted."""
        self._granted += credits

    def resumed(self) -> tuple[object, int]:
        """`{}` before any event was read, else after the last; the credits left."""
        params: dict[str, object] = {} if self._last is None else {"after": self._last}
        return params, self._granted - self._received


def _positions_of(params: object) -> list[object]:
    """The position of each event of a `$/events` batch, in order."""
    members = object_members(params)
    events = None if members is None else members.get("events")
    if not isinstance(events, list):
        return []
    positions: list[object] = []
    for item in cast("list[object]", events):
        fields = object_members(item)
        if fields is not None and isinstance(fields.get("event"), str):
            positions.append(fields.get("position"))
    return positions
