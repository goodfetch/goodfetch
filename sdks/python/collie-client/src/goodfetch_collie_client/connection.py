"""The Connection every Product's Connection type is built on (ADRs 0078 and 0095).

It opens with `hello`, naming the Product the Client needs, its version and
the form `python`, and holds the engine's answer; it starts Runs and closes
them before itself; and once closed, every call fails with `closed`. The
only check of the engine's answers is the protocol version's (STYLE.md).
TypeScript's `client-connection.ts` is its twin.
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import TYPE_CHECKING, Literal

import anyio

from goodfetch_collie_client.attached_engine import EngineExit
from goodfetch_collie_client.framing import object_members
from goodfetch_collie_client.generated.connection import (
    PRODUCT_VERSION,
    PROTOCOL_VERSION,
)
from goodfetch_collie_client.generated.errors import (
    CallCancelledError,
    ClosedError,
    DeadlineExceededError,
    EngineExitedError,
    EngineOutputInvalidError,
    ProtocolVersionUnsupportedError,
)
from goodfetch_collie_client.generated.models import (
    Form,
    HelloParams,
    Product,
    ProductVersion,
    ProtocolVersion,
    RunStartParams,
    Surface,
)
from goodfetch_collie_client.generated.operations import AsyncCollieOperations
from goodfetch_collie_client.once import Once
from goodfetch_collie_client.rpc import Channel, InvalidLine, ReadFailed, WriteFailed
from goodfetch_collie_client.run import AsyncRun

if TYPE_CHECKING:
    from collections.abc import Iterable, Mapping

    from anyio.abc import ByteStream
    from pydantic import BaseModel

    from goodfetch_collie_client.attached_engine import AttachedEngine
    from goodfetch_collie_client.goodfetch_error import ErrorClass, GoodfetchError
    from goodfetch_collie_client.rpc import TransportEnd

CLOSE_TIMEOUT = 30.0
"""How long `close()` waits for an engine to stop, in seconds (ADR 0080).

`timeout.close`'s default.
"""

EXIT_GRACE = 2.0
"""How long an ended transport waits for its engine's exit status, in seconds."""

_CONNECT_AGAIN = "Connect again: a closed Connection never reopens."


@dataclass(frozen=True)
class EngineInfo:
    """The engine a Connection reached, fixed when it opened (G6)."""

    kind: Literal["attached", "daemon", "remote"]
    """How it is reached; an Attached engine is the one kind so far."""
    product: Product | str
    """The engine's Product, a newer one kept.

    `foxhound` serves a Collie Connection too (ADR 0095).
    """
    version: str
    """The engine's Product version, reported, never compared (ADR 0090)."""
    protocol_version: str
    """The protocol version `major.minor` it speaks."""
    engine_path: str | None
    """The executable started, or `None` when none was."""


class ClientConnection:
    """The common Connection under each Product's Connection type."""

    def __init__(
        self,
        transport: ByteStream,
        engine: AttachedEngine | None,
        error_classes: Mapping[str, ErrorClass],
        close_timeout: float,
    ) -> None:
        """Takes the transport, its engine when one started, and the code classes."""
        self._engine = engine
        self._channel = Channel(transport, self, error_classes)
        self._operations = AsyncCollieOperations(self)
        self._runs: dict[AsyncRun, None] = {}
        self._info: EngineInfo | None = None
        self._ended = False
        self._closing = Once()
        self._close_timeout = close_timeout

    @classmethod
    async def open(
        cls,
        transport: ByteStream,
        *,
        needs: Product,
        error_classes: Mapping[str, ErrorClass],
        engine: AttachedEngine | None = None,
        close_timeout: float = CLOSE_TIMEOUT,
    ) -> ClientConnection:
        """Opens a Connection over `transport` and finishes `hello` before it returns.

        A refused Product or protocol version raises here, once the transport
        and its engine are ended; so does a cancelled opening.
        """
        connection = cls(transport, engine, error_classes, close_timeout)
        try:
            await connection._hello(needs)
        except BaseException:
            with anyio.CancelScope(shield=True):
                await connection._end()
            raise
        return connection

    @property
    def engine(self) -> EngineInfo:
        """The engine reached."""
        if self._info is None:
            message = "invariant: an open Connection has finished its hello"
            raise TypeError(message)
        return self._info

    async def call(self, operation: str, params: BaseModel) -> object:
        """Sends one call and returns its result as the engine sent it.

        The engine's answers are not validated beyond `hello`'s protocol
        version (STYLE.md), nor are the params, which the engine checks.
        """
        wire = params.model_dump(
            mode="json", by_alias=True, exclude_unset=True, warnings=False
        )
        return await self._channel.call(operation, wire)

    async def start(
        self, *, run_id: str | None = None, tags: Iterable[str] | None = None
    ) -> AsyncRun:
        """Starts a Run and returns once it is started (ADR 0102)."""
        self.ensure_open("run.start")
        fields: dict[str, object] = {}
        if run_id is not None:
            fields["run_id"] = run_id
        if tags is not None:
            fields["tags"] = list(tags)
        started = await self._operations.run_start(
            RunStartParams.model_construct(None, **fields)
        )
        run = AsyncRun(self, started)
        self._runs[run] = None
        return run

    def reachable(self) -> bool:
        """Whether the Connection can still reach the engine."""
        return not self._ended

    def forget(self, run: AsyncRun) -> None:
        """Forgets a closed Run."""
        self._runs.pop(run, None)

    async def close(self) -> None:
        """Closes the Connection; idempotent, and every caller waits for the one close.

        It closes each Run it opened first, stopping one still running,
        cancels the calls in flight, then ends its Attached engine and waits
        for it to stop, all within the close timeout. Cancelling the caller
        does not cut the close short: the timeout bounds it.

        Raises:
            DeadlineExceededError: The close outlasted its timeout, so the
                engine was killed.
            GoodfetchError: Closing a Run failed; the first failure is raised.
        """
        await self._closing.run(self._close, shield=True)

    async def ended(self, end: TransportEnd) -> GoodfetchError:
        """The error each call still waiting, and every later one, fails with."""
        self._ended = True
        if isinstance(end, InvalidLine):
            if self._engine is not None:
                self._engine.kill()
            return EngineOutputInvalidError(
                "the engine wrote a line that is not one JSON-RPC message, "
                "so the Connection ended",
                details={"line": end.line},
            )
        if self._closing.started:
            return ClosedError(
                "this Connection is closed; connect again", fix=_CONNECT_AGAIN
            )
        exit_ = await self._exit_of_engine()
        how = exit_.signal or (
            None if exit_.exit_code is None else f"exit code {exit_.exit_code}"
        )
        error = EngineExitedError(
            "the engine ended before its Connection was closed"
            + ("" if how is None else f", with {how}"),
            details={
                "exit_code": exit_.exit_code,
                "signal": exit_.signal,
                "stderr_tail": exit_.stderr_tail,
            },
        )
        if isinstance(end, ReadFailed | WriteFailed):
            error.__cause__ = end.cause
        return error

    def ensure_open(self, operation: str) -> None:
        """Raises `closed` once the Connection's close has begun.

        Raises:
            ClosedError: The Connection is closed.
        """
        if self._closing.started:
            raise closed_error(operation)

    async def _hello(self, needs: Product) -> None:
        params = HelloParams(
            protocol_version=ProtocolVersion(PROTOCOL_VERSION),
            surface=Surface.sdk_python,
            needs=needs,
            product_version=ProductVersion(PRODUCT_VERSION),
            form=Form.python,
        )
        answer = await self.call("hello", params)
        fields = object_members(answer) or {}
        spoken = fields.get("protocol_version")
        if not isinstance(spoken, str) or not speaks(spoken):
            raise _unsupported(spoken if isinstance(spoken, str) else None, fields)
        product = fields.get("product")
        version = fields.get("product_version")
        self._info = EngineInfo(
            kind="attached",
            product=_product_of(product),
            version=version if isinstance(version, str) else "",
            protocol_version=spoken,
            engine_path=None if self._engine is None else self._engine.path,
        )

    async def _close(self) -> None:
        """Closes in order within the close timeout; past it, kills the engine."""
        started_at = anyio.current_time()
        failure: Exception | None = None
        with anyio.move_on_after(self._close_timeout) as scope:
            failure = await self._close_in_order()
        if scope.cancelled_caught:
            self._cancel_calls()
            if self._engine is not None:
                self._engine.kill()
            await self._channel.close()
            if self._engine is not None:
                await self._engine.exited()
            await self._channel.abandon()
            limit = round(self._close_timeout * 1000)
            raise DeadlineExceededError(
                f"the Connection did not close within timeout.close ({limit} ms), "
                "so its engine was stopped",
                code="deadline_exceeded",
                group="deadline_exceeded",
                retryable=True,
                fix=(
                    "Raise timeout.close, or find why the engine stopped answering "
                    "in the engine's stderr, which the logger got."
                ),
                details={
                    "limit": {
                        "key": "timeout.close",
                        "value": limit,
                        "layer": "default",
                    },
                    "elapsed": round((anyio.current_time() - started_at) * 1000),
                    "waited_for": "close",
                },
            )
        if failure is not None:
            raise failure

    async def _close_in_order(self) -> Exception | None:
        """Closes each Run, cancels the calls in flight and ends the engine.

        Returns:
            The first Run's failure to close, if any.
        """
        failure: Exception | None = None
        for run in list(self._runs):
            try:
                await run.close()
            except Exception as error:
                # Every Run is closed before the first failure is raised.
                failure = failure or error
        await self._channel.notify_cancelled(self._cancel_calls())
        await self._channel.close()
        # An Attached engine stops at the end of its input.
        if self._engine is not None:
            await self._engine.exited()
        await self._channel.wait_ended()
        await self._channel.abandon()
        return failure

    def _cancel_calls(self) -> list[int]:
        return self._channel.cancel_all(
            CallCancelledError(
                "the call's Connection was closed", details={"by": "client"}
            )
        )

    async def _end(self) -> None:
        """Ends a Connection that never opened: both sides, then the engine."""
        await self._channel.abandon()
        if self._engine is not None:
            await self._engine.stop(self._close_timeout)
        await self._channel.wait_ended()

    async def _exit_of_engine(self) -> EngineExit:
        if self._engine is not None:
            with anyio.move_on_after(EXIT_GRACE):
                return await self._engine.exited()
        return EngineExit(None, None, "")


def closed_error(operation: str) -> ClosedError:
    """The error a call on a closed Connection fails with."""
    message = f"{operation}: this Connection is closed; connect again"
    return ClosedError(message, fix=_CONNECT_AGAIN)


def _product_of(value: object) -> Product | str:
    if isinstance(value, str):
        try:
            return Product(value)
        except ValueError:
            return value
    return ""


def _unsupported(
    spoken: str | None, answer: Mapping[str, object]
) -> ProtocolVersionUnsupportedError:
    own = f"this Client speaks protocol {PROTOCOL_VERSION}"
    use = (
        f"Use the engine and the SDK of one Product version: this SDK"
        f" {PRODUCT_VERSION} speaks protocol {PROTOCOL_VERSION}"
    )
    if spoken is None:
        return ProtocolVersionUnsupportedError(
            f"{own}, and the engine's answer names no protocol version",
            fix=f"{use}.",
            details={
                "client_protocol_version": PROTOCOL_VERSION,
                "engine_protocol_version": None,
            },
        )
    version = answer.get("product_version")
    engine = version if isinstance(version, str) else "of an unknown version"
    return ProtocolVersionUnsupportedError(
        f"{own}, and the engine answered with {spoken}",
        fix=f"{use}, and the engine {engine} speaks {spoken}.",
        details={
            "client_protocol_version": PROTOCOL_VERSION,
            "engine_protocol_version": spoken,
        },
    )


def speaks(version: str) -> bool:
    """Whether an engine of protocol `version` takes this Client (ADR 0030).

    The same line, at the same or a newer minor. During `0.y` previews each
    `0.y` minor is a line of its own, as `goodfetch-protocol`'s rule reads a
    `hello`.
    """
    try:
        major, minor = (int(part) for part in version.split(".")[:2])
        own_major, own_minor = (int(part) for part in PROTOCOL_VERSION.split("."))
    except ValueError:
        return False
    if major == 0:
        return own_major == 0 and minor == own_minor
    return major == own_major and minor >= own_minor
