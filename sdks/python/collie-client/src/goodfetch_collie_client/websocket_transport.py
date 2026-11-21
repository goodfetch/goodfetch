"""The protocol over WebSocket to an Exposed Daemon (ADR 0049; spec #606's Transports).

The subprotocol `goodfetch.v<major>` of the protocol version the SDK speaks,
and the same JSON-RPC messages as over stdio, one per WebSocket message each
way, so a Connection's newline-delimited transport runs over it unchanged.
websockets' Sans-I/O layer does the handshake and the framing over an AnyIO
stream, so it runs on asyncio and Trio alike (ADR 0081). TypeScript's
`websocket-transport.ts` is its twin.
"""

from __future__ import annotations

import contextlib
import logging
from collections import deque
from typing import TYPE_CHECKING, Final

import anyio
from anyio.abc import ByteStream
from websockets.client import ClientProtocol
from websockets.exceptions import InvalidState
from websockets.frames import Frame, Opcode
from websockets.protocol import State
from websockets.typing import Origin, Subprotocol
from websockets.uri import parse_uri

from goodfetch_collie_client.generated.connection import WEBSOCKET_SUBPROTOCOL

if TYPE_CHECKING:
    from collections.abc import Iterable

    from websockets.protocol import Event

_UNLOGGED: Final = logging.Logger("goodfetch-websocket")
"""The logger websockets is given, which records nothing.

Its debug lines show each frame, `hello`'s token included (ADR 0067); it is
no named logger, so no host's configuration turns it on.
"""
_UNLOGGED.disabled = True

_MESSAGE_OPCODES: Final = frozenset({Opcode.TEXT, Opcode.BINARY, Opcode.CONT})
"""The frames that carry a message's bytes."""


async def open_websocket(
    stream: ByteStream, url: str, *, origin: str
) -> ByteStream | None:
    """The Connection's transport over a WebSocket to `url`, opened on `stream`.

    The handshake sends `origin`, the Daemon's own, as a Client outside a
    browser does (ADRs 0049 and 0067), and offers `WEBSOCKET_SUBPROTOCOL`.
    `None`, once `stream` is closed, when the handshake failed: the Daemon
    refused it, `stream` ended, or it took no subprotocol, which a Daemon of
    another protocol major does and a WebSocket does not tell apart.
    """
    protocol = ClientProtocol(
        parse_uri(url),
        origin=Origin(origin),
        subprotocols=[Subprotocol(WEBSOCKET_SUBPROTOCOL)],
        # As over stdio, the Client takes a message of any length.
        max_size=None,
        logger=_UNLOGGED,
    )
    protocol.send_request(protocol.connect())
    try:
        await _send_all(stream, protocol.data_to_send())
        while protocol.state is State.CONNECTING and protocol.handshake_exc is None:
            try:
                data = await stream.receive()
            except anyio.EndOfStream:
                protocol.receive_eof()
            else:
                protocol.receive_data(data)
    except BaseException:
        await anyio.aclose_forcefully(stream)
        raise
    events = protocol.events_received()
    if (
        protocol.handshake_exc is not None
        or protocol.state is not State.OPEN
        or protocol.subprotocol != WEBSOCKET_SUBPROTOCOL
    ):
        await anyio.aclose_forcefully(stream)
        return None
    return _WebSocketStream(stream, protocol, events)


class _WebSocketStream(ByteStream):
    """A WebSocket, open, as a Connection's transport.

    Each message it receives is one line to read, and each line written is
    one text message. The Daemon's close ends the bytes to read; a write once
    it is closing fails.
    """

    def __init__(
        self, stream: ByteStream, protocol: ClientProtocol, events: Iterable[Event]
    ) -> None:
        self._stream = stream
        self._protocol = protocol
        self._events: deque[Event] = deque(events)
        self._message: list[bytes] = []
        """The bytes of the message being received, frame by frame."""
        self._unread = b""
        """What `receive` has not returned yet of the last line."""
        self._unsent = bytearray()
        """What was written past its last full line."""
        self._ended = False
        self._send_lock = anyio.Lock()

    async def receive(self, max_bytes: int = 65536) -> bytes:
        """Up to `max_bytes` of the next message received, as one line.

        Raises:
            anyio.EndOfStream: The Daemon closed the WebSocket, or its stream ended.
            anyio.BrokenResourceError: The Daemon broke the WebSocket's framing.
        """
        while True:
            line = self._unread or self._next_line()
            if line is not None:
                self._unread = line[max_bytes:]
                return line[:max_bytes]
            if self._ended:
                raise anyio.EndOfStream
            if self._protocol.parser_exc is not None:
                raise anyio.BrokenResourceError from self._protocol.parser_exc
            try:
                data = await self._stream.receive()
            except anyio.EndOfStream:
                self._protocol.receive_eof()
                self._ended = True
            else:
                self._protocol.receive_data(data)
            self._events.extend(self._protocol.events_received())
            # Pongs and the close's answer; a write that fails shows on the
            # next send, and the end of the bytes to read wins over it.
            with contextlib.suppress(
                anyio.BrokenResourceError, anyio.ClosedResourceError, OSError
            ):
                await self._flush()

    def _next_line(self) -> bytes | None:
        """The next whole message among the frames received, its line ended."""
        while self._events:
            event = self._events.popleft()
            if not isinstance(event, Frame):
                continue
            if event.opcode is Opcode.CLOSE:
                self._ended = True
                self._events.clear()
                return None
            if event.opcode in _MESSAGE_OPCODES:
                self._message.append(bytes(event.data))
                if event.fin:
                    line = b"".join(self._message) + b"\n"
                    self._message.clear()
                    return line
        return None

    async def send(self, item: bytes) -> None:
        """Sends each whole line written as one text message.

        Raises:
            anyio.ClosedResourceError: The WebSocket is closing or closed.
        """
        self._unsent += item
        end = self._unsent.find(b"\n")
        try:
            while end != -1:
                self._protocol.send_text(bytes(self._unsent[:end]))
                del self._unsent[: end + 1]
                end = self._unsent.find(b"\n")
        except InvalidState as cause:
            raise anyio.ClosedResourceError from cause
        await self._flush()

    async def send_eof(self) -> None:
        """Closes the WebSocket with 1000.

        The Daemon's messages until its own close still arrive. While a
        send is under way, it sends the close after its own bytes, and this
        returns at once: a send stuck on a Daemon that reads nothing holds
        no close back from `aclose`.
        """
        if self._protocol.state is State.OPEN:
            self._protocol.send_close(1000)
        if not self._send_lock.locked():
            await self._flush()

    async def aclose(self) -> None:
        """Closes the stream under the WebSocket at once."""
        await self._stream.aclose()

    async def _flush(self) -> None:
        """Sends what the protocol queued, in its order, until nothing is.

        Waiting for the send under way can be cancelled; a send, once
        started, is never cut short, which would break the framing.
        """
        async with self._send_lock:
            with anyio.CancelScope(shield=True):
                # Taken under the lock, so concurrent flushes keep the order,
                # and again until empty, so what was queued meanwhile goes too.
                while writes := self._protocol.data_to_send():
                    await _send_all(self._stream, writes)


async def _send_all(stream: ByteStream, writes: list[bytes]) -> None:
    """Sends `writes`, an empty one ending the stream's sending side."""
    for data in writes:
        if data:
            await stream.send(data)
        else:
            await stream.send_eof()
