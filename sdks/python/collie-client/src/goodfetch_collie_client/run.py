"""A Run, always opened by its program (ADRs 0078 and 0102).

`finish()` ends it and answers its Run report. Its ID and how it started are
read without a call, and `close()` frees the value and, while the Run is
still running, stops it as an abort by its Client, resumable by its Run ID.
Any other call on a closed Run fails with `closed`. Hand-written until the
handles are generated (issue 1233); TypeScript's `run.ts` is its twin.
"""

from __future__ import annotations

from typing import TYPE_CHECKING, Literal, Protocol

from goodfetch_collie_client.framing import object_members
from goodfetch_collie_client.generated.errors import ClosedError
from goodfetch_collie_client.generated.models import (
    RunAbortParams,
    RunId,
    RunWaitParams,
)
from goodfetch_collie_client.generated.operations import AsyncCollieOperations
from goodfetch_collie_client.once import Once
from goodfetch_collie_client.portal import LoopStopped

if TYPE_CHECKING:
    from typing import Self

    from pydantic import BaseModel

    from goodfetch_collie_client.generated.models import (
        RunReport,
        RunStarted,
        StartedAs,
    )
    from goodfetch_collie_client.portal import PrivateLoop


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


class _RunCalls:
    """The Run's own calls: once the Run is closed, each fails `closed` at once."""

    def __init__(self, host: RunHost, run: AsyncRun) -> None:
        self._host = host
        self._run = run

    async def call(self, operation: str, params: BaseModel) -> object:
        self._run.ensure_open(operation)
        result = await self._host.call(operation, params)
        # A Run report says when the Run ended.
        if self._run.state == "running" and _says_ended(result):
            self._run.state = "ended"
        return result


class AsyncRun:
    """A Run of the async API."""

    def __init__(self, host: RunHost, started: RunStarted) -> None:
        """Takes its Connection and the engine's answer to its start."""
        self._host = host
        self._started = started
        self._operations = AsyncCollieOperations(_RunCalls(host, self))
        self.state: Literal["running", "ended", "closed"] = "running"
        """Where it stands for its program."""
        self._closing = Once()

    @property
    def id(self) -> str:
        """Its Run ID."""
        return self._started.run_id.root

    @property
    def started_as(self) -> StartedAs:
        """Whether its start began it, found it or resumed it."""
        return self._started.started_as

    async def finish(self) -> RunReport:
        """Lets the Run finish and answers its Run report."""
        return await self._operations.run_finish(RunWaitParams(run_id=RunId(self.id)))

    async def close(self) -> None:
        """Closes the Run; idempotent.

        A Run still running is stopped, resumable by its Run ID.
        """
        await self._closing.run(self._close)

    async def _close(self) -> None:
        was_running = self.state == "running"
        self.state = "closed"
        try:
            if was_running and self._host.reachable():
                await self._host.call(
                    "run.abort", RunAbortParams(run_id=RunId(self.id))
                )
        finally:
            self._host.forget(self)

    async def __aenter__(self) -> Self:
        """Holds the Run for an `async with` block, which closes it."""
        return self

    async def __aexit__(self, *_: object) -> None:
        """Closes the Run."""
        await self.close()

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


def _says_ended(result: object) -> bool:
    """Whether `result` is an answer that says its Run is no longer running."""
    members = object_members(result)
    if members is None:
        return False
    lifecycle = members.get("lifecycle")
    return lifecycle is not None and lifecycle != "running"


class Run:
    """A Run of the sync API, which any number of threads may share."""

    def __init__(self, loop: PrivateLoop, run: AsyncRun) -> None:
        """Takes its Connection's private loop and the Run it drives there."""
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

    def finish(self) -> RunReport:
        """Lets the Run finish and answers its Run report."""
        try:
            return self._loop.call(self._run.finish)
        except LoopStopped:
            # Its Connection closed it before stopping the loop.
            raise self._run.closed_error("run.finish") from None

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
