"""Collie's calls under the engine's cap of calls in flight, and its own bounds.

`hello`'s result caps the calls in flight (ADR 0038): the Connection sends
at most that many at once and queues the rest, in the order they were
made. Each test plays the engine by hand, since a script cannot hold a
call's answer back while later calls arrive. TypeScript's `collie.test.ts`
is its twin.
"""

import json
import math
from collections.abc import Awaitable, Callable, Mapping
from datetime import timedelta
from typing import TypeVar, cast

import anyio
import anyio.lowlevel
import pytest
from anyio.abc import ByteStream
from goodfetch_collie import (
    AsyncCollie,
    Collie,
    GoodfetchError,
    JSONValue,
)
from goodfetch_collie.testing import async_connect_over
from goodfetch_test_support import Way, three_ways

_ANSWERED_IN_TIME = 5.0
"""How long, in seconds, a test waits for the Client to send what it awaits."""

_NO_PARAMS: Mapping[str, JSONValue] = {}

_Opened = TypeVar("_Opened")


class HandPlayedEngine(ByteStream):
    """An engine played by hand: its `hello` result caps the calls in flight at `cap`.

    Each later request it reads waits in `calls` until `answer` answers it,
    a cancelled one too; one read while `cap` calls wait is refused at once
    with `too_many_calls`, as the engine refuses it. With `hold_writes`,
    each write after `hello` waits until `release`, as on a transport the
    engine has stopped reading.
    """

    def __init__(self, cap: int | None, *, hold_writes: bool = False) -> None:
        """Takes the cap `hello` answers, left out of it when `None`."""
        self.calls: list[Mapping[str, object]] = []
        self.notified: list[object] = []
        """The method of each notification read, in order."""
        self.refused: list[object] = []
        """The method of each request refused with `too_many_calls`, in order."""
        self.held = 0
        """How many writes wait for `release`."""
        self._answered: set[int] = set()
        self._cap = cap
        self._hold_writes = hold_writes
        self._greeted = False
        self._released = anyio.Event()
        self._read = anyio.Event()
        """Set, then replaced, each time the engine reads or holds a write."""
        self._replies, self._to_client = anyio.create_memory_object_stream[bytes](
            math.inf
        )

    def methods(self) -> list[object]:
        """The method of each request read after `hello`, in order."""
        return [call.get("method") for call in self.calls]

    def answer(self, index: int, result: object) -> None:
        """Answers with `result` the request read in place `index`."""
        self._answered.add(index)
        self._reply({"id": self.calls[index]["id"], "result": result})

    def reply_to(self, call_id: int, result: object) -> None:
        """Answers with `result` the id `call_id`, whether or not it was sent."""
        self._reply({"id": call_id, "result": result})

    async def answer_once_sent(self, index: int, result: object) -> None:
        """Answers the request in place `index` once the Client has sent it."""
        await self.until(lambda: len(self.calls) > index)
        self.answer(index, result)

    async def until(self, done: Callable[[], bool]) -> None:
        """Waits until `done`, checked each time the engine reads or holds a write."""
        with anyio.fail_after(_ANSWERED_IN_TIME):
            while not done():
                await self._read.wait()

    def _moved(self) -> None:
        self._read.set()
        self._read = anyio.Event()

    def release(self) -> None:
        """Lets every write held, and every later one, through."""
        self._released.set()

    async def receive(self, max_bytes: int = 65536) -> bytes:
        """The engine's next reply."""
        del max_bytes
        try:
            return await self._to_client.receive()
        except anyio.ClosedResourceError:
            raise anyio.EndOfStream from None

    async def send(self, item: bytes) -> None:
        """Reads the Client's bytes, answering `hello` at once."""
        await anyio.lowlevel.checkpoint()
        if self._greeted and self._hold_writes and not self._released.is_set():
            self.held += 1
            self._moved()
            await self._released.wait()
        for line in item.decode().splitlines():
            message = cast("Mapping[str, object]", json.loads(line))
            if "method" not in message:
                continue
            if "id" not in message:
                self.notified.append(message["method"])
            elif message["method"] == "hello":
                self._greet(message["id"])
            elif self._cap is not None and self._in_flight() >= self._cap:
                self.refused.append(message["method"])
                self._refuse(message["id"])
            else:
                self.calls.append(message)
        self._moved()

    async def send_eof(self) -> None:
        """Ends the engine's side, as an Attached engine does at its input's end."""
        self._replies.close()

    async def aclose(self) -> None:
        """Ends both sides."""
        self._replies.close()
        self._to_client.close()

    def _greet(self, hello_id: object) -> None:
        result: dict[str, object] = {
            "protocol_version": "0.1",
            "product": "collie",
            "product_version": "0.0.0",
            "engine_instance": "0123456789abcdef",
        }
        if self._cap is not None:
            result["max_calls_in_flight"] = self._cap
        self._greeted = True
        self._reply({"id": hello_id, "result": result})

    def _in_flight(self) -> int:
        return len(self.calls) - len(self._answered)

    def _refuse(self, call_id: object) -> None:
        self._reply(
            {
                "id": call_id,
                "error": {
                    "code": -32000,
                    "message": "the engine already serves its most calls in flight",
                    "data": {"code": "too_many_calls", "retryable": True},
                },
            }
        )

    def _reply(self, reply: Mapping[str, object]) -> None:
        line = json.dumps({"jsonrpc": "2.0", **reply}) + "\n"
        self._replies.send_nowait(line.encode())


async def failure(call: Awaitable[object]) -> GoodfetchError:
    """The Goodfetch error `call` raises."""
    with pytest.raises(GoodfetchError) as raised:
        await call
    return raised.value


async def answering(
    engine: HandPlayedEngine,
    index: int,
    result: object,
    opening: Callable[[], Awaitable[_Opened]],
) -> _Opened:
    """What `opening` returns, its request in place `index` answered with `result`."""
    opened: list[_Opened] = []
    async with anyio.create_task_group() as answers:
        answers.start_soon(engine.answer_once_sent, index, result)
        opened.append(await opening())
    [value] = opened
    return value


@pytest.mark.anyio
async def test_starts_300_calls_at_once_and_gets_every_answer_sent_in_order() -> None:
    """At most the cap is in flight; the rest are sent in order as places free.

    None fails with `too_many_calls`, which the engine answers a call past
    its cap with.
    """
    engine = HandPlayedEngine(256)
    collie = await async_connect_over(engine)
    answers: dict[int, JSONValue] = {}
    sent_at_once = 0

    async def call(index: int) -> None:
        answers[index] = await collie.call("run.list", {})

    async with anyio.create_task_group() as calls:
        for index in range(300):
            calls.start_soon(call, index)
        await engine.until(lambda: len(engine.calls) == 256)
        # Every call made has been sent or queued before an answer frees a place.
        await anyio.wait_all_tasks_blocked()
        sent_at_once = len(engine.calls)
        for index in range(256):
            engine.answer(index, {"runs": []})
        for index in range(256, 300):
            await engine.answer_once_sent(index, {"runs": []})
    await collie.close()

    assert sent_at_once == 256
    assert engine.refused == []
    assert [call["id"] for call in engine.calls] == list(range(1, 301))
    assert answers == {index: {"runs": []} for index in range(300)}


@pytest.mark.covers("error:deadline_exceeded")
@pytest.mark.anyio
async def test_fails_a_raw_call_that_waits_past_its_timeout_for_a_place() -> None:
    """`deadline_exceeded`, waited for `calls_in_flight`; the call is never sent."""
    engine = HandPlayedEngine(1)
    collie = await async_connect_over(engine)
    errors: list[GoodfetchError] = []

    async with anyio.create_task_group() as calls:
        calls.start_soon(collie.call, "run.list", _NO_PARAMS)
        await engine.until(lambda: len(engine.calls) == 1)
        errors.append(
            await failure(
                collie.call("run.list", {}, timeout=timedelta(milliseconds=5))
            )
        )
        await engine.answer_once_sent(0, {"runs": []})
    await collie.close()

    [error] = errors
    assert error.code == "deadline_exceeded"
    assert error.operation == "run.list"
    assert error.limit is not None
    assert error.limit.model_dump(mode="json", exclude_none=True) == {
        "key": "timeout",
        "value": 5,
        "layer": "run",
        "origin": "argument",
    }
    assert error.details is not None
    assert error.details["waited_for"] == "calls_in_flight"
    assert engine.methods() == ["run.list"]


@pytest.mark.covers("error:deadline_exceeded", "setting:timeout.navigation")
@pytest.mark.anyio
async def test_fails_a_page_call_that_waits_past_its_timeout_naming_its_setting() -> (
    None
):
    """A page's `goto` names `timeout.navigation`, the setting its `timeout` sets."""
    engine = HandPlayedEngine(1)
    collie = await async_connect_over(engine)
    started = {
        "run_id": "01ke6p4ym0j8x3v5wq2rtbn7hc",
        "lifecycle": "running",
        "needs": "collie",
        "started_as": "new",
    }
    run = await answering(engine, 0, started, collie.start)
    opened = {"page_id": "page-1", "session_id": "session-1"}
    page = await answering(engine, 1, opened, run.new_page)
    errors: list[GoodfetchError] = []

    async with anyio.create_task_group() as calls:
        calls.start_soon(collie.call, "run.list", _NO_PARAMS)
        await engine.until(lambda: len(engine.calls) == 3)
        errors.append(
            await failure(
                page.goto("https://example.com/", timeout=timedelta(milliseconds=5))
            )
        )
        await engine.answer_once_sent(2, {"runs": []})

    [error] = errors
    assert error.code == "deadline_exceeded"
    assert error.limit is not None
    assert (error.limit.key, error.limit.value) == ("timeout.navigation", 5)
    assert error.details is not None
    assert error.details["waited_for"] == "calls_in_flight"
    assert engine.methods() == ["run.start", "run.new_page", "run.list"]


@pytest.mark.anyio
async def test_drops_a_queued_call_cancelled_with_its_task_never_sending_it() -> None:
    """The queued call leaves the queue: no request and no `$/cancel` reach it."""
    engine = HandPlayedEngine(1)
    collie = await async_connect_over(engine)

    async with anyio.create_task_group() as calls:
        calls.start_soon(collie.call, "run.list", _NO_PARAMS)
        await engine.until(lambda: len(engine.calls) == 1)
        with anyio.move_on_after(0.01):
            await collie.call("config.explain", {})
        await engine.answer_once_sent(0, {"runs": []})
    await collie.close()

    assert engine.methods() == ["run.list"]
    assert engine.notified == []


@pytest.mark.anyio
async def test_sends_a_queued_raw_call_whose_timeout_the_engine_refuses() -> None:
    """A `timeout` of 0 bounds no wait: the call is sent in its turn, as given."""
    engine = HandPlayedEngine(1)
    collie = await async_connect_over(engine)
    answers: list[JSONValue] = []

    async def refused_by_the_engine() -> None:
        answers.append(await collie.call("run.list", {}, timeout=timedelta(0)))

    async with anyio.create_task_group() as calls:
        calls.start_soon(collie.call, "run.list", _NO_PARAMS)
        await engine.until(lambda: len(engine.calls) == 1)
        calls.start_soon(refused_by_the_engine)
        await anyio.wait_all_tasks_blocked()
        engine.answer(0, {"runs": []})
        await engine.answer_once_sent(1, {"runs": []})
    await collie.close()

    sent = cast("Mapping[str, object]", engine.calls[1]["params"])
    assert sent["timeout"] == 0
    assert answers == [{"runs": []}]


@pytest.mark.anyio
async def test_ignores_a_reply_to_a_queued_call_and_sends_it_in_its_turn() -> None:
    """A reply naming a call not yet sent answers nothing; the call waits on."""
    engine = HandPlayedEngine(1)
    collie = await async_connect_over(engine)
    answers: list[JSONValue] = []

    async def queued() -> None:
        answers.append(await collie.call("config.explain", {}))

    async with anyio.create_task_group() as calls:
        calls.start_soon(collie.call, "run.list", _NO_PARAMS)
        await engine.until(lambda: len(engine.calls) == 1)
        calls.start_soon(queued)
        await anyio.wait_all_tasks_blocked()
        engine.reply_to(2, {"settings": ["too early"]})
        await anyio.wait_all_tasks_blocked()
        engine.answer(0, {"runs": []})
        await engine.answer_once_sent(1, {"settings": []})
    await collie.close()

    assert engine.methods() == ["run.list", "config.explain"]
    assert answers == [{"settings": []}]


@pytest.mark.anyio
async def test_keeps_a_sent_calls_place_until_the_engine_answers_its_cancel() -> None:
    """A call cancelled once sent holds its place until its answer frees it."""
    engine = HandPlayedEngine(1)
    collie = await async_connect_over(engine)
    first = anyio.CancelScope()
    answers: list[JSONValue] = []
    sent_before_the_answer: list[object] = []

    async def cancelled() -> None:
        with first:
            await collie.call("run.list", {})

    async def later() -> None:
        answers.append(await collie.call("config.explain", {}))

    async with anyio.create_task_group() as calls:
        calls.start_soon(cancelled)
        await engine.until(lambda: len(engine.calls) == 1)
        first.cancel()
        calls.start_soon(later)
        await engine.until(lambda: "$/cancel" in engine.notified)
        # Every task blocked: a wrongly freed place would have sent it.
        await anyio.wait_all_tasks_blocked()
        sent_before_the_answer = engine.methods()
        engine.answer(0, {"runs": []})
        await engine.answer_once_sent(1, {"settings": []})
    await collie.close()

    assert sent_before_the_answer == ["run.list"]
    assert engine.refused == []
    assert engine.methods() == ["run.list", "config.explain"]
    assert answers == [{"settings": []}]


@pytest.mark.covers(
    "error:connection_overloaded", "setting:connection.max_pending_calls"
)
@pytest.mark.anyio
async def test_refuses_the_call_past_its_bound_on_calls_waiting() -> None:
    """`connection_overloaded` at once, unsent, while the earlier call goes on."""
    engine = HandPlayedEngine(None)
    collie = await async_connect_over(
        engine, settings={"connection": {"max_pending_calls": 1}}
    )
    answers: list[JSONValue] = []
    errors: list[GoodfetchError] = []

    async def earlier() -> None:
        answers.append(await collie.call("run.list", {}))

    async with anyio.create_task_group() as calls:
        calls.start_soon(earlier)
        await engine.until(lambda: len(engine.calls) == 1)
        errors.append(await failure(collie.call("config.explain", {})))
        engine.answer(0, {"runs": []})
    await collie.close()

    [error] = errors
    assert (error.code, error.details) == (
        "connection_overloaded",
        {"bound": "pending_calls", "value": 1},
    )
    assert answers == [{"runs": []}]
    assert engine.methods() == ["run.list"]


@pytest.mark.covers(
    "error:connection_overloaded", "setting:connection.max_queued_write_bytes"
)
@pytest.mark.anyio
async def test_refuses_the_call_past_its_bound_on_bytes_waiting_to_be_written() -> None:
    """While a write waits, a call that would pass the bound fails at once."""
    engine = HandPlayedEngine(None, hold_writes=True)
    collie = await async_connect_over(
        engine, settings={"connection": {"max_queued_write_bytes": 1}}
    )
    errors: list[GoodfetchError] = []

    async with anyio.create_task_group() as calls:
        calls.start_soon(collie.call, "run.list", _NO_PARAMS)
        await engine.until(lambda: engine.held == 1)
        errors.append(await failure(collie.call("config.explain", {})))
        engine.release()
        await engine.answer_once_sent(0, {"runs": []})
    await collie.close()

    [error] = errors
    assert (error.code, error.details) == (
        "connection_overloaded",
        {"bound": "queued_write_bytes", "value": 1},
    )
    assert engine.methods() == ["run.list"]


@pytest.mark.covers("error:setting_invalid")
@pytest.mark.parametrize(
    ("bound", "value", "shown"),
    [
        ("max_pending_calls", 0, "0"),
        ("max_queued_write_bytes", -1, "-1"),
        ("max_pending_calls", 1.5, "1.5"),
        ("max_queued_write_bytes", True, "True"),
    ],
)
@three_ways
async def test_fails_connect_on_a_bound_that_is_no_whole_number_of_1_or_more(
    way: Way, bound: str, value: object, shown: str
) -> None:
    """It fails before any engine starts, naming the setting."""
    settings = cast("Mapping[str, object]", {"connection": {bound: value}})
    error = await failure(
        way.open(
            lambda: AsyncCollie.connect(env={}, settings=settings),  # type: ignore[arg-type]
            lambda: Collie.connect(env={}, settings=settings),  # type: ignore[arg-type]
        )
    )

    key = f"connection.{bound}"
    assert (error.code, error.group, error.details) == (
        "setting_invalid",
        "program",
        {
            "key": key,
            "layer": "run",
            "reason": f"{shown} is no whole number of 1 or more",
        },
    )
    assert error.fix is not None
    assert f"`{key}`" in error.fix
