"""The engine's requests a Client serves, such as Foxhound's `crawl_request.handle`.

The channel hands each request the engine sends to its owner, which runs
it where it chooses and answers through it; a method the owner does not
serve is answered as JSON-RPC's unknown method. The engine's `$/cancel`
names a request of its own: it stops the work and answers `call_cancelled`,
and the work's own answer is then dropped, as it is once the channel ends.
TypeScript's `rpc.ts` serves them alike.
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
from goodfetch_collie_client.generated.errors import (
    CallCancelledError,
    RunNotFoundError,
)

if TYPE_CHECKING:
    from collections.abc import Awaitable, Callable, Mapping

    from anyio.abc import TaskGroup
    from goodfetch_collie_client.goodfetch_error import GoodfetchError


class _Engine(ByteStream):
    """An engine that writes the lines a test gives it and keeps what it reads."""

    def __init__(self) -> None:
        self.read: list[Mapping[str, object]] = []
        """Each message the Client wrote, in order."""
        self.answered = anyio.Event()
        """Set once the Client wrote a message with an `id`."""
        self._lines, self._to_client = anyio.create_memory_object_stream[bytes](
            math.inf
        )

    def write(self, message: Mapping[str, object]) -> None:
        """Sends `message` to the Client."""
        line = json.dumps({"jsonrpc": "2.0", **message})
        self._lines.send_nowait(line.encode() + b"\n")

    def end(self) -> None:
        """Ends the engine's output."""
        self._lines.close()

    async def receive(self, max_bytes: int = 65536) -> bytes:
        del max_bytes
        try:
            return await self._to_client.receive()
        except (anyio.ClosedResourceError, anyio.EndOfStream):
            raise anyio.EndOfStream from None

    async def send(self, item: bytes) -> None:
        await anyio.lowlevel.checkpoint()
        for line in item.decode().splitlines():
            message = cast("Mapping[str, object]", json.loads(line))
            self.read.append(message)
            if "id" in message:
                self.answered.set()

    async def send_eof(self) -> None:
        self._lines.close()

    async def aclose(self) -> None:
        self._lines.close()
        self._to_client.close()


class _Owner:
    """Serves `crawl_request.handle` with `work`, run in `group`."""

    def __init__(
        self,
        group: TaskGroup,
        work: Callable[[ServedRequest], Awaitable[object]],
    ) -> None:
        self._group = group
        self._work = work

    async def ended(self, end: TransportEnd) -> GoodfetchError:
        del end
        return CallCancelledError("the engine's side ended")

    def notified(self, method: str, params: object) -> None:
        del method, params

    def serves(self, request: ServedRequest) -> bool:
        if request.method != "crawl_request.handle":
            return False

        async def work() -> object:
            return await self._work(request)

        self._group.start_soon(request.serve, work)
        return True


def _handle(request_id: int) -> Mapping[str, object]:
    return {
        "id": request_id,
        "method": "crawl_request.handle",
        "params": {"run_id": "run-1"},
    }


@pytest.mark.anyio
async def test_answers_a_served_request_with_its_result() -> None:
    """The owner gets the method and params, and its result is the reply."""
    engine = _Engine()
    heard: list[tuple[str, object]] = []

    async def handled(request: ServedRequest) -> object:
        heard.append((request.method, request.params))
        return {}

    async with anyio.create_task_group() as group:
        channel = Channel(engine, _Owner(group, handled), {})
        engine.write(_handle(7))
        group.start_soon(channel.wait_ended)
        await engine.answered.wait()
        engine.end()

    assert (heard, engine.read) == (
        [("crawl_request.handle", {"run_id": "run-1"})],
        [{"jsonrpc": "2.0", "id": 7, "result": {}}],
    )


@pytest.mark.anyio
async def test_answers_a_request_no_one_serves_as_an_unknown_method() -> None:
    """The engine never waits on a method the Client does not serve."""
    engine = _Engine()

    async def handled(request: ServedRequest) -> object:
        del request
        return {}

    async with anyio.create_task_group() as group:
        channel = Channel(engine, _Owner(group, handled), {})
        engine.write({"id": 3, "method": "hook.run", "params": {}})
        group.start_soon(channel.wait_ended)
        await engine.answered.wait()
        engine.end()

    assert engine.read == [
        {
            "jsonrpc": "2.0",
            "id": 3,
            "error": {"code": -32601, "message": "this Client serves no hook.run"},
        }
    ]


@pytest.mark.anyio
async def test_answers_a_served_requests_error_with_its_code_and_fields() -> None:
    """The reply carries the wire class of its group, its code and its fields."""
    engine = _Engine()

    async def handled(request: ServedRequest) -> object:
        del request
        message = "the Run run-9 is not one this Connection started"
        raise RunNotFoundError(
            message,
            fix="Start the crawl on this Connection.",
            details={"run_id": "run-9"},
        )

    async with anyio.create_task_group() as group:
        channel = Channel(engine, _Owner(group, handled), {})
        engine.write(_handle(4))
        group.start_soon(channel.wait_ended)
        await engine.answered.wait()
        engine.end()

    assert engine.read == [
        {
            "jsonrpc": "2.0",
            "id": 4,
            "error": {
                "code": -32001,
                "message": "the Run run-9 is not one this Connection started",
                "data": {
                    "run_id": "run-9",
                    "code": "run_not_found",
                    "retryable": False,
                    "fix": "Start the crawl on this Connection.",
                },
            },
        }
    ]


@pytest.mark.anyio
async def test_answers_an_error_that_is_no_goodfetch_error_as_internal() -> None:
    """Such an error is a bug in what served the request."""
    engine = _Engine()

    async def handled(request: ServedRequest) -> object:
        del request
        message = "the handler's table has no such run"
        raise LookupError(message)

    async with anyio.create_task_group() as group:
        channel = Channel(engine, _Owner(group, handled), {})
        engine.write(_handle(8))
        group.start_soon(channel.wait_ended)
        await engine.answered.wait()
        engine.end()

    assert engine.read == [
        {
            "jsonrpc": "2.0",
            "id": 8,
            "error": {
                "code": -32007,
                "message": "the handler's table has no such run",
                "data": {"code": "internal", "retryable": False},
            },
        }
    ]


@pytest.mark.anyio
async def test_stops_a_request_the_engine_cancels_and_answers_call_cancelled() -> None:
    """The work is cancelled, and nothing it would answer reaches the engine."""
    engine = _Engine()
    started = anyio.Event()
    stopped: list[str] = []

    async def handled(request: ServedRequest) -> object:
        del request
        started.set()
        try:
            await anyio.sleep_forever()
        finally:
            stopped.append("cancelled")
        return {}

    async with anyio.create_task_group() as group:
        channel = Channel(engine, _Owner(group, handled), {})
        engine.write(_handle(5))
        group.start_soon(channel.wait_ended)
        await started.wait()
        engine.write({"method": "$/cancel", "params": {"id": 5}})
        await engine.answered.wait()
        await anyio.wait_all_tasks_blocked()
        engine.end()

    assert (stopped, engine.read) == (
        ["cancelled"],
        [
            {
                "jsonrpc": "2.0",
                "id": 5,
                "error": {
                    "code": -32006,
                    "message": "the engine cancelled its request",
                    "data": {
                        "by": "client",
                        "code": "call_cancelled",
                        "retryable": False,
                    },
                },
            }
        ],
    )


@pytest.mark.anyio
async def test_stops_the_requests_it_serves_once_the_engines_side_ends() -> None:
    """No answer reaches an engine that ended, so the work stops unanswered."""
    engine = _Engine()
    started = anyio.Event()
    stopped: list[str] = []

    async def handled(request: ServedRequest) -> object:
        del request
        started.set()
        try:
            await anyio.sleep_forever()
        finally:
            stopped.append("cancelled")
        return {}

    async with anyio.create_task_group() as group:
        channel = Channel(engine, _Owner(group, handled), {})
        engine.write(_handle(6))
        group.start_soon(channel.wait_ended)
        await started.wait()
        engine.end()

    assert (stopped, engine.read) == (["cancelled"], [])
