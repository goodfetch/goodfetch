"""Seam B of the Scripted engine's players (ADR 0069).

The in-process player plays the shared corpus to a tiny Client written here,
over the in-memory transport an SDK takes, as `goodfetch-scripted-engine`
plays it over stdio. Each async test runs on asyncio and on trio.
"""

import json
from pathlib import Path
from typing import cast

import pytest
from anyio import EndOfStream
from anyio.abc import ByteStream
from goodfetch_test_support.play_script import (
    Closed,
    Exited,
    Finished,
    NothingToAnswer,
    PlayedMessage,
    PlayedRaw,
    UnexpectedMessage,
    UnusedStep,
    play_script,
)
from goodfetch_test_support.schemas import SchemaError

pytestmark = pytest.mark.anyio


@pytest.fixture(
    params=[pytest.param("asyncio", id="asyncio"), pytest.param("trio", id="trio")]
)
def anyio_backend(request: pytest.FixtureRequest) -> str:
    """Both backends, named, so trio is never left out (#401 moves it to the plugin)."""
    return cast("str", request.param)


COLLIE_HELLO = {
    "protocol_version": "0.1",
    "surface": "sdk_typescript",
    "needs": "collie",
    "product_version": "0.1.0",
}
"""The hello every self-test script expects from a Collie Client."""


def write_script(folder: Path, steps: list[object]) -> Path:
    """Writes a script of `steps` that no shared test reads, and returns its path."""
    path = folder / "script.json"
    path.write_text(json.dumps({"description": "A test's own script.", "steps": steps}))
    return path


class TinyClient:
    """A Client that writes and reads newline-delimited JSON-RPC, and nothing more."""

    def __init__(self, transport: ByteStream) -> None:
        """Takes the transport the play hands the SDK."""
        self._transport = transport
        self._buffered = b""
        self._next_id = 0

    async def write(self, text: str) -> None:
        """Writes one line of text."""
        await self._transport.send(text.encode())

    async def send(self, message: object) -> None:
        """Writes one message."""
        await self.write(f"{json.dumps(message)}\n")

    async def request(self, method: str, params: object) -> int:
        """Sends a request with the next ID, and returns the ID."""
        request_id = self._next_id
        self._next_id += 1
        await self.send(
            {"jsonrpc": "2.0", "id": request_id, "method": method, "params": params}
        )
        return request_id

    async def next_line(self) -> str | None:
        """The engine's next line, or `None` when it closed its end."""
        while b"\n" not in self._buffered:
            try:
                self._buffered += await self._transport.receive()
            except EndOfStream:
                rest, self._buffered = self._buffered, b""
                return rest.decode() if rest else None
        line, _, self._buffered = self._buffered.partition(b"\n")
        return line.decode()

    async def next(self) -> object:
        """The engine's next message, or `None` when it closed its end."""
        line = await self.next_line()
        return None if line is None else json.loads(line)

    async def close(self) -> None:
        """Closes the Client's end."""
        await self._transport.send_eof()


@pytest.mark.parametrize(
    ("script", "product", "product_version"),
    [
        pytest.param("self-test/hello-as-collie.json", "collie", "0.1.0", id="collie"),
        pytest.param(
            "self-test/hello-as-foxhound.json", "foxhound", "0.2.0", id="foxhound"
        ),
    ],
)
async def test_a_script_replying_as_a_product_is_read_as_that_products_engine(
    script: str, product: str, product_version: str
) -> None:
    """The hello's reply names the Product the script answers as."""
    played = play_script(script)
    client = TinyClient(played.transport)

    request_id = await client.request("hello", COLLIE_HELLO)
    reply = await client.next()
    await client.close()

    assert reply == {
        "jsonrpc": "2.0",
        "id": request_id,
        "result": {
            "protocol_version": "0.1",
            "product": product,
            "product_version": product_version,
            "engine_instance": "0123456789abcdef",
        },
    }
    assert played.done() == Finished()


async def test_a_self_test_script_plays_a_call_to_its_end() -> None:
    """A call after the hello gets the script's reply, and the play finishes."""
    played = play_script("self-test/call-answered.json")
    client = TinyClient(played.transport)

    await client.request("hello", COLLIE_HELLO)
    await client.next()
    request_id = await client.request("engine.info", {})
    reply = await client.next()
    await client.close()

    assert reply == {
        "jsonrpc": "2.0",
        "id": request_id,
        "result": {
            "product": "collie",
            "product_version": "0.1.0",
            "protocol_version": "0.1",
            "product_major": 0,
        },
    }
    assert played.done() == Finished()


async def test_the_play_records_every_message_it_read_and_wrote() -> None:
    """The record holds the Client's request and the engine's reply, in order."""
    played = play_script("self-test/hello-as-collie.json")
    client = TinyClient(played.transport)

    await client.request("hello", COLLIE_HELLO)
    await client.next()
    await client.close()

    assert played.messages == [
        PlayedMessage(
            from_="client",
            message={
                "jsonrpc": "2.0",
                "id": 0,
                "method": "hello",
                "params": COLLIE_HELLO,
            },
        ),
        PlayedMessage(
            from_="engine",
            message={
                "jsonrpc": "2.0",
                "id": 0,
                "result": {
                    "protocol_version": "0.1",
                    "product": "collie",
                    "product_version": "0.1.0",
                    "engine_instance": "0123456789abcdef",
                },
            },
        ),
    ]


async def test_a_not_in_product_refusal_ends_the_exchange() -> None:
    """The refusal is the engine's last message; then the script closes."""
    played = play_script("self-test/hello-refused-not-in-product.json")
    client = TinyClient(played.transport)

    await client.request("hello", {**COLLIE_HELLO, "needs": "foxhound"})
    reply = await client.next()
    after = await client.next()

    assert cast("dict[str, dict[str, dict[str, str]]]", reply)["error"]["data"][
        "code"
    ] == ("not_in_product")
    assert after is None
    assert played.done() == Closed()


async def test_a_script_calls_a_handler_and_sends_an_event_during_a_call() -> None:
    """The handler call, the event and the call's reply come in the script's order."""
    played = play_script("self-test/handler-call-and-event.json")
    client = TinyClient(played.transport)

    await client.request("hello", COLLIE_HELLO)
    await client.next()
    call = await client.request("sample.echo", {"text": "hello"})
    handler_call = await client.next()
    await client.send({"jsonrpc": "2.0", "id": 1, "result": {"replaced": False}})
    event = await client.next()
    reply = await client.next()
    await client.close()

    assert [handler_call, event, reply] == [
        {
            "jsonrpc": "2.0",
            "id": 1,
            "method": "sample_store.put",
            "params": {"key": "greeting", "value": "hello"},
        },
        {"jsonrpc": "2.0", "method": "sample.echoed", "params": {"text": "hello"}},
        {"jsonrpc": "2.0", "id": call, "result": {"text": "hello"}},
    ]
    assert played.done() == Finished()


async def test_a_script_sends_events_once_the_client_grants_credits_then_closes() -> (
    None
):
    """No event comes before the credit; the batch comes after it, then the end."""
    played = play_script("self-test/events-on-credit.json")
    client = TinyClient(played.transport)
    await client.request("hello", COLLIE_HELLO)
    await client.next()
    subscription = await client.request("events.subscribe", {})

    await client.send(
        {
            "jsonrpc": "2.0",
            "method": "$/credit",
            "params": {"id": subscription, "credits": 1},
        }
    )
    batch = await client.next()
    after = await client.next()

    assert [batch, after] == [
        {
            "jsonrpc": "2.0",
            "method": "$/events",
            "params": {
                "id": 1,
                "events": [
                    {"position": 1, "event": "sample.echoed", "data": {"text": "hello"}}
                ],
            },
        },
        None,
    ]
    assert played.done() == Closed()


async def test_a_reply_after_an_expected_cancel_answers_the_call_it_cancels() -> None:
    """The cancel is a notification, so the reply answers the subscription before it."""
    played = play_script("self-test/call-cancelled.json")
    client = TinyClient(played.transport)
    await client.request("hello", COLLIE_HELLO)
    await client.next()
    subscription = await client.request("events.subscribe", {})

    await client.send(
        {"jsonrpc": "2.0", "method": "$/cancel", "params": {"id": subscription}}
    )
    reply = cast("dict[str, object]", await client.next())
    await client.close()

    assert reply["id"] == subscription
    assert (
        cast("dict[str, dict[str, str]]", reply["error"])["data"]["code"]
        == "call_cancelled"
    )
    assert played.done() == Finished()


async def test_a_stalled_script_answers_nothing_until_the_client_gives_up() -> None:
    """The stalled call gets no answer; closing the Client's end finishes the play."""
    played = play_script("self-test/stall.json")
    client = TinyClient(played.transport)
    await client.request("hello", COLLIE_HELLO)
    await client.next()
    await client.request("sample.echo", {"text": "hello"})

    await client.close()

    assert await client.next() is None
    assert played.done() == Finished()


async def test_a_crash_mid_call_leaves_no_answer_and_ends_with_its_exit_code() -> None:
    """The engine's end closes with no answer to the call; the play exits with 101."""
    played = play_script("attached-engine/crash-mid-call.json")
    client = TinyClient(played.transport)
    await client.request("hello", COLLIE_HELLO)
    await client.next()

    await client.request("sample.echo", {"text": "hello"})

    assert await client.next() is None
    assert played.done() == Exited(101)


async def test_garbage_on_stdout_gives_the_client_a_line_that_is_not_json_rpc() -> None:
    """The raw step's text reaches the Client as written."""
    played = play_script("attached-engine/garbage-on-stdout.json")
    client = TinyClient(played.transport)
    await client.request("hello", COLLIE_HELLO)
    await client.next()

    line = await client.next_line()
    await client.close()

    assert line == "panicked at engine/src/lib.rs:1:1"
    assert played.done() == Finished()


async def test_a_handlers_wrong_answer_fails_the_play_at_once() -> None:
    """An answer that differs from `await_reply`'s fails the write that sends it."""
    played = play_script("self-test/handler-call-and-event.json")
    client = TinyClient(played.transport)
    await client.request("hello", COLLIE_HELLO)
    await client.next()
    await client.request("sample.echo", {"text": "hello"})
    await client.next()

    with pytest.raises(UnexpectedMessage):
        await client.send({"jsonrpc": "2.0", "id": 1, "result": {"replaced": True}})


async def test_a_message_no_step_expects_fails_the_play_at_once() -> None:
    """The write fails, and so do the Client's next read and `done`."""
    played = play_script("self-test/call-answered.json")
    client = TinyClient(played.transport)

    with pytest.raises(UnexpectedMessage):
        await client.request("hello", {**COLLIE_HELLO, "needs": "foxhound"})
    with pytest.raises(UnexpectedMessage):
        await client.next()
    with pytest.raises(UnexpectedMessage):
        played.done()


async def test_a_message_without_the_jsonrpc_member_fails_the_play_at_once() -> None:
    """The real engine refuses a message that is not JSON-RPC 2.0, as the play does."""
    played = play_script("self-test/call-answered.json")
    client = TinyClient(played.transport)

    with pytest.raises(UnexpectedMessage):
        await client.send({"id": 0, "method": "hello", "params": COLLIE_HELLO})


async def test_an_answer_with_both_a_result_and_an_error_fails_the_play_at_once() -> (
    None
):
    """A reply carries one of `result` and `error`, never both."""
    played = play_script("self-test/handler-call-and-event.json")
    client = TinyClient(played.transport)
    await client.request("hello", COLLIE_HELLO)
    await client.next()
    await client.request("sample.echo", {"text": "hello"})
    await client.next()

    with pytest.raises(UnexpectedMessage):
        await client.send(
            {
                "jsonrpc": "2.0",
                "id": 1,
                "result": {"replaced": False},
                "error": {"code": -32000, "message": "both"},
            }
        )


def test_an_await_reply_that_follows_no_request_fails_with_nothing_to_answer(
    tmp_path: Path,
) -> None:
    """The play fails as it starts, before the Client writes anything."""
    script = write_script(tmp_path, [{"await_reply": {"result": {}}}])

    played = play_script(script)

    with pytest.raises(NothingToAnswer):
        played.done()


async def test_a_json_line_that_is_not_an_object_is_recorded_as_a_message() -> None:
    """The line is JSON, so the record holds its value, and the play fails on it."""
    played = play_script("self-test/call-answered.json")
    client = TinyClient(played.transport)

    with pytest.raises(UnexpectedMessage):
        await client.write("[1]\n")

    assert played.messages == [PlayedMessage(from_="client", message=[1])]


async def test_a_line_that_is_not_json_fails_the_play_at_once() -> None:
    """The record holds the line as raw text."""
    played = play_script("self-test/call-answered.json")
    client = TinyClient(played.transport)

    with pytest.raises(UnexpectedMessage):
        await client.write("hello\n")

    assert played.messages == [PlayedRaw(from_="client", raw="hello")]


async def test_a_message_after_the_last_step_fails_the_play() -> None:
    """The script expects the Client to close its end once its steps are played."""
    played = play_script("self-test/hello-as-collie.json")
    client = TinyClient(played.transport)
    await client.request("hello", COLLIE_HELLO)
    await client.next()

    with pytest.raises(UnexpectedMessage):
        await client.request("engine.info", {})


async def test_a_step_left_unused_fails_the_play_when_the_client_closes_its_end() -> (
    None
):
    """Closing the Client's end fails, and so does `done`."""
    played = play_script("self-test/call-answered.json")
    client = TinyClient(played.transport)
    await client.request("hello", COLLIE_HELLO)
    await client.next()

    with pytest.raises(UnusedStep):
        await client.close()
    with pytest.raises(UnusedStep):
        played.done()


async def test_a_step_left_unused_fails_done() -> None:
    """`done` closes the Client's end as the Client would."""
    played = play_script("self-test/call-answered.json")
    client = TinyClient(played.transport)
    await client.request("hello", COLLIE_HELLO)
    await client.next()

    with pytest.raises(UnusedStep):
        played.done()


def test_a_script_that_fails_script_schema_json_is_refused(tmp_path: Path) -> None:
    """A step the schema does not define is refused before the play starts."""
    script = write_script(tmp_path, [{"pause": {"seconds": 1}}])

    with pytest.raises(SchemaError):
        play_script(script)


@pytest.mark.parametrize(
    "credits",
    [
        pytest.param(1, id="the-same-number"),
        pytest.param(1.0, id="the-same-number-with-a-fraction"),
    ],
)
async def test_params_match_when_their_canonical_json_is_the_same(
    credits: object,
) -> None:
    """`1` and `1.0` are the same JSON number, as RFC 8785 writes both `1`."""
    played = play_script("self-test/events-on-credit.json")
    client = TinyClient(played.transport)
    await client.request("hello", COLLIE_HELLO)
    await client.next()
    subscription = await client.request("events.subscribe", {})

    await client.send(
        {
            "jsonrpc": "2.0",
            "method": "$/credit",
            "params": {"id": subscription, "credits": credits},
        }
    )

    assert played.done() == Closed()


async def test_true_where_the_script_expects_1_fails_the_play() -> None:
    """`true` is no number in JSON, though Python counts `True == 1`."""
    played = play_script("self-test/events-on-credit.json")
    client = TinyClient(played.transport)
    await client.request("hello", COLLIE_HELLO)
    await client.next()
    subscription = await client.request("events.subscribe", {})

    with pytest.raises(UnexpectedMessage):
        await client.send(
            {
                "jsonrpc": "2.0",
                "method": "$/credit",
                "params": {"id": subscription, "credits": True},
            }
        )
