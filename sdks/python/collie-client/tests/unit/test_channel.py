"""The Channel's queue of calls held back for a place (issue 3188).

The Collie tests play the queue through a Connection; this pins what no
Connection reaches while a call waits in the queue: the Connection's close
first sends `run.abort`, which waits behind it. A stream's `sent` is told
only of a call written, since a caller grants its credits on it.
"""

from __future__ import annotations

import json
import math
from typing import TYPE_CHECKING, cast

import anyio
import anyio.lowlevel
import pytest
from anyio.abc import ByteStream
from goodfetch_collie_client._rpc import Channel, ServedRequest, TransportEnd
from goodfetch_collie_client.generated.errors import CallCancelledError

if TYPE_CHECKING:
    from collections.abc import Mapping

    from goodfetch_collie_client.goodfetch_error import GoodfetchError


class _SilentEngine(ByteStream):
    """An engine that reads every request and answers none."""

    def __init__(self) -> None:
        self.methods: list[object] = []
        """The method of each message read, in order."""
        self._replies, self._to_client = anyio.create_memory_object_stream[bytes](
            math.inf
        )

    async def receive(self, max_bytes: int = 65536) -> bytes:
        del max_bytes
        try:
            return await self._to_client.receive()
        except anyio.ClosedResourceError:
            raise anyio.EndOfStream from None

    async def send(self, item: bytes) -> None:
        await anyio.lowlevel.checkpoint()
        for line in item.decode().splitlines():
            message = cast("Mapping[str, object]", json.loads(line))
            self.methods.append(message.get("method"))

    async def send_eof(self) -> None:
        self._replies.close()

    async def aclose(self) -> None:
        self._replies.close()
        self._to_client.close()


class _Owner:
    async def ended(self, end: TransportEnd) -> GoodfetchError:
        del end
        return CallCancelledError("the engine's side ended")

    def notified(self, method: str, params: object) -> None:
        del method, params

    def serves(self, request: ServedRequest) -> bool:
        del request
        return False


@pytest.mark.anyio
async def test_tells_sent_nothing_of_a_queued_call_failed_before_its_turn() -> None:
    """Cancelled with every call while queued, it is never sent, nor `sent` told."""
    engine = _SilentEngine()
    channel = Channel(engine, _Owner(), {})
    channel.cap_calls_in_flight(1)
    told: list[int] = []
    failed: list[str] = []
    cancelled: list[int] = []

    async def making(method: str) -> None:
        try:
            await channel.stream(method, {}, lambda _m, _p: None, sent=told.append)
        except CallCancelledError:
            failed.append(method)

    async with anyio.create_task_group() as calls:
        calls.start_soon(making, "run.list")
        await anyio.wait_all_tasks_blocked()
        calls.start_soon(making, "config.explain")
        await anyio.wait_all_tasks_blocked()
        cancelled = channel.cancel_all(CallCancelledError("the Connection closed"))

    assert cancelled == [0]
    assert told == [0]
    assert engine.methods == ["run.list"]
    assert sorted(failed) == ["config.explain", "run.list"]
