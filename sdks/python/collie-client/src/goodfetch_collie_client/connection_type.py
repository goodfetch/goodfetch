"""The Connection both Products' Connection types extend (ADRs 0095 and 0099).

A helper a library serving both Products writes takes `AsyncConnection` or
`Connection`, so it accepts either Product's Connection: `AsyncCollie` and
`Collie` extend them, as Foxhound's twins will. Each declares the members
both Products share, and gives the scoped form, which closes the Connection.
TypeScript's `connection.ts` is its twin.
"""

from __future__ import annotations

from abc import ABC, abstractmethod
from typing import TYPE_CHECKING

from goodfetch_collie_client.handle import NotGiven, not_given

if TYPE_CHECKING:
    from collections.abc import Iterable, Mapping
    from datetime import timedelta
    from typing import Self

    from pydantic import JsonValue

    from goodfetch_collie_client.connection import EngineInfo
    from goodfetch_collie_client.limits import Limits
    from goodfetch_collie_client.run import AsyncRun, Run


class AsyncConnection(ABC):
    """A Client's open link to one engine, through which a program reaches its Runs.

    The async API's: `AsyncCollie` extends it.
    """

    @property
    @abstractmethod
    def engine(self) -> EngineInfo:
        """The engine reached, fixed when the Connection opened.

        Read without a round trip.
        """

    @abstractmethod
    async def start(
        self,
        *,
        run_id: str | None = None,
        tags: Iterable[str] | None = None,
        limits: Limits | None = None,
    ) -> AsyncRun:
        """Starts a Run and returns once it is started (ADR 0102).

        A `run_id` naming an interrupted Run resumes it, and one naming a
        finished Run returns it as it ended. Raises `closed` once the
        Connection is closed.
        """

    @abstractmethod
    async def call(
        self,
        operation: str,
        params: Mapping[str, JsonValue] | None = None,
        *,
        timeout: timedelta | NotGiven | None = not_given,
    ) -> JsonValue:
        """Sends `operation`, one acting on no value, as the wire's JSON (ADR 0099).

        The way down to an operation the SDK does not generate. `params`
        are sent and the result answered as on the wire, in snake_case.
        `timeout` is sent as the operation's own; cancelling the task
        cancels the call. It is never retried, and raises `closed` once the
        Connection is closed.
        """

    @abstractmethod
    async def close(self) -> None:
        """Closes the Connection; idempotent.

        Closes every Run it opened first, stopping one still running, cancels
        the calls in flight, and stops an Attached engine, waiting for it.
        Any later call fails with `closed`.
        """

    async def __aenter__(self) -> Self:
        """Holds the Connection for an `async with` block, which closes it."""
        return self

    async def __aexit__(self, *_: object) -> None:
        """Closes the Connection."""
        await self.close()


class Connection(ABC):
    """A Client's open link to one engine, through which a program reaches its Runs.

    The sync API's: `Collie` extends it.
    """

    @property
    @abstractmethod
    def engine(self) -> EngineInfo:
        """The engine reached, fixed when the Connection opened.

        Read without a round trip.
        """

    @abstractmethod
    def start(
        self,
        *,
        run_id: str | None = None,
        tags: Iterable[str] | None = None,
        limits: Limits | None = None,
    ) -> Run:
        """Starts a Run and returns once it is started (ADR 0102).

        A `run_id` naming an interrupted Run resumes it, and one naming a
        finished Run returns it as it ended. Raises `closed` once the
        Connection is closed.
        """

    @abstractmethod
    def call(
        self,
        operation: str,
        params: Mapping[str, JsonValue] | None = None,
        *,
        timeout: timedelta | NotGiven | None = not_given,
    ) -> JsonValue:
        """Sends `operation` as `AsyncConnection.call` does; waits for its result."""

    @abstractmethod
    def close(self) -> None:
        """Closes the Connection; idempotent, from any thread.

        Closes every Run it opened first, stopping one still running, cancels
        the calls in flight, and stops an Attached engine, waiting for it.
        Any later call fails with `closed`.
        """

    def __enter__(self) -> Self:
        """Holds the Connection for a `with` block, which closes it."""
        return self

    def __exit__(self, *_: object) -> None:
        """Closes the Connection."""
        self.close()
