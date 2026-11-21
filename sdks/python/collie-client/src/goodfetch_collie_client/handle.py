"""What a generated handle reaches its engine through (ADR 0099).

The raw call, the ID its calls carry, the host of each handle it opens
or builds, and its listeners, for the async handles and their sync twins,
`not_given`, what an optional parameter left out holds, `timeout_of`, how
a call's own `timeout` is sent, and the `Subscription` a listener's `on`
returns (ADR 0081). Hand-written, since no catalog entry shapes it; the
client package's runtime implements it. TypeScript's `handle.ts` is its
twin.
"""

from __future__ import annotations

from collections.abc import Awaitable, Callable
from datetime import timedelta
from typing import TYPE_CHECKING, Final, Literal, Protocol, TypeAlias, TypeVar, final

from .generated.errors import SettingInvalidError
from .generated.models import RunOrigin, SettingLimit, SettingsLayer

if TYPE_CHECKING:
    from collections.abc import Mapping
    from typing import Self

    from pydantic import BaseModel, JsonValue

__all__ = [
    "AsyncHandleHost",
    "AsyncListener",
    "HandleHost",
    "Listener",
    "NotGiven",
    "Subscription",
    "not_given",
    "timeout_of",
]

_Id = TypeVar("_Id", covariant=True)
_ChildId = TypeVar("_ChildId")
_Payload = TypeVar("_Payload")
_Model = TypeVar("_Model", bound="BaseModel")

AsyncListener: TypeAlias = Callable[[_Payload], Awaitable[None] | None]
"""A listener of the async API: it gets each event's payload, one at a time.

An awaitable it returns, as an `async def` listener's call does, is awaited
before the next event (ADR 0081).
"""

Listener: TypeAlias = Callable[[_Payload], None]
"""A listener of the sync API: it gets each event's payload, one at a time.

It runs on a worker thread its Connection owns, never on the program's.
"""


@final
class Subscription:
    """A listener registered with `on`, which `close()` removes (ADR 0081).

    Closing it is idempotent and waits for nothing: the listener gets no
    event after it, and its call's `$/cancel` reaches the engine before any
    later call of the program does. It is a context manager in both APIs,
    whose block removes the listener. A program never constructs one.
    """

    def __init__(self, close: Callable[[], None]) -> None:
        """Takes what removes the listener."""
        self._close = close

    def close(self) -> None:
        """Removes the listener; idempotent."""
        self._close()

    def __enter__(self) -> Self:
        """Holds the listener for a `with` block, which removes it."""
        return self

    def __exit__(self, *_: object) -> None:
        """Removes the listener."""
        self.close()


@final
class NotGiven:
    """What an optional parameter left out holds (ADR 0080's three states).

    The call sends nothing for it, so the engine applies its default, such
    as a timeout's setting, where `None` sends null, such as no timeout.
    """

    def __bool__(self) -> Literal[False]:
        """False, as a parameter left out sets nothing."""
        return False

    def __repr__(self) -> str:
        """`not_given`, the one value of its type."""
        return "not_given"


not_given: Final = NotGiven()
"""The default of each optional parameter of a generated handle."""

_TIMEOUT_FIX: Final = "give a `timedelta` above zero, or `timeout=None` for no timeout"
_MICROSECOND: Final = timedelta(microseconds=1)


def whole_milliseconds(duration: timedelta) -> int:
    """`duration` in whole milliseconds, a part of one counting as one (ADR 0080)."""
    return -(-(duration // _MICROSECOND) // 1000)


def timeout_of(timeout: object, setting: str) -> int | None:
    """A call's own `timeout` as sent (ADR 0080): null or whole milliseconds.

    `None` is no timeout, sent as null, and a `timedelta` is sent in whole
    milliseconds, rounded up. It takes any value, since a program may pass
    one its type checker would refuse: zero or less, or anything but a
    `timedelta`, raises `SettingInvalidError` before anything is sent, its
    `limit` naming `setting`, the Duration setting the `timeout` sets for
    the call alone, at the run layer, given as an argument, and its `fix`
    the null form, as the engine's refusal of `0` does. TypeScript's
    `timeoutOf` is its twin.
    """
    if timeout is None:
        return None
    if not isinstance(timeout, timedelta):
        value: object = timeout
        reason = f"a timeout is a `timedelta`, not {type(timeout).__name__}"
    else:
        microseconds = timeout // _MICROSECOND
        if microseconds > 0:
            return whole_milliseconds(timeout)
        value = (
            microseconds // 1000 if microseconds % 1000 == 0 else microseconds / 1000
        )
        reason = (
            "a timeout of 0 ms would end the call at once"
            if microseconds == 0
            else f"a timeout of {value} ms is negative"
        )
    raise SettingInvalidError(
        f"The call's `timeout`, its `{setting}`: {reason}; {_TIMEOUT_FIX}.",
        fix=_TIMEOUT_FIX,
        limit=SettingLimit(
            key=setting,
            value=value,
            layer=SettingsLayer.run,
            origin=RunOrigin.argument,
        ),
        details={"key": setting, "layer": "run", "reason": reason},
    )


class AsyncHandleHost(Protocol[_Id]):
    """What an async handle of ID `_Id` reaches its engine through."""

    @property
    def id(self) -> _Id:
        """The ID each of the handle's calls carries."""
        ...

    async def call(self, operation: str, params: BaseModel) -> object:
        """Sends one call and returns its result as the engine sent it."""
        ...

    async def raw(self, operation: str, params: Mapping[str, object]) -> JsonValue:
        """Sends the raw call `operation` unless the value is closed (ADR 0099).

        Its params and its result are the wire's JSON, unconverted.
        """
        ...

    def child(self, id: _ChildId) -> AsyncHandleHost[_ChildId]:
        """The host of a handle this one opens or builds, whose calls carry `id`."""
        ...

    async def close(self, operation: str, params: BaseModel) -> None:
        """Closes the handle through `operation`, its own `close` (ADR 0078).

        Idempotent: it is sent once, and nothing is sent for a handle already
        closed, by its program or with the value that opened it. Every other
        call on the handle, and on a handle it opened or built, then fails
        with `closed`. A listener of the handle, or of one it opened or
        built, that threw and that nothing raised yet is raised then, as
        `listener_failed`, once the handle is closed (ADR 0081).
        """
        ...

    def listen(
        self,
        operation: str,
        params: BaseModel,
        event: str,
        payload: type[_Model],
        listener: AsyncListener[_Model],
    ) -> Subscription:
        """Sends the listener's call `operation`; `listener` gets `event`'s payloads.

        The call is sent in the order the program called, with no await of
        the program's: `on` is a plain method in both APIs (ADR 0081).
        """
        ...

    def listen_published(
        self,
        operation: str,
        params: BaseModel,
        event: str,
        param: str,
        payload: type[_Model],
        listener: AsyncListener[_Model],
    ) -> Subscription:
        """Registers `listener` for `event`, which the engine publishes (ADR 0081).

        Such as `session.on`'s: the Connection's own Subscription is sent
        first when none is in flight, then the call `operation`, which only
        arms the listener in the engine and stays in flight until the
        `Subscription` closes or the value goes. The listener gets each
        `event` of the Connection's Subscription whose payload names the
        handle by `param`, in the order the program called, as `listen`.
        """
        ...


class HandleHost(Protocol[_Id]):
    """What a sync handle of ID `_Id` reaches its engine through."""

    @property
    def id(self) -> _Id:
        """The ID each of the handle's calls carries."""
        ...

    def call(self, operation: str, params: BaseModel) -> object:
        """Sends one call and returns its result as the engine sent it."""
        ...

    def raw(self, operation: str, params: Mapping[str, object]) -> JsonValue:
        """Sends the raw call `operation`, as `AsyncHandleHost.raw` does."""
        ...

    def child(self, id: _ChildId) -> HandleHost[_ChildId]:
        """The host of a handle this one opens or builds, whose calls carry `id`."""
        ...

    def close(self, operation: str, params: BaseModel) -> None:
        """Closes the handle through `operation`, its own `close` (ADR 0078).

        Idempotent, as `AsyncHandleHost.close` is.
        """
        ...

    def listen(
        self,
        operation: str,
        params: BaseModel,
        event: str,
        payload: type[_Model],
        listener: Listener[_Model],
    ) -> Subscription:
        """Sends the listener's call, as `AsyncHandleHost.listen` does."""
        ...

    def listen_published(
        self,
        operation: str,
        params: BaseModel,
        event: str,
        param: str,
        payload: type[_Model],
        listener: Listener[_Model],
    ) -> Subscription:
        """Registers the listener, as `AsyncHandleHost.listen_published` does."""
        ...
