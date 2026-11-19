"""What a generated handle reaches its engine through (ADR 0099).

The raw call, the ID its calls carry, and the host of each handle it opens
or builds, for the async handles and their sync twins, `not_given`, what
an optional parameter left out holds, and `timeout_of`, how a call's own
`timeout` is sent. Hand-written, since no catalog entry
shapes it; the client package's runtime implements it. TypeScript's
`handle.ts` is its twin.
"""

from __future__ import annotations

from datetime import timedelta
from typing import TYPE_CHECKING, Final, Literal, Protocol, TypeVar, final

from .generated.errors import SettingInvalidError

if TYPE_CHECKING:
    from collections.abc import Mapping

    from pydantic import BaseModel, JsonValue

_Id = TypeVar("_Id", covariant=True)
_ChildId = TypeVar("_ChildId")


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
        microseconds = (
            timeout.days * 86_400_000_000
            + timeout.seconds * 1_000_000
            + timeout.microseconds
        )
        if microseconds > 0:
            return -(-microseconds // 1000)
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
        details={
            "key": setting,
            "layer": "run",
            "reason": reason,
            "limit": {
                "key": setting,
                "value": value,
                "layer": "run",
                "origin": "argument",
            },
        },
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
        with `closed`.
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
