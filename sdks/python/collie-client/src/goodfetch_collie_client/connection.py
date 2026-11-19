"""The Connection every Product's Connection type is built on (ADRs 0078 and 0095).

It opens with `hello`, naming the Product the Client needs, its version and
the form `python`, and holds the engine's answer; then it sets the
Connection's Limits, each a `usage.set` on scope `connection` (ADR 0080),
and a refused one fails the opening. It starts Runs and closes
them before itself; and once closed, every call fails with `closed`. The
only check of the engine's answers is the protocol version's (STYLE.md).
TypeScript's `client-connection.ts` is its twin.
"""

from __future__ import annotations

import logging
from dataclasses import dataclass, field
from typing import TYPE_CHECKING, Literal, cast

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
    RunOrigin,
    RunStartParams,
    SetLogLevelParams,
    SettingLimit,
    SettingsLayer,
    StartedAs,
    Surface,
    UsageSetParams,
)
from goodfetch_collie_client.generated.operations import (
    DEPRECATED_OPERATIONS,
    EXPERIMENTAL_OPERATIONS,
    AsyncCollieOperations,
)
from goodfetch_collie_client.goodfetch_error import GoodfetchError
from goodfetch_collie_client.handle import NotGiven, not_given
from goodfetch_collie_client.host_logging import (
    CLIENT_LOGGER,
    RUN_LOGGER,
    log_engine_event,
    log_level,
)
from goodfetch_collie_client.limits import Limits, limit_list
from goodfetch_collie_client.markers import warn_entry
from goodfetch_collie_client.once import Once
from goodfetch_collie_client.rpc import Channel, InvalidLine, ReadFailed, WriteFailed
from goodfetch_collie_client.run import AsyncRun
from goodfetch_collie_client.settings import settings_on_wire
from goodfetch_collie_client.shared_subscription import SharedSubscription

if TYPE_CHECKING:
    from collections.abc import Callable, Iterable, Mapping, Sequence

    from anyio.abc import ByteStream
    from pydantic import BaseModel, JsonValue

    from goodfetch_collie_client.attached_engine import AttachedEngine
    from goodfetch_collie_client.generated.models import RunLimit, RunStarted
    from goodfetch_collie_client.generated.settings import RunSettings
    from goodfetch_collie_client.goodfetch_error import ErrorClass
    from goodfetch_collie_client.rpc import TransportEnd

CONNECT_TIMEOUT = 30_000
"""How long opening waits for `hello`'s answer, in milliseconds (ADR 0083).

`timeout.connect`'s default, when `connect()`'s `timeout` is left out.
"""

CLOSE_TIMEOUT = 30.0
"""How long `close()` waits for an engine to stop, in seconds (ADR 0080).

`timeout.close`'s default.
"""

EXIT_GRACE = 2.0
"""How long an ended transport waits for its engine's exit status, in seconds."""

_CONNECT_AGAIN = "Connect again: a closed Connection never reopens."

LOG = "$/log"
"""The notification that carries one of the engine's log events."""


EngineKind = Literal["attached", "daemon", "remote"]
"""How an engine is reached.

An Attached engine the Connection started, the Daemon of this machine, or a
remote Daemon.
"""


@dataclass(frozen=True)
class EngineFallback:
    """Why `auto` ran another engine than the Daemon it found, and what to do."""

    cause: str
    """Why it did."""
    fix: str
    """What to do about it."""


@dataclass(frozen=True)
class EngineInfo:
    """The engine a Connection reached, fixed when it opened (G6).

    It never holds a Daemon's token.
    """

    kind: EngineKind
    """How it is reached."""
    product: Product | str
    """The engine's Product, a newer one kept.

    `foxhound` serves a Collie Connection too (ADR 0095).
    """
    version: str
    """The engine's Product version, reported, never compared (ADR 0090)."""
    protocol_version: str
    """The protocol version `major.minor` it speaks."""
    project_file: str | None
    """The project file the engine read, or `None` when it read none."""
    home: str | None
    """The Goodfetch home the engine runs in.

    `None` for a remote Daemon, or when none is known.
    """
    engine_path: str | None
    """The executable started, or `None` when none was."""
    url: str | None
    """Where a Daemon was reached.

    A remote one's URL, or this machine's socket path or loopback address;
    `None` for an Attached engine.
    """
    fallback: EngineFallback | None
    """Why `auto` ran another engine than the Daemon it found, and what to do.

    `None` when it ran the one it chose.
    """


@dataclass(frozen=True)
class Reached:
    """How a Connection's engine was reached, which `engine` reports."""

    kind: EngineKind
    """How it is reached."""
    home: str | None
    """The Goodfetch home it runs in, or `None` when none is known."""
    url: str | None
    """Where a Daemon was reached, or `None` for an Attached engine."""


ATTACHED_ELSEWHERE = Reached("attached", None, None)
"""An Attached engine with no home known, as over a transport a test gives."""


@dataclass(frozen=True)
class DaemonHello:
    """What a Daemon's `hello` takes beyond an Attached engine's."""

    token: str = field(repr=False)
    """The Daemon's token, which `hello` carries and nothing else holds."""
    closed_at_hello: Callable[[], GoodfetchError]
    """What fails the opening when the Daemon closes before it answers."""


class ClientConnection:
    """The common Connection under each Product's Connection type."""

    def __init__(
        self,
        transport: ByteStream,
        engine: AttachedEngine | None,
        error_classes: Mapping[str, ErrorClass],
        close_timeout: float,
        reached: Reached = ATTACHED_ELSEWHERE,
    ) -> None:
        """Takes the transport, its engine when one started, and the code classes.

        `reached` says how the engine was reached, which `engine` reports.
        """
        self._engine = engine
        self._reached = reached
        self._channel = Channel(transport, self, error_classes)
        self._operations = AsyncCollieOperations(self)
        self._runs: dict[AsyncRun, None] = {}
        self._published = SharedSubscription(self)
        self._info: EngineInfo | None = None
        self._ended = False
        self._closing = Once()
        self._close_timeout = close_timeout
        self._log_level = log_level()
        """The level of the engine's logs the engine was last told.

        By `hello` or `connection.set_log_level`.
        """

    @classmethod
    async def open(
        cls,
        transport: ByteStream,
        *,
        needs: Product,
        error_classes: Mapping[str, ErrorClass],
        engine: AttachedEngine | None = None,
        reached: Reached = ATTACHED_ELSEWHERE,
        daemon: DaemonHello | None = None,
        close_timeout: float = CLOSE_TIMEOUT,
        limits: Sequence[RunLimit] = (),
        connect_timeout: int | NotGiven | None = not_given,
    ) -> ClientConnection:
        """Opens a Connection over `transport`, finishing `hello` before it returns.

        Then each of `limits`, the wire's list of the Connection's Limits,
        is set on it. `reached` says how the engine was reached, and
        `daemon` gives a Daemon's token, which `hello` carries.
        `connect_timeout`, already checked by `timeout_of`, is how many
        milliseconds `hello` waits for its answer: `timeout.connect`'s 30 s
        when left out, none for `None`. A refused Product, protocol version
        or Limit raises here, once the transport and its engine are ended;
        so does a cancelled opening, and a `hello` unanswered within
        `connect_timeout`, with `DeadlineExceededError`. A Daemon that
        closes the Connection before it answers `hello`, as one does for a
        token it does not take, fails it with `daemon`'s `closed_at_hello`.
        """
        connection = cls(transport, engine, error_classes, close_timeout, reached)
        try:
            await connection._hello(needs, daemon, connect_timeout)
            for limit in limits:
                await connection._set_limit(limit)
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

    async def raw(self, operation: str, params: Mapping[str, object]) -> JsonValue:
        """Sends the raw call `operation` (ADR 0099), unless the Connection is closed.

        Its params and its result are the wire's JSON, unconverted.
        """
        self.ensure_open(operation)
        result = await self._channel.call(operation, params)
        # Engine answers are not validated beyond `hello`'s protocol version
        # (STYLE.md): a result is the JSON the channel decoded.
        return cast("JsonValue", result)

    async def stream(
        self,
        method: str,
        params: Mapping[str, object],
        notified: Callable[[str, object], None],
        *,
        sent: Callable[[int], None] | None = None,
    ) -> object:
        """Sends `method`, answered once the engine streamed what the Client granted.

        `notified` hears each notification naming the call's id, such as
        `$/chunk`, on the reading task, so it must not block; `sent` is told
        that id once the call is written, for the credits `notify` grants.
        """
        self.ensure_open(method)
        return await self._channel.stream(method, params, notified, sent=sent)

    async def notify(self, method: str, params: Mapping[str, object]) -> None:
        """Sends the engine the notification `method`, such as `$/credit`."""
        await self._channel.notify(method, params)

    async def start(
        self,
        *,
        run_id: str | None = None,
        tags: Iterable[str] | None = None,
        limits: Limits | None = None,
        settings: RunSettings | None = None,
    ) -> AsyncRun:
        """Leaving the scope or `close()` stops the Run with a Checkpoint.

        `finish()` ends it. Starts a Run and returns once it is started
        (ADR 0102). A start that resumed its Run is logged at info, and one
        that found it ended at warning, on the `goodfetch.client.run` logger.
        A `limits` amount the SDK cannot turn exactly fails with
        `setting_invalid` before anything is sent. When the effective level
        of `goodfetch.engine` changed since the engine was last told, the
        start tells it first with `connection.set_log_level`.
        """
        warn_entry("run.start", EXPERIMENTAL_OPERATIONS, DEPRECATED_OPERATIONS)
        self.ensure_open("run.start")
        fields: dict[str, object] = {}
        if run_id is not None:
            fields["run_id"] = run_id
        if tags is not None:
            fields["tags"] = list(tags)
        run_limits = limit_list(limits)
        if run_limits:
            fields["limits"] = run_limits
        if settings is not None:
            fields["settings"] = settings_on_wire(settings)
        await self._follow_log_level()
        self.ensure_open("run.start")
        started = await self._operations.run_start(
            RunStartParams.model_construct(None, **fields)
        )
        _log_start(started)
        run = AsyncRun(self, started)
        self._runs[run] = None
        return run

    def notified(self, method: str, params: object) -> None:
        """Logs each of the engine's `$/log` events; drops any other notification."""
        if method == LOG:
            log_engine_event(params)

    def reachable(self) -> bool:
        """Whether the Connection can still reach the engine."""
        return not self._ended

    def forget(self, run: AsyncRun) -> None:
        """Forgets a closed Run."""
        self._runs.pop(run, None)

    @property
    def published(self) -> SharedSubscription:
        """Its own Subscription, which the listeners of published events share."""
        return self._published

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

    async def _hello(
        self,
        needs: Product,
        daemon: DaemonHello | None,
        connect_timeout: int | NotGiven | None,
    ) -> None:
        params = HelloParams(
            protocol_version=ProtocolVersion(PROTOCOL_VERSION),
            surface=Surface.sdk_python,
            needs=needs,
            product_version=ProductVersion(PRODUCT_VERSION),
            form=Form.python,
            log_level=self._log_level,
        )
        if daemon is not None:
            # Set only for a Daemon, so an Attached engine's hello holds no token.
            params.token = daemon.token
        try:
            answer = await self._hello_answer(params, connect_timeout)
        except EngineExitedError as error:
            if daemon is None:
                raise
            raise daemon.closed_at_hello() from error
        fields = object_members(answer) or {}
        spoken = fields.get("protocol_version")
        if not isinstance(spoken, str) or not speaks(spoken):
            raise _unsupported(spoken if isinstance(spoken, str) else None, fields)
        product = fields.get("product")
        version = fields.get("product_version")
        self._info = EngineInfo(
            kind=self._reached.kind,
            product=_product_of(product),
            version=version if isinstance(version, str) else "",
            protocol_version=spoken,
            project_file=None,
            home=self._reached.home,
            engine_path=None if self._engine is None else self._engine.path,
            url=self._reached.url,
            fallback=None,
        )

    async def _hello_answer(
        self, params: HelloParams, connect_timeout: int | NotGiven | None
    ) -> object:
        """`hello`'s answer, once it came within `connect_timeout`.

        Past it, raises `DeadlineExceededError`, its `limit` naming
        `timeout.connect` at the run layer, given as an argument, or at its
        default, and `waited_for` `hello` (ADR 0083).
        """
        if connect_timeout is None:
            return await self.call("hello", params)
        if isinstance(connect_timeout, NotGiven):
            value = CONNECT_TIMEOUT
            limit = SettingLimit(
                key="timeout.connect", value=value, layer=SettingsLayer.default
            )
        else:
            value = connect_timeout
            limit = SettingLimit(
                key="timeout.connect",
                value=value,
                layer=SettingsLayer.run,
                origin=RunOrigin.argument,
            )
        started_at = anyio.current_time()
        with anyio.move_on_after(value / 1000):
            return await self.call("hello", params)
        raise _phase_exceeded(
            f"timeout.connect ({value} ms) passed before the engine answered hello",
            fix=(
                "Raise connect()'s timeout, or find why the engine does not answer "
                "in the engine's stderr, which the logger got."
            ),
            limit=limit,
            started_at=started_at,
            waited_for="hello",
        )

    async def _follow_log_level(self) -> None:
        """Tells the engine the level `goodfetch.engine` records, if it changed.

        With `connection.set_log_level`, once the engine answered; nothing
        is sent when it did not change, and nothing polls it. The engine's
        logs are telemetry, so a refusal fails no start: it goes to the
        `goodfetch.client` logger at warning, and the level the engine kept
        stays the one told, so the next Run's start tells it again, as after
        a cancelled call. A Connection that ended or is closing fails with
        its own error.
        """
        level = log_level()
        before = self._log_level
        if level == before:
            return
        self._log_level = level
        try:
            await self._operations.connection_set_log_level(
                SetLogLevelParams(log_level=level)
            )
        except BaseException as error:
            # Refused, failed or cancelled: the engine may keep the level it
            # had, so the next start tells it again, unless a start begun
            # meanwhile told it a newer one.
            if self._log_level == level:
                self._log_level = before
            if (
                not isinstance(error, GoodfetchError)
                or self._ended
                or self._closing.started
            ):
                raise
            logging.getLogger(CLIENT_LOGGER).warning(
                "The engine refused the log level `%s`; it keeps sending its logs "
                "at `%s`.",
                level,
                before,
                exc_info=error,
                extra={"log_level": str(level)},
            )

    async def _set_limit(self, limit: RunLimit) -> None:
        """Sets `limit` on the Connection: no `scope_id`, no period (ADR 0080)."""
        await self._operations.usage_set(
            UsageSetParams(scope="connection", **limit.model_dump(exclude_unset=True))
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
            raise _phase_exceeded(
                f"the Connection did not close within timeout.close ({limit} ms), "
                "so its engine was stopped",
                fix=(
                    "Raise timeout.close, or find why the engine stopped answering "
                    "in the engine's stderr, which the logger got."
                ),
                limit=SettingLimit(
                    key="timeout.close", value=limit, layer=SettingsLayer.default
                ),
                started_at=started_at,
                waited_for="close",
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


def _log_start(started: RunStarted) -> None:
    """Tells the host how a start with an existing Run ID reached its Run.

    Resumed at info, ended at warning, since the Run then takes no new work
    (ADR 0102). A new Run is not logged.
    """
    run_id = started.run_id.root
    # Engine answers are not validated (STYLE.md): `started_as` may be a bare string.
    fields = {"run_id": run_id, "started_as": str(started.started_as)}
    logger = logging.getLogger(RUN_LOGGER)
    match started.started_as:
        case StartedAs.new:
            return
        case StartedAs.resumed:
            logger.info(
                "The start resumed the Run `%s` from its committed state.",
                run_id,
                extra=fields,
            )
        case StartedAs.ended:
            logger.warning(
                "The start found the Run `%s` ended: it is returned as it ended "
                "and takes no new work.",
                run_id,
                extra=fields,
            )


def _phase_exceeded(
    message: str,
    *,
    fix: str,
    limit: SettingLimit,
    started_at: float,
    waited_for: Literal["hello", "close"],
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
