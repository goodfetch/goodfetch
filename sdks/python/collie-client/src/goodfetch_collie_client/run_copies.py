"""A Run's Checkpoint and RunTrace copies (ADRs 0102 and 0116).

`AsyncRun.checkpoint()` answers an `AsyncCheckpoint` (sync `Checkpoint`), and
`AsyncRun.trace()` / `AsyncRunView.trace()` answers an `AsyncRunTrace | None`
(sync `RunTrace | None`), only once the engine's file is whole. `save_as(path)`
writes the file on the program's machine whole or not at all through a hidden
temporary file, and `stream()` yields its bytes in order.
TypeScript's `run-copies.ts` is its twin.
"""

from __future__ import annotations

import base64
import functools
from collections.abc import Mapping
from typing import TYPE_CHECKING, cast, final

from goodfetch_collie_client._file_saving import save_file
from goodfetch_collie_client.caller import result_of
from goodfetch_collie_client.generated.models import (
    Checkpoint as WireCheckpoint,
)
from goodfetch_collie_client.generated.models import (
    CheckpointSaved,
    RunId,
    TraceSaved,
)
from goodfetch_collie_client.generated.models import (
    RunTrace as WireRunTrace,
)
from goodfetch_collie_client.generated.operations import (
    DEPRECATED_OPERATIONS,
    EXPERIMENTAL_OPERATIONS,
)
from goodfetch_collie_client.listeners import Order, spawner
from goodfetch_collie_client.markers import warn_entry
from goodfetch_collie_client.portal import LoopStopped
from goodfetch_collie_client.sequence import AsyncSequence, Reading, Sequence

if TYPE_CHECKING:
    import os
    from datetime import datetime

    from goodfetch_collie_client.connection import ClientConnection
    from goodfetch_collie_client.generated.errors import ClosedError
    from goodfetch_collie_client.portal import PrivateLoop

__all__ = ["AsyncCheckpoint", "AsyncRunTrace", "Checkpoint", "RunTrace"]


def closed_error(operation: str) -> ClosedError:
    """Deferred import of `connection.closed_error` to avoid a module cycle."""
    from goodfetch_collie_client.connection import closed_error as _closed_error

    return _closed_error(operation)


CHUNK = "$/chunk"
"""The notification each byte chunk `checkpoint.stream` or `trace.stream` sends."""


def _chunk_of(method: str, params: object) -> bytes | None:
    """One `$/chunk` notification's bytes, or None for any other notification."""
    if method != CHUNK or not isinstance(params, Mapping):
        return None
    members = cast("Mapping[str, object]", params)
    data = members.get("data")
    return base64.b64decode(data) if isinstance(data, str) else None


def _stream_chunks(
    connection: ClientConnection,
    operation: str,
    params: Mapping[str, object],
) -> AsyncSequence[bytes]:
    """Opens a credit-controlled `$/chunk` byte stream for `operation`."""
    connection.ensure_open(operation)
    spawn = spawner()
    reading = Reading(connection, operation, params, _chunk_of, Order())
    spawn(reading.run)
    return AsyncSequence(reading)


@final
class AsyncCheckpoint:
    """A completed Checkpoint of a Run in the async API (ADRs 0002, 0102 and 0116).

    Returned by `AsyncRun.checkpoint()` once the engine's file is whole, with
    `save_as(path)` and `stream()`.
    """

    def __init__(
        self,
        connection: ClientConnection,
        descriptor: WireCheckpoint,
    ) -> None:
        """Takes the Connection and the wire Checkpoint."""
        self._connection = connection
        self._descriptor = descriptor

    @property
    def run_id(self) -> RunId:
        """The Run's ID."""
        return self._descriptor.run_id

    @property
    def location(self) -> str:
        """Where it is: the file's path on the engine's machine."""
        return self._descriptor.location

    @property
    def size(self) -> int:
        """Its size in bytes."""
        return self._descriptor.size

    @property
    def digest(self) -> str:
        """The SHA-256 of its bytes, in lowercase hexadecimal."""
        return self._descriptor.digest

    @property
    def run_epoch(self) -> int:
        """The run epoch of the writer that took it (ADR 0003)."""
        return self._descriptor.run_epoch

    @property
    def journal_position(self) -> int:
        """The position of the run's journal it holds."""
        return self._descriptor.journal_position

    @property
    def time(self) -> datetime:
        """When it was taken."""
        return self._descriptor.time

    async def save_as(self, path: str | os.PathLike[str]) -> CheckpointSaved:
        """Saves this Checkpoint's file to `path`, whole or not at all (ADR 0116).

        Saves a Checkpoint's file, which the engine keeps on its machine, whole
        or not at all (ADRs 0102 and 0116). An Attached engine given the
        program's path writes it through a hidden temporary file beside it,
        synced and renamed over it, creating missing folders and replacing a
        file there; any other engine writes nothing on its own machine and
        sends the file in `$/chunk` notifications of at most 64 KiB, one per
        credit granted with `$/credit`, for the SDK to write. Answers the
        file's size, its SHA-256 digest and its run epoch. A path that cannot
        be written raises `path_not_writable`, and full or failing storage
        `file_write_failed`, leaving the path as it was.

        Experimental: outside semver until it is promoted (ADR 0030).
        """
        warn_entry("checkpoint.save_as", EXPERIMENTAL_OPERATIONS, DEPRECATED_OPERATIONS)
        raw = await save_file(
            self._connection,
            "checkpoint.save_as",
            {
                "run_id": self._descriptor.run_id.root,
                "location": self._descriptor.location,
            },
            path,
            call="save_as",
        )
        return result_of(CheckpointSaved, raw)

    def stream(self) -> AsyncSequence[bytes]:
        """Streams this Checkpoint's bytes in order, read with `async for` (ADR 0116).

        Sends a Checkpoint's bytes in order, in `$/chunk` notifications of at
        most 64 KiB, one per credit granted with `$/credit`, and answers its
        size, its SHA-256 digest and its run epoch once every byte is sent
        (ADRs 0102 and 0116). Nothing is written on either machine: a program
        that writes the bytes itself meets its own language's file errors.

        Experimental: outside semver until it is promoted (ADR 0030).
        """
        warn_entry("checkpoint.stream", EXPERIMENTAL_OPERATIONS, DEPRECATED_OPERATIONS)
        return _stream_chunks(
            self._connection,
            "checkpoint.stream",
            {
                "run_id": self._descriptor.run_id.root,
                "location": self._descriptor.location,
            },
        )


@final
class Checkpoint:
    """A completed Checkpoint of a Run in the sync API (ADRs 0002, 0102 and 0116)."""

    def __init__(self, loop: PrivateLoop, checkpoint: AsyncCheckpoint) -> None:
        """Takes the Connection's private loop and the async Checkpoint."""
        self._loop = loop
        self._checkpoint = checkpoint

    @property
    def run_id(self) -> RunId:
        """The Run's ID."""
        return self._checkpoint.run_id

    @property
    def location(self) -> str:
        """Where it is: the file's path on the engine's machine."""
        return self._checkpoint.location

    @property
    def size(self) -> int:
        """Its size in bytes."""
        return self._checkpoint.size

    @property
    def digest(self) -> str:
        """The SHA-256 of its bytes, in lowercase hexadecimal."""
        return self._checkpoint.digest

    @property
    def run_epoch(self) -> int:
        """The run epoch of the writer that took it (ADR 0003)."""
        return self._checkpoint.run_epoch

    @property
    def journal_position(self) -> int:
        """The position of the run's journal it holds."""
        return self._checkpoint.journal_position

    @property
    def time(self) -> datetime:
        """When it was taken."""
        return self._checkpoint.time

    def save_as(self, path: str | os.PathLike[str]) -> CheckpointSaved:
        """Saves this Checkpoint's file to `path`, as `AsyncCheckpoint.save_as` says.

        Saves a Checkpoint's file, which the engine keeps on its machine, whole
        or not at all (ADRs 0102 and 0116). An Attached engine given the
        program's path writes it through a hidden temporary file beside it,
        synced and renamed over it, creating missing folders and replacing a
        file there; any other engine writes nothing on its own machine and
        sends the file in `$/chunk` notifications of at most 64 KiB, one per
        credit granted with `$/credit`, for the SDK to write. Answers the
        file's size, its SHA-256 digest and its run epoch. A path that cannot
        be written raises `path_not_writable`, and full or failing storage
        `file_write_failed`, leaving the path as it was.

        Experimental: outside semver until it is promoted (ADR 0030).
        """
        saving = functools.partial(self._checkpoint.save_as, path)
        try:
            return self._loop.call(saving)
        except LoopStopped:
            raise closed_error("checkpoint.save_as") from None

    def stream(self) -> Sequence[bytes]:
        """Streams this Checkpoint's bytes in order, read with `for`.

        Sends a Checkpoint's bytes in order, in `$/chunk` notifications of at
        most 64 KiB, one per credit granted with `$/credit`, and answers its
        size, its SHA-256 digest and its run epoch once every byte is sent
        (ADRs 0102 and 0116). Nothing is written on either machine: a program
        that writes the bytes itself meets its own language's file errors.

        Experimental: outside semver until it is promoted (ADR 0030).
        """
        checkpoint = self._checkpoint

        async def opening() -> AsyncSequence[bytes]:
            return checkpoint.stream()

        try:
            sequence = self._loop.call(opening)
        except LoopStopped:
            raise closed_error("checkpoint.stream") from None
        return Sequence(self._loop, sequence, lambda: closed_error("checkpoint.stream"))


@final
class AsyncRunTrace:
    """A Run's trace copy in the async API (ADRs 0102 and 0116).

    Returned by `AsyncRun.trace()` and `AsyncRunView.trace()` once the engine's
    copy is whole, with `save_as(path)` and `stream()`.
    """

    def __init__(
        self,
        connection: ClientConnection,
        descriptor: WireRunTrace,
    ) -> None:
        """Takes the Connection and the wire RunTrace."""
        self._connection = connection
        self._descriptor = descriptor

    @property
    def run_id(self) -> RunId:
        """The Run's ID."""
        return self._descriptor.run_id

    @property
    def location(self) -> str:
        """Where it is: the trace file's path on the engine's machine."""
        return self._descriptor.location

    @property
    def size(self) -> int:
        """Its size in bytes."""
        return self._descriptor.size

    @property
    def digest(self) -> str:
        """The SHA-256 of its bytes, in lowercase hexadecimal."""
        return self._descriptor.digest

    async def save_as(self, path: str | os.PathLike[str]) -> TraceSaved:
        """Saves this Run trace's file to `path`, whole or not at all (ADR 0116).

        Saves a Run trace's file, which the engine keeps on its machine, whole
        or not at all (ADRs 0102 and 0116). An Attached engine given the
        program's path writes it through a hidden temporary file beside it,
        synced and renamed over it, creating missing folders and replacing a
        file there; any other engine writes nothing on its own machine and
        sends the file in `$/chunk` notifications of at most 64 KiB, one per
        credit granted with `$/credit`, for the SDK to write. Answers the
        file's size and its SHA-256 digest. A path that cannot be written
        raises `path_not_writable`, and full or failing storage
        `file_write_failed`, leaving the path as it was.

        Experimental: outside semver until it is promoted (ADR 0030).
        """
        warn_entry("trace.save_as", EXPERIMENTAL_OPERATIONS, DEPRECATED_OPERATIONS)
        raw = await save_file(
            self._connection,
            "trace.save_as",
            {
                "run_id": self._descriptor.run_id.root,
                "location": self._descriptor.location,
            },
            path,
            call="save_as",
        )
        return result_of(TraceSaved, raw)

    def stream(self) -> AsyncSequence[bytes]:
        """Streams this Run trace's bytes in order, read with `async for` (ADR 0116).

        Sends a Run trace's bytes in order, in `$/chunk` notifications of at
        most 64 KiB, one per credit granted with `$/credit`, and answers its
        size and its SHA-256 digest once every byte is sent (ADRs 0102 and
        0116). Nothing is written on either machine: a program that writes the
        bytes itself meets its own language's file errors.

        Experimental: outside semver until it is promoted (ADR 0030).
        """
        warn_entry("trace.stream", EXPERIMENTAL_OPERATIONS, DEPRECATED_OPERATIONS)
        return _stream_chunks(
            self._connection,
            "trace.stream",
            {
                "run_id": self._descriptor.run_id.root,
                "location": self._descriptor.location,
            },
        )


@final
class RunTrace:
    """A Run's trace copy in the sync API (ADRs 0102 and 0116)."""

    def __init__(self, loop: PrivateLoop, trace: AsyncRunTrace) -> None:
        """Takes the Connection's private loop and the async RunTrace."""
        self._loop = loop
        self._trace = trace

    @property
    def run_id(self) -> RunId:
        """The Run's ID."""
        return self._trace.run_id

    @property
    def location(self) -> str:
        """Where it is: the trace file's path on the engine's machine."""
        return self._trace.location

    @property
    def size(self) -> int:
        """Its size in bytes."""
        return self._trace.size

    @property
    def digest(self) -> str:
        """The SHA-256 of its bytes, in lowercase hexadecimal."""
        return self._trace.digest

    def save_as(self, path: str | os.PathLike[str]) -> TraceSaved:
        """Saves this Run trace's file to `path`, as `AsyncRunTrace.save_as` says.

        Saves a Run trace's file, which the engine keeps on its machine, whole
        or not at all (ADRs 0102 and 0116). An Attached engine given the
        program's path writes it through a hidden temporary file beside it,
        synced and renamed over it, creating missing folders and replacing a
        file there; any other engine writes nothing on its own machine and
        sends the file in `$/chunk` notifications of at most 64 KiB, one per
        credit granted with `$/credit`, for the SDK to write. Answers the
        file's size and its SHA-256 digest. A path that cannot be written
        raises `path_not_writable`, and full or failing storage
        `file_write_failed`, leaving the path as it was.

        Experimental: outside semver until it is promoted (ADR 0030).
        """
        saving = functools.partial(self._trace.save_as, path)
        try:
            return self._loop.call(saving)
        except LoopStopped:
            raise closed_error("trace.save_as") from None

    def stream(self) -> Sequence[bytes]:
        """Streams this Run trace's bytes in order, read with `for`.

        Sends a Run trace's bytes in order, in `$/chunk` notifications of at
        most 64 KiB, one per credit granted with `$/credit`, and answers its
        size and its SHA-256 digest once every byte is sent (ADRs 0102 and
        0116). Nothing is written on either machine: a program that writes the
        bytes itself meets its own language's file errors.

        Experimental: outside semver until it is promoted (ADR 0030).
        """
        trace = self._trace

        async def opening() -> AsyncSequence[bytes]:
            return trace.stream()

        try:
            sequence = self._loop.call(opening)
        except LoopStopped:
            raise closed_error("trace.stream") from None
        return Sequence(self._loop, sequence, lambda: closed_error("trace.stream"))
