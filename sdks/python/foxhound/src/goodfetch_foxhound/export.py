"""A crawl's Dataset Export (ADRs 0103, 0116 and 0119; issue 1488).

`AsyncCrawlRun.export` calls `run.export` and resolves to an `AsyncExport`
only once the engine's file is whole; `CrawlRun.export` is its sync twin,
returning an `Export`. `save_as(path)` writes the file on the program's
machine whole or not at all through a hidden temporary file, and `stream()`
yields its bytes in order.
"""

from __future__ import annotations

import base64
from collections.abc import Mapping
from typing import TYPE_CHECKING, cast, final

from goodfetch_collie_client._file_saving import save_file
from goodfetch_collie_client.caller import result_of
from goodfetch_collie_client.connection import closed_error
from goodfetch_collie_client.markers import warn_entry
from goodfetch_collie_client.portal import LoopStopped
from goodfetch_collie_client.sequence import Sequence

from goodfetch_foxhound.generated.models import (
    Export as WireExport,
)
from goodfetch_foxhound.generated.models import (
    ExportCompression,
    ExportFormat,
    ExportId,
    ExportSaved,
)
from goodfetch_foxhound.generated.operations import (
    DEPRECATED_OPERATIONS,
    EXPERIMENTAL_OPERATIONS,
)

if TYPE_CHECKING:
    import os
    from collections.abc import Callable

    from goodfetch_collie_client.connection import ClientConnection
    from goodfetch_collie_client.portal import PrivateLoop
    from goodfetch_collie_client.sequence import AsyncSequence

__all__ = ["AsyncExport", "Export"]

CHUNK = "$/chunk"
"""The notification each byte chunk `export.stream` sends comes in."""


def _chunk_of(method: str, params: object) -> bytes | None:
    """One `$/chunk` notification's bytes, or None for any other notification."""
    if method != CHUNK or not isinstance(params, Mapping):
        return None
    members = cast("Mapping[str, object]", params)
    data = members.get("data")
    return base64.b64decode(data) if isinstance(data, str) else None


@final
class AsyncExport:
    """A completed Export of one Dataset of a crawl's Run (ADRs 0103 and 0116).

    Returned by `AsyncCrawlRun.export` once the engine's file is whole, with
    `save_as(path)` and `stream()`.
    """

    def __init__(
        self,
        connection: ClientConnection,
        descriptor: WireExport,
        open_stream: Callable[
            [str, Mapping[str, object], Callable[[str, object], bytes | None]],
            AsyncSequence[bytes],
        ],
    ) -> None:
        """Takes the Connection, the wire Export descriptor and the stream opener."""
        self._connection = connection
        self._descriptor = descriptor
        self._open_stream = open_stream

    @property
    def id(self) -> ExportId:
        """The Export's ID."""
        return self._descriptor.export_id

    @property
    def format(self) -> ExportFormat:
        """Its format."""
        return self._descriptor.format

    @property
    def compression(self) -> ExportCompression:
        """Its compression."""
        return self._descriptor.compression

    @property
    def records(self) -> int:
        """How many Records it holds."""
        return self._descriptor.records

    @property
    def last_number(self) -> int:
        """The number of the last Record it holds, 0 when it holds none."""
        return self._descriptor.last_number

    @property
    def size(self) -> int:
        """Its size in bytes."""
        return self._descriptor.size

    @property
    def sha256(self) -> str:
        """The SHA-256 of its bytes, in lowercase hexadecimal."""
        return self._descriptor.sha256

    @property
    def run_epoch(self) -> int:
        """The run epoch of the Run's state it was written from (ADR 0003)."""
        return self._descriptor.run_epoch

    async def save_as(self, path: str | os.PathLike[str]) -> ExportSaved:
        """Saves this Export's file to `path`, whole or not at all (ADR 0116).

        Saves an Export's file, which the engine keeps in its Run's folder,
        whole or not at all. An Attached engine given the program's path
        writes it through a hidden temporary file beside it, synced and
        renamed over it, creating missing folders and replacing a file there;
        any other engine writes nothing on its own machine and sends the file
        in `$/chunk` notifications of at most 64 KiB, one per credit granted
        with `$/credit`, for the SDK to write. Answers the file's size, its
        SHA-256 and the run epoch the Export was written under. A path that
        cannot be written raises `path_not_writable`, and full or failing
        storage `file_write_failed`, leaving the path as it was.

        Experimental: outside semver until it is promoted (ADR 0030).

        Raises:
            PathNotWritableError: The path cannot hold a file.
            FileWriteFailedError: Storage failed while writing the file.
            ClosedError: The Connection is closed.
        """
        warn_entry("export.save_as", EXPERIMENTAL_OPERATIONS, DEPRECATED_OPERATIONS)
        raw = await save_file(
            self._connection,
            "export.save_as",
            {"export_id": self._descriptor.export_id.root},
            path,
            call="save_as",
        )
        return result_of(ExportSaved, raw)

    def stream(self) -> AsyncSequence[bytes]:
        """Streams this Export's bytes in order, read with `async for` (ADR 0116).

        Sends an Export's bytes, which the engine keeps in its Run's folder,
        in order, in `$/chunk` notifications of at most 64 KiB, one per credit
        granted with `$/credit`, and answers its size, its SHA-256 and the run
        epoch it was written under once every byte is sent. Nothing is written
        on either machine: a program that writes the bytes itself meets its
        own language's file errors.

        Experimental: outside semver until it is promoted (ADR 0030).

        Raises:
            ClosedError: The Run or its Connection is closed; nothing is sent.
            RuntimeError: No asyncio or Trio loop runs on this thread.
        """
        warn_entry("export.stream", EXPERIMENTAL_OPERATIONS, DEPRECATED_OPERATIONS)
        return self._open_stream(
            "export.stream",
            {"export_id": self._descriptor.export_id.root},
            _chunk_of,
        )


@final
class Export:
    """A completed Export of one Dataset of a crawl's Run, sync twin (ADR 0081).

    Returned by `CrawlRun.export` once the engine's file is whole, with
    `save_as(path)` and `stream()`.
    """

    def __init__(self, loop: PrivateLoop, async_export: AsyncExport) -> None:
        """Takes the Connection's private loop and its async Export twin."""
        self._loop = loop
        self._async_export = async_export

    @property
    def id(self) -> ExportId:
        """The Export's ID."""
        return self._async_export.id

    @property
    def format(self) -> ExportFormat:
        """Its format."""
        return self._async_export.format

    @property
    def compression(self) -> ExportCompression:
        """Its compression."""
        return self._async_export.compression

    @property
    def records(self) -> int:
        """How many Records it holds."""
        return self._async_export.records

    @property
    def last_number(self) -> int:
        """The number of the last Record it holds, 0 when it holds none."""
        return self._async_export.last_number

    @property
    def size(self) -> int:
        """Its size in bytes."""
        return self._async_export.size

    @property
    def sha256(self) -> str:
        """The SHA-256 of its bytes, in lowercase hexadecimal."""
        return self._async_export.sha256

    @property
    def run_epoch(self) -> int:
        """The run epoch of the Run's state it was written from (ADR 0003)."""
        return self._async_export.run_epoch

    def save_as(self, path: str | os.PathLike[str]) -> ExportSaved:
        """Saves this Export's file to `path`, as `AsyncExport.save_as` does.

        Saves an Export's file, which the engine keeps in its Run's folder,
        whole or not at all. An Attached engine given the program's path
        writes it through a hidden temporary file beside it, synced and
        renamed over it, creating missing folders and replacing a file there;
        any other engine writes nothing on its own machine and sends the file
        in `$/chunk` notifications of at most 64 KiB, one per credit granted
        with `$/credit`, for the SDK to write. Answers the file's size, its
        SHA-256 and the run epoch the Export was written under. A path that
        cannot be written raises `path_not_writable`, and full or failing
        storage `file_write_failed`, leaving the path as it was.

        Experimental: outside semver until it is promoted (ADR 0030).

        Raises:
            PathNotWritableError: The path cannot hold a file.
            FileWriteFailedError: Storage failed while writing the file.
            ClosedError: The Connection is closed.
        """
        async_export = self._async_export
        try:
            return self._loop.call(lambda: async_export.save_as(path))
        except LoopStopped:
            raise closed_error("export.save_as") from None

    def stream(self) -> Sequence[bytes]:
        """Streams this Export's bytes in order, read with `for` (ADR 0116).

        Sends an Export's bytes, which the engine keeps in its Run's folder,
        in order, in `$/chunk` notifications of at most 64 KiB, one per credit
        granted with `$/credit`, and answers its size, its SHA-256 and the run
        epoch it was written under once every byte is sent. Nothing is written
        on either machine: a program that writes the bytes itself meets its
        own language's file errors.

        Experimental: outside semver until it is promoted (ADR 0030).

        Raises:
            ClosedError: The Run or its Connection is closed; nothing is sent.
        """
        async_export = self._async_export

        async def opening() -> AsyncSequence[bytes]:
            return async_export.stream()

        try:
            sequence = self._loop.call(opening)
        except LoopStopped:
            raise closed_error("export.stream") from None
        return Sequence(self._loop, sequence, lambda: closed_error("export.stream"))
