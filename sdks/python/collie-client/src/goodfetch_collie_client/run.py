"""A Run, always opened by its program (ADRs 0078 and 0102).

Its members are the catalog's, generated (ADR 0099): `finish()` ends it and
answers its Run report. Hand-written here is what no catalog entry gives:
its ID and how it started, read without a call, and `close()`, which frees
the value and, while the Run is still running, stops it as an abort by its
Client, resumable by its Run ID. Any other call on a closed Run, or on a
value it opened, fails with `closed`. TypeScript's `run.ts` is its twin.
"""

from __future__ import annotations

import functools
import weakref
from typing import TYPE_CHECKING, Generic, Literal, Protocol, TypeVar

import anyio

from goodfetch_collie_client.framing import object_members
from goodfetch_collie_client.generated.errors import ClosedError
from goodfetch_collie_client.generated.handles import AsyncRun as AsyncRunMembers
from goodfetch_collie_client.generated.handles import Run as RunMembers
from goodfetch_collie_client.generated.models import RunAbortParams, RunId
from goodfetch_collie_client.once import Once
from goodfetch_collie_client.portal import LoopStopped

if TYPE_CHECKING:
    from typing import Self

    from pydantic import BaseModel

    from goodfetch_collie_client.generated.models import RunStarted, StartedAs
    from goodfetch_collie_client.handle import AsyncHandleHost, HandleHost
    from goodfetch_collie_client.portal import PrivateLoop

_Id = TypeVar("_Id")
_ChildId = TypeVar("_ChildId")


class RunHost(Protocol):
    """What a Run reaches its engine through: its Connection."""

    async def call(self, operation: str, params: BaseModel) -> object:
        """Sends one call and returns its result as the engine sent it."""
        ...

    def reachable(self) -> bool:
        """Whether the Connection can still reach the engine."""
        ...

    def forget(self, run: AsyncRun) -> None:
        """Forgets a closed Run."""
        ...


class _RunScope:
    """What a Run and the calls made through it share: its ID and where it stands."""

    def __init__(self, run_id: str) -> None:
        self.id = run_id
        self.state: Literal["running", "ended", "closed"] = "running"

    def ensure_open(self, operation: str) -> None:
        """Raises `closed` when the Run is closed.

        Raises:
            ClosedError: The Run is closed.
        """
        if self.state == "closed":
            raise self.closed_error(operation)

    def closed_error(self, operation: str) -> ClosedError:
        """The error a call on the closed Run fails with."""
        message = (
            f"{operation}: the Run {self.id} is closed; start it again by its Run ID"
        )
        return ClosedError(
            message, fix="Start the Run again by its Run ID to reach it."
        )


class _ValueScope:
    """A value a program holds under a Run: a Session, a page, or a Locator."""

    def __init__(self, parent: _ValueScope | None) -> None:
        self.parent = parent
        """The value that opened or built it, when the Run did not."""
        self.closed = False
        """Whether its program closed it."""
        self.closing: anyio.Event | None = None
        """Set once its `close` in flight ends."""

    def is_closed(self) -> bool:
        """Whether it, or a value that opened or built it, is closed."""
        held: _ValueScope | None = self
        while held is not None:
            if held.closed:
                return True
            held = held.parent
        return False


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

    async def call(self, operation: str, params: BaseModel) -> object:
        """Sends one call, unless the Run or the value is closed."""
        self.run.ensure_open(operation)
        if self._value is not None and self._value.is_closed():
            message = (
                f"{operation}: the value it is called on is closed,"
                " or the one that opened it is"
            )
            raise ClosedError(
                message, fix="Open another from its Run: a closed value never reopens."
            )
        result = await self._connection.call(operation, params)
        if self._value is None and self.run.state == "running" and _says_ended(result):
            self.run.state = "ended"
        return result

    def child(self, id: _ChildId) -> AsyncHandleHost[_ChildId]:
        """The host of a handle this one opens or builds, whose calls carry `id`."""
        return self.child_host(id)

    def child_host(self, id: _ChildId) -> _AsyncHost[_ChildId]:
        """`child`, as this class: what the sync twin's `child` wraps."""
        return _AsyncHost(self._connection, self.run, _ValueScope(self._value), id)

    async def close(self, operation: str, params: BaseModel) -> None:
        """Closes the value through `operation`, its own `close`; idempotent.

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
        while True:
            # What its Run or the value above it closed is closed with it,
            # and the engine holds it no more: nothing is left to send.
            if self.run.state == "closed" or value.is_closed():
                value.closed = True
                return
            if value.closing is not None:
                await value.closing.wait()
                continue
            # A close the engine refuses, or that was never sent, closed
            # nothing: the next one sends again.
            closing = anyio.Event()
            value.closing = closing
            try:
                await self._connection.call(operation, params)
                value.closed = True
            finally:
                value.closing = None
                closing.set()


class _SyncHost(Generic[_Id]):
    """What a sync handle reaches its engine through: its async twin's host."""

    def __init__(self, loop: PrivateLoop, host: _AsyncHost[_Id]) -> None:
        self._loop = loop
        self._host = host

    @property
    def id(self) -> _Id:
        """The ID each of the handle's calls carries."""
        return self._host.id

    def call(self, operation: str, params: BaseModel) -> object:
        """Sends one call on the private loop and waits for its result."""
        try:
            return self._loop.call(
                functools.partial(self._host.call, operation, params)
            )
        except LoopStopped:
            # Its Connection closed the Run before stopping the loop.
            raise self._host.run.closed_error(operation) from None

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


_hosts: weakref.WeakKeyDictionary[AsyncRun, _AsyncHost[RunId]] = (
    weakref.WeakKeyDictionary()
)
"""Each async Run's host, which its sync twin's host wraps."""


class AsyncRun(AsyncRunMembers):
    """A Run of the async API."""

    def __init__(self, connection: RunHost, started: RunStarted) -> None:
        """Takes its Connection and the engine's answer to its start."""
        scope = _RunScope(started.run_id.root)
        host = _AsyncHost(connection, scope, None, started.run_id)
        super().__init__(host)
        _hosts[self] = host
        self._connection = connection
        self._scope = scope
        self._started_as = started.started_as
        self._closing = Once()

    @property
    def id(self) -> str:
        """Its Run ID."""
        return self._scope.id

    @property
    def started_as(self) -> StartedAs:
        """Whether its start began it, found it or resumed it."""
        return self._started_as

    async def close(self) -> None:
        """Closes the Run; idempotent.

        A Run still running is stopped, resumable by its Run ID.
        """
        await self._closing.run(self._close)

    async def _close(self) -> None:
        was_running = self._scope.state == "running"
        self._scope.state = "closed"
        try:
            if was_running and self._connection.reachable():
                await self._connection.call(
                    "run.abort", RunAbortParams(run_id=RunId(self.id))
                )
        finally:
            self._connection.forget(self)

    async def __aenter__(self) -> Self:
        """Holds the Run for an `async with` block, which closes it."""
        return self

    async def __aexit__(self, *_: object) -> None:
        """Closes the Run."""
        await self.close()


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
        super().__init__(_SyncHost(loop, _hosts[run]))
        self._loop = loop
        self._run = run

    @property
    def id(self) -> str:
        """Its Run ID."""
        return self._run.id

    @property
    def started_as(self) -> StartedAs:
        """Whether its start began it, found it or resumed it."""
        return self._run.started_as

    def close(self) -> None:
        """Closes the Run; idempotent.

        A Run still running is stopped, resumable by its Run ID.
        """
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
