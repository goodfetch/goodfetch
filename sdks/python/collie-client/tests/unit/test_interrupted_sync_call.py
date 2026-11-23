"""The order on the wire of a sync call's `$/cancel` (issue 3517, ADR 0081).

An exception raised in the thread blocked in a sync call, as Ctrl+C's
`KeyboardInterrupt`, returns to the caller at once, and the call's
`$/cancel` reaches the engine before any request sent after it. The call's
own task may notice its cancellation late, here because it is answering the
engine's request through a write the engine holds, so the next request
waits for that write too; once the engine reads again, the `$/cancel`
comes first. The signal is a stand-in for SIGINT that raises an exception
of the test's own, so the test runner never sees a `KeyboardInterrupt`.
"""

from __future__ import annotations

import json
import math
import signal
import sys
import threading
import time
from typing import TYPE_CHECKING, cast

import anyio
import anyio.lowlevel
import pytest
from anyio.abc import ByteStream
from goodfetch_collie_client._rpc import Channel, ServedRequest, TransportEnd
from goodfetch_collie_client.generated.errors import CallCancelledError
from goodfetch_collie_client.portal import PrivateLoop

if TYPE_CHECKING:
    from collections.abc import Iterator, Mapping
    from types import FrameType

    from goodfetch_collie_client.goodfetch_error import GoodfetchError

pytestmark = pytest.mark.skipif(
    sys.platform == "win32", reason="it signals the main thread, which needs POSIX"
)

WAIT_SECONDS = 10.0
"""How long the test waits for any one step before it fails."""

ASKED = {"jsonrpc": "2.0", "id": "asked", "method": "engine.asks"}
"""The engine's request the first call's reader answers, through the held write."""


class _Interrupted(Exception):
    """What the stand-in for Ctrl+C raises in the thread blocked in the call."""


class _HoldingEngine(ByteStream):
    """An engine that holds the Client's answer to its request until released.

    It asks the Client something once it reads `slow.call`, answers
    `next.call` at once, and records what it reads, in order.
    """

    def __init__(self) -> None:
        self.read: list[str] = []
        """Each message read: its method, or `answer` for an answer."""
        self.holding = threading.Event()
        """Set once the Client's answer is being written and held."""
        self.release = anyio.Event()
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
            method = message.get("method")
            if method is None:
                self.holding.set()
                await self.release.wait()
                self.read.append("answer")
                continue
            self.read.append(str(method))
            if method == "slow.call":
                await self._replies.send(json.dumps(ASKED).encode() + b"\n")
            elif method == "next.call":
                answer = {"jsonrpc": "2.0", "id": message["id"], "result": "answered"}
                await self._replies.send(json.dumps(answer).encode() + b"\n")

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


def _stack(frame: FrameType | None) -> Iterator[FrameType]:
    while frame is not None:
        yield frame
        frame = frame.f_back


def _blocked_in_the_call(thread: threading.Thread) -> bool:
    """Whether `thread` waits for its sync call's result, past starting it."""
    frames = sys._current_frames()  # pyright: ignore[reportPrivateUsage]
    frame = frames.get(cast("int", thread.ident))
    names = [each.f_code.co_name for each in _stack(frame)]
    return (
        frame is not None
        and frame.f_code.co_filename == threading.__file__
        and "call" in names
        and "start_task_soon" not in names
    )


def _interrupt_once_held(engine: _HoldingEngine, caller: threading.Thread) -> None:
    """Raises `_Interrupted` in `caller` once its call answers through the hold."""
    assert engine.holding.wait(WAIT_SECONDS)
    deadline = time.monotonic() + WAIT_SECONDS
    while not _blocked_in_the_call(caller):
        assert time.monotonic() < deadline, "the caller never waited for its call"
        time.sleep(0.001)
    signal.pthread_kill(cast("int", caller.ident), signal.SIGUSR1)


def _raise_interrupted(signum: int, frame: FrameType | None) -> None:
    del signum, frame
    raise _Interrupted


@pytest.fixture
def interruptible() -> Iterator[None]:
    """The stand-in for Ctrl+C's handler, for the test's length."""
    assert threading.current_thread() is threading.main_thread()
    previous = signal.signal(signal.SIGUSR1, _raise_interrupted)
    try:
        yield
    finally:
        signal.signal(signal.SIGUSR1, previous)


@pytest.mark.usefixtures("interruptible")
def test_an_interrupted_sync_call_sends_its_cancel_before_the_next_request() -> None:
    """Interrupted while its task is busy, the call's `$/cancel` still goes first."""
    loop = PrivateLoop()
    try:

        async def opening() -> tuple[_HoldingEngine, Channel]:
            engine = _HoldingEngine()
            return engine, Channel(engine, _Owner(), {})

        engine, channel = loop.call(opening)
        interrupter = threading.Thread(
            target=_interrupt_once_held, args=(engine, threading.main_thread())
        )
        interrupter.start()
        started = time.monotonic()
        with pytest.raises(_Interrupted):
            loop.call(lambda: channel.call("slow.call", {}))
        interrupter.join()
        assert time.monotonic() - started < WAIT_SECONDS

        next_started = threading.Event()
        answers: list[object] = []

        async def next_call() -> object:
            next_started.set()
            return await channel.call("next.call", {})

        caller = threading.Thread(target=lambda: answers.append(loop.call(next_call)))
        caller.start()
        assert next_started.wait(WAIT_SECONDS)
        loop.call(anyio.wait_all_tasks_blocked)
        loop.call(lambda: _set(engine.release))
        caller.join(WAIT_SECONDS)
        loop.call(anyio.wait_all_tasks_blocked)

        assert answers == ["answered"]
        assert engine.read == ["slow.call", "answer", "$/cancel", "next.call"]
    finally:
        loop.stop()


async def _set(event: anyio.Event) -> None:
    event.set()
