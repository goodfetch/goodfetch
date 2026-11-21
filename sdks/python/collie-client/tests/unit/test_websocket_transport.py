"""The protocol over WebSocket to an Exposed Daemon, on an in-memory wire.

The Collie tests reach the Scripted engine's WebSocket; these pin what a
script cannot show: a line read in pieces no longer than `receive` asks for,
and a close that a send stuck on a Daemon reading nothing does not hold back.
"""

from __future__ import annotations

import logging

import anyio
import pytest
from anyio.abc import ByteStream
from goodfetch_collie_client._websocket_transport import open_websocket
from goodfetch_collie_client.generated.connection import WEBSOCKET_SUBPROTOCOL
from websockets.http11 import Request
from websockets.server import ServerProtocol
from websockets.typing import Subprotocol

_UNLOGGED = logging.Logger("test-websocket")
_UNLOGGED.disabled = True


class _Daemon(ByteStream):
    """The Client's end of a wire to a Daemon played by websockets' Sans-I/O layer.

    It answers the handshake with the offered subprotocol; `say` queues a
    message to the Client, and while `stuck` is set a send never ends until
    the stream is closed, as on a Daemon that reads nothing.
    """

    def __init__(self) -> None:
        self.protocol = ServerProtocol(
            subprotocols=[Subprotocol(WEBSOCKET_SUBPROTOCOL)], logger=_UNLOGGED
        )
        self._to_client, self._from_daemon = anyio.create_memory_object_stream[bytes](
            16
        )
        self.stuck = False
        self._closed = anyio.Event()

    def say(self, text: str) -> None:
        self.protocol.send_text(text.encode())
        self._deliver()

    def _deliver(self) -> None:
        for data in self.protocol.data_to_send():
            if data:
                self._to_client.send_nowait(data)

    async def receive(self, max_bytes: int = 65536) -> bytes:
        del max_bytes
        return await self._from_daemon.receive()

    async def send(self, item: bytes) -> None:
        if self.stuck:
            await self._closed.wait()
            raise anyio.ClosedResourceError
        self.protocol.receive_data(item)
        for event in self.protocol.events_received():
            if isinstance(event, Request):
                self.protocol.send_response(self.protocol.accept(event))
        self._deliver()

    async def send_eof(self) -> None:
        pass

    async def aclose(self) -> None:
        self._closed.set()
        await self._to_client.aclose()


@pytest.mark.anyio
async def test_reads_a_message_in_pieces_no_longer_than_asked() -> None:
    """The rest of the line waits for the next `receive`."""
    daemon = _Daemon()
    stream = await open_websocket(
        daemon, "ws://127.0.0.1:7700", origin="http://127.0.0.1:7700"
    )
    assert stream is not None

    daemon.say('{"id":1}')
    pieces = [await stream.receive(3) for _ in range(3)]

    assert pieces == [b'{"i', b'd":', b"1}\n"]


@pytest.mark.anyio
async def test_closes_at_once_while_a_send_is_stuck_on_a_daemon_reading_nothing() -> (
    None
):
    """`send_eof` returns, and `aclose` then ends the stuck send."""
    daemon = _Daemon()
    stream = await open_websocket(
        daemon, "ws://127.0.0.1:7700", origin="http://127.0.0.1:7700"
    )
    assert stream is not None
    daemon.stuck = True

    async with anyio.create_task_group() as tasks:

        async def stuck_send() -> None:
            with pytest.raises(anyio.ClosedResourceError):
                await stream.send(b'{"id":1}\n')

        tasks.start_soon(stuck_send)
        await anyio.wait_all_tasks_blocked()
        with anyio.fail_after(1):
            await stream.send_eof()
        await stream.aclose()
