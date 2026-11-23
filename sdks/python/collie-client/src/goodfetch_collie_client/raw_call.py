"""The raw catalog call (ADR 0099, G8): `call(operation, params, *, timeout)`.

On the Connection and on every handle, the way down to an operation the
SDK does not generate, such as one a newer engine added, or one of
Foxhound's from a Collie program on a Foxhound engine. Its params and its
result are the wire's JSON, in snake_case: nothing is parsed into a model
and no point in time read as a `datetime`. On a value, the operation must
be in the value's namespace, and the value's ID fills its parameter. The
rest is the main path's: a closed value fails with `closed`, the timeout
and cancellation apply, failures raise the same classes, nothing is
retried, and the engine validates the params. The call is marked raw in
its `_meta`, the envelope the SDK writes and a program never does (ADR
0132). Hand-written, since no catalog entry shapes it. TypeScript's
`raw-call.ts` is its twin.
"""

from __future__ import annotations

from datetime import timedelta
from typing import TYPE_CHECKING

from pydantic import BaseModel

from goodfetch_collie_client.generated.errors import ProgramError
from goodfetch_collie_client.handle import NotGiven, not_given, whole_milliseconds

if TYPE_CHECKING:
    from collections.abc import Mapping, Sequence

    from pydantic import JsonValue

    from goodfetch_collie_client.handle import AsyncHandleHost, HandleHost

__all__ = ["AsyncHandle", "Handle", "carried_id", "raw_params"]

_META = "_meta"
"""The member of a request's params that holds its envelope (ADR 0132)."""


def _refused(operation: str, reason: str, fix: str) -> ProgramError:
    """The program's error for a raw call refused before it is sent."""
    return ProgramError(
        f"{operation}: {reason}",
        code="invalid_params",
        group="program",
        retryable=False,
        fix=fix,
        details={"operation": operation},
    )


def _wire_id(value_id: object) -> object:
    """A value's ID as the wire carries it, as `ClientConnection.call` dumps params."""
    if isinstance(value_id, BaseModel):
        return value_id.model_dump(
            mode="json", by_alias=True, exclude_unset=True, warnings=False
        )
    return value_id


def _wire_timeout(timeout: object) -> object:
    """A raw call's `timeout` as sent: a `timedelta` in whole milliseconds, rounded up.

    Anything else is sent as given, `None` as null: the engine refuses a
    timeout that is no whole number of 1 or more, naming the operation's
    setting, which the SDK does not know (ADR 0080).
    """
    if isinstance(timeout, timedelta):
        return whole_milliseconds(timeout)
    return timeout


def raw_params(
    operation: str,
    params: Mapping[str, JsonValue] | None,
    timeout: timedelta | NotGiven | None,
    value: tuple[Sequence[str], str, object] | None = None,
) -> dict[str, object]:
    """The params `operation` is sent with, its `timeout` and raw mark among them.

    On `value`, its namespaces, the parameter its ID fills and that ID,
    refuses an operation of another namespace and fills the value's own
    parameter.

    Raises:
        ProgramError: The operation is not the value's, or its params give
            what the call fills, its `_meta` included.
    """
    given: dict[str, object] = {} if params is None else dict(params)
    if _META in given:
        raise _refused(
            operation,
            f"`{_META}` is the call's envelope, which the SDK writes",
            f"Leave `{_META}` out of its params.",
        )
    sent = dict(given)
    if value is not None:
        namespaces, param, value_id = value
        namespace = operation.split(".", 1)[0]
        if namespace not in namespaces:
            names = " or ".join(f"`{name}`" for name in namespaces)
            raise _refused(
                operation,
                f"the value it is called on takes only operations of {names}",
                "Call it on the value of its namespace,"
                " or on the Connection when it acts on no value.",
            )
        if param in given:
            raise _refused(
                operation,
                f"`{param}` is the value's own, which the call fills",
                f"Leave `{param}` out, or call the operation on the value it names.",
            )
        sent[param] = _wire_id(value_id)
    if not isinstance(timeout, NotGiven):
        if "timeout" in given:
            raise _refused(
                operation,
                "`timeout` is given both in its params and as its argument",
                "Give the timeout once, as the argument.",
            )
        sent["timeout"] = _wire_timeout(timeout)
    # The envelope's raw mark, `RequestMeta`'s `raw` (ADR 0099, ADR 0132).
    sent[_META] = {"raw": True}
    return sent


class AsyncHandle:
    """What every async handle a program holds has: the raw call on its value."""

    def __init__(
        self, host: AsyncHandleHost[object], namespace: str, param: str
    ) -> None:
        """Takes its host, its namespace and the parameter its ID fills."""
        self._raw_host = host
        self._namespaces = (namespace,)
        self._param = param

    async def call(
        self,
        operation: str,
        params: Mapping[str, JsonValue] | None = None,
        *,
        timeout: timedelta | NotGiven | None = not_given,
    ) -> JsonValue:
        """Sends `operation`, of this value's namespace, as the wire's JSON (ADR 0099).

        Answers its result as the engine sent it, in snake_case. The value's
        ID fills its own parameter, such as `page_id`. An operation of
        another namespace raises `ProgramError` before it is sent, as a call
        on a closed value raises `ClosedError`. `timeout` is sent as the
        operation's own; cancelling the task cancels the call. It is never
        retried.
        """
        host = self._raw_host
        sent = raw_params(
            operation, params, timeout, (self._namespaces, self._param, host.id)
        )
        return await host.raw(operation, sent)


class Handle:
    """What every sync handle a program holds has: the raw call on its value."""

    def __init__(self, host: HandleHost[object], namespace: str, param: str) -> None:
        """Takes its host, its namespace and the parameter its ID fills."""
        self._raw_host = host
        self._namespaces = (namespace,)
        self._param = param

    def call(
        self,
        operation: str,
        params: Mapping[str, JsonValue] | None = None,
        *,
        timeout: timedelta | NotGiven | None = not_given,
    ) -> JsonValue:
        """Sends `operation`, as `AsyncHandle.call` does, and waits for its result."""
        host = self._raw_host
        sent = raw_params(
            operation, params, timeout, (self._namespaces, self._param, host.id)
        )
        return host.raw(operation, sent)


def carried_id(handle: AsyncHandle | Handle) -> object:
    """The ID `handle`'s calls carry, for a Product's package whose own calls name it.

    Such as Foxhound's `enqueue_links`, which sends the locator it is given
    (ADR 0093); never for programs. TypeScript's `idOf<Name>` functions are
    its twins.
    """
    # Python has no friend access: the handle's host is its own.
    return handle._raw_host.id  # pyright: ignore[reportPrivateUsage]
