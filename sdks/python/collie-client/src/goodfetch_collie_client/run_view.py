"""A view of a Run another program started (ADR 0102), such as a detached one.

A Connection's `open_run()` opens it by the Run's ID with no round trip. It
takes no writer role, reads `progress()`, `report()`, `trace()` and the
Key-value store's reads (`get`, `get_bytes`, `keys`), and closing it frees only
the view: the Run goes on. TypeScript's `run-view.ts` is its twin.
"""

from __future__ import annotations

import functools
from typing import TYPE_CHECKING

from goodfetch_collie_client._followed import PagedReading
from goodfetch_collie_client.caller import result_of
from goodfetch_collie_client.generated.errors import ClosedError
from goodfetch_collie_client.generated.models import (
    RunId,
    RunParams,
    RunProgress,
    RunProgressParams,
    RunReport,
    RunTraceResult,
)
from goodfetch_collie_client.generated.operations import (
    DEPRECATED_OPERATIONS,
    EXPERIMENTAL_OPERATIONS,
)
from goodfetch_collie_client.goodfetch_error import runtime_message
from goodfetch_collie_client.key_value_store import (
    AsyncKeyValueStore,
    AsyncKeyValueStoreView,
    KeyValueStore,
    KeyValueStoreView,
)
from goodfetch_collie_client.listeners import Order, spawner
from goodfetch_collie_client.markers import warn_entry
from goodfetch_collie_client.portal import LoopStopped
from goodfetch_collie_client.run_copies import AsyncRunTrace, RunTrace, closed_error
from goodfetch_collie_client.sequence import AsyncSequence, Reading, Sequence

if TYPE_CHECKING:
    from collections.abc import Callable, Mapping
    from typing import Self

    from pydantic import BaseModel

    from goodfetch_collie_client.connection import ClientConnection
    from goodfetch_collie_client.portal import PrivateLoop

__all__ = ["AsyncRunView", "RunView"]


class _AsyncViewStoreHost:
    """Routes an `AsyncRunView`'s Key-value store reads through its Connection."""

    def __init__(self, view: AsyncRunView) -> None:
        self._view = view
        self._run_id = RunId(view.id)

    @property
    def id(self) -> RunId:
        return self._run_id

    async def call(self, operation: str, params: BaseModel) -> object:
        self._view._ensure_open(operation)
        return await self._view._connection.call(operation, params)

    def read(
        self,
        operation: str,
        params: Mapping[str, object],
        item_of: Callable[[str, object], str | None],
    ) -> AsyncSequence[str]:
        self._view._ensure_open(operation)
        spawn = spawner()
        reading = Reading(self._view._connection, operation, params, item_of, Order())
        spawn(reading.run)
        return AsyncSequence(reading)


class _SyncViewStoreHost:
    """Routes a sync `RunView`'s Key-value store reads through its private loop."""

    def __init__(self, loop: PrivateLoop, async_host: _AsyncViewStoreHost) -> None:
        self._loop = loop
        self._async_host = async_host

    @property
    def id(self) -> RunId:
        return self._async_host.id

    def call(self, operation: str, params: BaseModel) -> object:
        calling = functools.partial(self._async_host.call, operation, params)
        try:
            return self._loop.call(calling)
        except LoopStopped:
            raise closed_error(operation) from None

    def read(
        self,
        operation: str,
        params: Mapping[str, object],
        item_of: Callable[[str, object], str | None],
    ) -> Sequence[str]:
        async_host = self._async_host

        async def opening() -> AsyncSequence[str]:
            return async_host.read(operation, params, item_of)

        try:
            sequence = self._loop.call(opening)
        except LoopStopped:
            raise closed_error(operation) from None
        return Sequence(self._loop, sequence, lambda: closed_error(operation))


class AsyncRunView:
    """A Run named by its Run ID, opened as a view of the async API."""

    def __init__(self, connection: ClientConnection, run_id: str) -> None:
        """Takes the Connection and the Run ID `open_run()` was given."""
        self._connection = connection
        self._id = run_id
        self._closed = False
        self._store_host = _AsyncViewStoreHost(self)
        self._key_value_store = AsyncKeyValueStoreView(
            AsyncKeyValueStore(self._store_host)
        )

    def _ensure_open(self, operation: str) -> None:
        if self._closed:
            fix = "Open a new RunView with `open_run()` and make the call on it."
            raise ClosedError(
                runtime_message(
                    f"the RunView {self._id} is closed", operation=operation, fix=fix
                ),
                operation=operation,
                fix=fix,
            )

    @property
    def id(self) -> str:
        """The Run's ID, which names it for good. Read without a round trip."""
        return self._id

    @property
    def key_value_store(self) -> AsyncKeyValueStoreView:
        """Read-only view of the Run's Key-value store (ADRs 0102 and 0103)."""
        return self._key_value_store

    def progress(self) -> AsyncSequence[RunProgress]:
        """The Run's progress (`run.progress`, ADR 0102), read with `async for`.

        Reads a Run's progress: what it does and why it waits. Without `after`,
        it answers at once; given `after`, the position of the value the reader
        last read, it waits for a change and answers the latest value, so values
        in between are coalesced. The value whose `last` is true ends the
        sequence: the Run ended, and its `lifecycle` and `stop_reason` say how,
        or no engine works it (`writer: none`) until a start with its Run ID
        resumes it. A Run another engine works is read through that engine, the
        owner the Run index names, and one no engine works from its committed
        state; a read never takes the Run's writer lock. A Run ID the Run index
        does not list fails with `run_not_found`.

        Experimental: outside semver until it is promoted (ADR 0030).
        """
        warn_entry("run.progress", EXPERIMENTAL_OPERATIONS, DEPRECATED_OPERATIONS)
        self._ensure_open("run.progress")
        run_id = RunId(self._id)

        async def read(after: int | None) -> RunProgress:
            self._ensure_open("run.progress")
            params = RunProgressParams(run_id=run_id)
            if after is not None:
                params.after = after
            return result_of(
                RunProgress, await self._connection.call("run.progress", params)
            )

        reading = PagedReading(
            read,
            items=lambda value: (value,),
            position=lambda value: value.position,
            last=lambda value: value.last,
            credits=1,
            regrant=1,
        )
        spawner()(reading.run)
        return AsyncSequence(reading)

    async def report(self) -> RunReport:
        """Reads the Run's report (`run.report`, ADR 0102).

        Reads a Run's report: at once for a Run that ended, and at its end for
        a live one, which the read never ends. A Run another engine works is
        read through that engine, and one no engine works from its committed
        state, never taking its writer lock. A Run that has no live writer and
        has not ended fails with `run_interrupted`, whose `fix` names a start
        with its Run ID, and a Run ID the Run index does not list with
        `run_not_found`.

        Experimental: outside semver until it is promoted (ADR 0030).
        """
        warn_entry("run.report", EXPERIMENTAL_OPERATIONS, DEPRECATED_OPERATIONS)
        self._ensure_open("run.report")
        raw = await self._connection.call(
            "run.report", RunParams(run_id=RunId(self._id))
        )
        return result_of(RunReport, raw)

    async def trace(self) -> AsyncRunTrace | None:
        """Reads the Run's trace copy (`run.trace`, ADRs 0102 and 0116).

        Reads a Run's trace copy as recorded so far, or null when the Run keeps
        no trace (ADRs 0102 and 0116). Returns only once the copy is whole,
        with its size and SHA-256 digest; a write the engine cannot make fails
        with `file_write_failed`, side `engine`, and leaves no unfinished file.
        Takes no writer lock, so a view reads a Run no program holds without
        resuming it. A Run ID the Run index does not list fails with
        `run_not_found`.

        Experimental: outside semver until it is promoted (ADR 0030).
        """
        warn_entry("run.trace", EXPERIMENTAL_OPERATIONS, DEPRECATED_OPERATIONS)
        self._ensure_open("run.trace")
        raw = await self._connection.call(
            "run.trace", RunParams(run_id=RunId(self._id))
        )
        descriptor = result_of(RunTraceResult, raw).trace
        if descriptor is None:
            return None
        return AsyncRunTrace(self._connection, descriptor)

    async def close(self) -> None:
        """Closes the view; idempotent. Frees only the view: the Run goes on."""
        self._closed = True

    async def __aenter__(self) -> Self:
        """Holds the view for an `async with` block, which closes it."""
        return self

    async def __aexit__(self, *_: object) -> None:
        """Closes the view."""
        await self.close()


class RunView:
    """A Run named by its Run ID, opened as a view of the sync API."""

    def __init__(self, loop: PrivateLoop, view: AsyncRunView) -> None:
        """Takes the Connection's private loop and the view it opened there."""
        self._loop = loop
        self._view = view
        self._key_value_store = KeyValueStoreView(
            KeyValueStore(_SyncViewStoreHost(loop, view._store_host))
        )

    @property
    def id(self) -> str:
        """The Run's ID, which names it for good. Read without a round trip."""
        return self._view.id

    @property
    def key_value_store(self) -> KeyValueStoreView:
        """Read-only view of the Run's Key-value store (ADRs 0102 and 0103)."""
        return self._key_value_store

    def progress(self) -> Sequence[RunProgress]:
        """The Run's progress, read with `for`, as `AsyncRunView.progress` says.

        Reads a Run's progress: what it does and why it waits. Without `after`,
        it answers at once; given `after`, the position of the value the reader
        last read, it waits for a change and answers the latest value, so values
        in between are coalesced. The value whose `last` is true ends the
        sequence: the Run ended, and its `lifecycle` and `stop_reason` say how,
        or no engine works it (`writer: none`) until a start with its Run ID
        resumes it. A Run another engine works is read through that engine, the
        owner the Run index names, and one no engine works from its committed
        state; a read never takes the Run's writer lock. A Run ID the Run index
        does not list fails with `run_not_found`.

        Experimental: outside semver until it is promoted (ADR 0030).
        """
        view = self._view

        async def reading() -> AsyncSequence[RunProgress]:
            return view.progress()

        try:
            sequence = self._loop.call(reading)
        except LoopStopped:
            raise closed_error("run.progress") from None
        return Sequence(self._loop, sequence, lambda: closed_error("run.progress"))

    def report(self) -> RunReport:
        """Reads the Run's report, as `AsyncRunView.report` says.

        Reads a Run's report: at once for a Run that ended, and at its end for
        a live one, which the read never ends. A Run another engine works is
        read through that engine, and one no engine works from its committed
        state, never taking its writer lock. A Run that has no live writer and
        has not ended fails with `run_interrupted`, whose `fix` names a start
        with its Run ID, and a Run ID the Run index does not list with
        `run_not_found`.

        Experimental: outside semver until it is promoted (ADR 0030).
        """
        try:
            return self._loop.call(self._view.report)
        except LoopStopped:
            raise closed_error("run.report") from None

    def trace(self) -> RunTrace | None:
        """Reads the Run's trace copy, as `AsyncRunView.trace` says.

        Reads a Run's trace copy as recorded so far, or null when the Run keeps
        no trace (ADRs 0102 and 0116). Returns only once the copy is whole,
        with its size and SHA-256 digest; a write the engine cannot make fails
        with `file_write_failed`, side `engine`, and leaves no unfinished file.
        Takes no writer lock, so a view reads a Run no program holds without
        resuming it. A Run ID the Run index does not list fails with
        `run_not_found`.

        Experimental: outside semver until it is promoted (ADR 0030).
        """
        try:
            trace = self._loop.call(self._view.trace)
        except LoopStopped:
            raise closed_error("run.trace") from None
        return None if trace is None else RunTrace(self._loop, trace)

    def close(self) -> None:
        """Closes the view; idempotent. Frees only the view: the Run goes on."""
        try:
            self._loop.call(self._view.close)
        except LoopStopped:
            return

    def __enter__(self) -> Self:
        """Holds the view for a `with` block, which closes it."""
        return self

    def __exit__(self, *_: object) -> None:
        """Closes the view."""
        self.close()
